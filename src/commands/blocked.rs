use std::path::Path;

use crate::db::Database;
use crate::scope::Scope;

/// List tasks that are blocked by open dependencies.
pub fn run(db_path: &Path, scope: &Scope, json: bool) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let tasks = db.get_blocked_tasks(&scope.filter(&db)?)?;
    super::print_tasks(&tasks, json)
}
