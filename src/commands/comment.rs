use std::path::Path;

use crate::db::Database;
use crate::scope::{self, Scope};

/// Add a comment to a task in scope.
pub fn run(db_path: &Path, id: &str, body: &str, scope: &Scope, json: bool) -> Result<(), String> {
    let db = Database::open(db_path)?;
    scope::ensure_id_in_scope(&db, id, scope)?;
    let comment = db.add_comment(id, body)?;

    if json {
        let j = serde_json::to_string_pretty(&comment).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
    } else {
        println!("Added comment to {id}");
    }

    Ok(())
}
