use std::path::Path;

use crate::db::Database;
use crate::models::validate_close_reason;
use crate::scope::{self, Scope};

/// Close a task, optionally recording a comment and close reason.
pub fn run(
    db_path: &Path,
    id: &str,
    comment: Option<&str>,
    reason: Option<&str>,
    force: bool,
    scope: &Scope,
    json: bool,
) -> Result<(), String> {
    let db = Database::open(db_path)?;
    scope::ensure_id_in_scope(&db, id, scope)?;

    // Validate reason before touching the DB.
    if let Some(r) = reason {
        validate_close_reason(r)?;
    }

    // Feedback guard: refuse while the user has comments the agent has not answered.
    if !force {
        let pending = db.task_pending_user_comments(id)?;
        if !pending.is_empty() {
            let items: Vec<String> = pending
                .iter()
                .map(|c| {
                    format!(
                        "\"{}\"",
                        super::truncate_chars(&c.body.replace('\n', " "), 80)
                    )
                })
                .collect();
            return Err(format!(
                "task {} has {} unanswered user comment(s): {}. address them and reply with `tk comment {} \"...\"` first, or use --force only if the user agreed to close",
                id,
                pending.len(),
                items.join("; "),
                id
            ));
        }
    }

    // Close guard: refuse to close a parent task (epic) that still has open
    // subtask children. Use --force to override.
    //
    // Note: dependency-graph blocking (dep add) is a separate relationship;
    // closing a prerequisite (blocker) while dependents are still open is the
    // expected workflow and is not guarded here.
    let children = db.get_children(id)?;
    let open_children: Vec<_> = children
        .iter()
        .filter(|t| t.status != crate::models::Status::Done)
        .collect();

    if !open_children.is_empty() && !force {
        let names: Vec<String> = open_children
            .iter()
            .map(|t| format!("{} ({})", t.id, t.title))
            .collect();
        return Err(format!(
            "task {} has {} open dependent(s): {}. use --force to close anyway",
            id,
            open_children.len(),
            names.join(", ")
        ));
    }

    // Status change and comment succeed or fail together.
    db.with_savepoint(|| {
        db.close_task(id, reason)?;
        if let Some(body) = comment {
            db.add_comment_by(id, body, Some("agent"))?;
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
        println!("Closed task {id}");
    }

    Ok(())
}
