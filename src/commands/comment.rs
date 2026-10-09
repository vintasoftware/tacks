use std::path::Path;

use crate::db::Database;
use crate::scope::{self, Scope};

/// Add a comment to a task in scope.
///
/// The comment is attributed to `author` (default `agent`). The author `user` marks the
/// comment as user feedback, which agents see in `tk prime` / `tk show` until they reply.
pub fn run(
    db_path: &Path,
    id: &str,
    body: &str,
    author: Option<&str>,
    scope: &Scope,
    json: bool,
) -> Result<(), String> {
    let db = Database::open(db_path)?;
    scope::ensure_id_in_scope(&db, id, scope)?;
    let comment = db.add_comment_by(id, body, Some(author.unwrap_or("agent")))?;

    if json {
        let j = serde_json::to_string_pretty(&comment).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
    } else {
        println!("Added comment to {id}");
    }

    Ok(())
}
