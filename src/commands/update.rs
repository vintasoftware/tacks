use std::path::Path;

use crate::db::Database;
use crate::scope;

#[allow(clippy::too_many_arguments)]
pub fn run(
    db_path: &Path,
    id: &str,
    title: Option<&str>,
    priority: Option<u8>,
    status: Option<&str>,
    description: Option<&str>,
    claim: bool,
    assignee: Option<&str>,
    add_tags: Option<&str>,
    remove_tags: Option<&str>,
    notes: Option<&str>,
    parent: Option<&str>,
    move_to: Option<&str>,
    to_backlog: bool,
    scope: &scope::Scope,
    json: bool,
) -> Result<(), String> {
    if to_backlog && move_to.is_some() {
        return Err("--to-backlog cannot be used with --move-to".to_string());
    }
    if to_backlog && claim {
        return Err(
            "--to-backlog cannot be used with --claim; claiming moves the task into the current workspace"
                .to_string(),
        );
    }
    let db = Database::open(db_path)?;

    // Write guard: the task (and a new parent) must be in scope, checked before any write.
    scope::ensure_id_in_scope(&db, id, scope)?;
    if let Some(p) = parent
        && !p.eq_ignore_ascii_case("none")
    {
        scope::ensure_id_in_scope(&db, p, scope)?;
    }

    // Handle claim: set status to in_progress and assignee
    let effective_status = if claim { Some("in_progress") } else { status };

    let effective_assignee = if claim && assignee.is_none() {
        Some("agent")
    } else {
        assignee
    };

    // Convert --parent flag to the two-level Option:
    // --parent none    → Some(None)     (clear parent, promote to top-level)
    // --parent <id>    → Some(Some(id)) (reparent under <id>)
    // (not provided)   → None           (no change)
    let new_parent: Option<Option<&str>> = parent.map(|p| {
        if p.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(p)
        }
    });

    // Resolve the --move-to target (read-only) before any write; registration of a new
    // workspace happens inside the transaction below.
    let move_target: Option<Option<scope::ResolvedWorkspace>> = match move_to {
        None => None,
        Some(t) if t.eq_ignore_ascii_case("none") => Some(None),
        Some(t) => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("cannot determine current directory: {e}"))?;
            let resolved = scope::resolve(Some(std::path::Path::new(t)), &cwd)?
                .ok_or_else(|| format!("cannot resolve workspace: {t}"))?;
            Some(Some(resolved))
        }
    };

    // Resolve the --to-backlog target (read-only). The task's own project wins; an unscoped
    // task goes to the current project, which is registered inside the transaction.
    let task_before = db.get_task(id)?;
    let backlog_fallback: Option<scope::ResolvedWorkspace> = if to_backlog {
        let fallback = scope.resolve()?;
        if fallback.is_none() && task_before.as_ref().is_some_and(|t| t.project_id.is_none()) {
            return Err(scope::NO_BACKLOG_ERROR.to_string());
        }
        fallback
    } else {
        None
    };

    // Claiming a backlog task pulls it (and its subtasks) into the current workspace,
    // registered inside the transaction. The guard above already checked the scope.
    let pull_target: Option<scope::ResolvedWorkspace> = if claim
        && task_before
            .as_ref()
            .is_some_and(|t| t.workspace_id.is_none() && t.project_id.is_some())
    {
        Some(scope.resolve()?.ok_or_else(|| {
            "cannot claim a backlog task without a current workspace to move it into; use --workspace <path>"
                .to_string()
        })?)
    } else {
        None
    };

    // Everything below is one transaction: a failing update leaves the task unchanged.
    // Order: reparent (moves the subtree to the new parent's workspace), then the claim
    // pull-in, then the explicit --move-to / --to-backlog, which therefore win.
    db.with_savepoint(|| {
        db.update_task_with_parent(
            id,
            title,
            priority,
            effective_status,
            description,
            effective_assignee,
            None,
            notes,
            new_parent,
        )?;

        // Sync parent epic status if this task's status changed
        if effective_status.is_some()
            && let Some(task) = db.get_task(id)?
            && let Some(ref pid) = task.parent_id
        {
            db.sync_epic_status(pid)?;
        }

        // Handle tag changes
        if add_tags.is_some() || remove_tags.is_some() {
            let mut current_tags = db.get_task_tags(id)?;

            if let Some(add) = add_tags {
                for tag in add
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    if !current_tags.contains(&tag) {
                        current_tags.push(tag);
                    }
                }
            }

            if let Some(remove) = remove_tags {
                let remove_set: Vec<String> =
                    remove.split(',').map(|s| s.trim().to_string()).collect();
                current_tags.retain(|t| !remove_set.contains(t));
            }

            db.update_tags(id, &current_tags)?;
        }

        // Pull: a claimed backlog task moves into the current workspace, unless the
        // reparent above already placed it elsewhere.
        if let Some(resolved) = &pull_target
            && db
                .get_task(id)?
                .is_some_and(|t| t.workspace_id.is_none() && t.project_id.is_some())
        {
            let workspace_id = scope::register(&db, resolved)?.id;
            db.set_task_workspace(id, Some(workspace_id))?;
        }

        if to_backlog && let Some(task) = db.get_task(id)? {
            let project_id = match (task.project_id, &backlog_fallback) {
                (Some(pid), _) => pid,
                (None, Some(r)) => db.upsert_project(&r.project_path, &r.project_name)?.id,
                (None, None) => return Err(scope::NO_BACKLOG_ERROR.to_string()),
            };
            db.move_task_to_backlog(id, project_id)?;
        }

        if let Some(target) = &move_target {
            let workspace_id = match target {
                None => None,
                Some(resolved) => Some(scope::register(&db, resolved)?.id),
            };
            db.set_task_workspace(id, workspace_id)?;
        }
        Ok(())
    })?;

    if json {
        let task = db
            .get_task(id)?
            .ok_or_else(|| format!("task not found: {id}"))?;
        let j = serde_json::to_string_pretty(&task).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
    } else {
        println!("Updated task {id}");
    }

    Ok(())
}
