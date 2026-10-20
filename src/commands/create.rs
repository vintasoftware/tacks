use std::path::Path;

use chrono::Utc;

use crate::db::Database;
use crate::models::{Status, Task};
use crate::scope::{self, Scope};

#[allow(clippy::too_many_arguments)]
pub fn run(
    db_path: &Path,
    title: &str,
    priority: u8,
    description: Option<&str>,
    tags: Option<&str>,
    parent: Option<&str>,
    to_backlog: bool,
    scope: &Scope,
    json: bool,
) -> Result<(), String> {
    if to_backlog && parent.is_some() {
        return Err(
            "--to-backlog cannot be used with --parent; subtasks follow their parent".to_string(),
        );
    }
    let db = Database::open(db_path)?;

    // Placement: (id, workspace_id, project_id). `project_id` is informational for the
    // output; the DB derives it from the workspace (or the parent) on insert.
    let (id, workspace_id, project_id) = if let Some(parent_id) = parent {
        // Verify parent exists; subtasks inherit the parent's workspace
        let parent_task = db
            .get_task(parent_id)?
            .ok_or_else(|| format!("parent task not found: {parent_id}"))?;
        scope::ensure_in_scope(&db, &parent_task, scope)?;
        (
            db.generate_child_id(parent_id)?,
            parent_task.workspace_id,
            parent_task.project_id,
        )
    } else if to_backlog {
        // Register the current project on first write; there is no project outside git
        let resolved = scope
            .resolve()?
            .ok_or_else(|| scope::NO_BACKLOG_ERROR.to_string())?;
        let project = db.upsert_project(&resolved.project_path, &resolved.project_name)?;
        (db.generate_id()?, None, Some(project.id))
    } else {
        // Register the current workspace on first write; unscoped outside a git repo
        let ws = match scope.resolve()? {
            Some(r) => Some(scope::register(&db, &r)?),
            None => None,
        };
        (
            db.generate_id()?,
            ws.as_ref().map(|w| w.id),
            ws.as_ref().map(|w| w.project_id),
        )
    };

    let now = Utc::now();
    let tag_list: Vec<String> = tags
        .map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let task = Task {
        id: id.clone(),
        title: title.to_string(),
        description: description.map(|s| s.to_string()),
        status: Status::Open,
        priority,
        assignee: None,
        parent_id: parent.map(|s| s.to_string()),
        tags: tag_list,
        created_at: now,
        updated_at: now,
        close_reason: None,
        notes: None,
        workspace_id,
        project_id,
    };

    match (to_backlog, project_id) {
        (true, Some(pid)) => db.insert_backlog_task(&task, pid)?,
        _ => db.insert_task(&task)?,
    }

    // Auto-tag parent as epic when a child is created, and sync epic status
    if let Some(parent_id) = parent {
        let mut parent_tags = db.get_task_tags(parent_id)?;
        if !parent_tags.contains(&"epic".to_string()) {
            parent_tags.push("epic".to_string());
            db.update_tags(parent_id, &parent_tags)?;
        }
        db.sync_epic_status(parent_id)?;
    }

    if json {
        let j = serde_json::to_string_pretty(&task).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
    } else {
        println!("Created task {id}: {title}");
    }

    Ok(())
}
