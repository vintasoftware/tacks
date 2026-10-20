use std::path::Path;

use crate::db::Database;
use crate::scope::Scope;
use crate::workspace_overview::{BacklogRow, backlog_rows, workspace_rows};

/// List projects and their workspaces with task counts.
///
/// Workspaces whose path no longer exists are flagged `missing`. Tasks with no
/// workspace are listed as a separate "unscoped" entry when there are any. Human output
/// also prints a `(backlog)` line before the workspaces of every project that has backlog
/// tasks (projects without workspaces included). `--json` is unchanged: the backlog is
/// not a workspace, so its counts are not rows of this array (see `GET /api/backlogs`).
pub fn run(db_path: &Path, scope: &Scope, json: bool) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let current = match scope.registered_workspace(&db) {
        Ok(w) => w.map(|w| w.id),
        Err(e) => {
            eprintln!("warning: {e}");
            None
        }
    };
    db.auto_archive_removed_workspaces(|p| !Path::new(p).exists())?;
    let rows = workspace_rows(&db, current)?;

    if json {
        let j = serde_json::to_string_pretty(&rows).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
        return Ok(());
    }

    let backlogs = backlog_rows(&db)?;
    if rows.is_empty() && backlogs.is_empty() {
        println!("No workspaces found.");
        return Ok(());
    }

    println!(
        "  {:<18} {:<22} {:>4} {:>4} {:>4} {:>4}  PATH",
        "PROJECT", "WORKSPACE", "OPEN", "WIP", "BLOCKED", "DONE"
    );
    let mut pending = backlogs.iter().peekable();
    for r in &rows {
        // Backlog lines go before the first workspace of their project, or before the
        // unscoped row (which has no project) for projects without workspaces.
        while let Some(b) = pending.peek() {
            let due = match r.project_name.as_deref() {
                Some(name) => {
                    (b.project_name.as_str(), b.project_id)
                        <= (name, r.project_id.unwrap_or_default())
                }
                None => true,
            };
            if !due {
                break;
            }
            print_backlog_line(b);
            pending.next();
        }
        let mut path = r.path.clone().unwrap_or_default();
        if r.missing {
            path.push_str(" (missing)");
        }
        if r.archived_at.is_some() {
            path.push_str(" (archived)");
        }
        println!(
            "{} {:<18} {:<22} {:>4} {:>4} {:>7} {:>4}  {}",
            if r.current { "*" } else { " " },
            r.project_name.as_deref().unwrap_or_default(),
            r.name,
            r.open,
            r.in_progress,
            r.blocked,
            r.done,
            path
        );
    }
    for b in pending {
        print_backlog_line(b);
    }
    Ok(())
}

/// Print the `(backlog)` line of a project in the workspaces table.
fn print_backlog_line(b: &BacklogRow) {
    println!(
        "  {:<18} {:<22} {:>4} {:>4} {:>7} {:>4}  (project backlog, no workspace)",
        b.project_name, "(backlog)", b.open, b.in_progress, b.blocked, b.done
    );
}
