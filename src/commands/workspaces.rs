use std::path::Path;

use crate::db::Database;
use crate::scope::Scope;
use crate::workspace_overview::workspace_rows;

/// List projects and their workspaces with task counts.
///
/// Workspaces whose path no longer exists are flagged `missing`. Tasks with no
/// workspace are listed as a separate "unscoped" entry when there are any.
pub fn run(db_path: &Path, scope: &Scope, json: bool) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let current = match scope.registered_workspace(&db) {
        Ok(w) => w.map(|w| w.id),
        Err(e) => {
            eprintln!("warning: {e}");
            None
        }
    };
    let rows = workspace_rows(&db, current)?;

    if json {
        let j = serde_json::to_string_pretty(&rows).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
        return Ok(());
    }

    if rows.is_empty() {
        println!("No workspaces found.");
        return Ok(());
    }

    println!(
        "  {:<18} {:<22} {:>4} {:>4} {:>4} {:>4}  PATH",
        "PROJECT", "WORKSPACE", "OPEN", "WIP", "BLOCKED", "DONE"
    );
    for r in &rows {
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
    Ok(())
}
