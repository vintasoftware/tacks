use std::path::Path;

use super::print_tasks;
use crate::db::Database;
use crate::scope::Scope;

pub fn run(db_path: &Path, limit: Option<u32>, scope: &Scope, json: bool) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let tasks = db.get_ready_tasks(limit, &scope.filter(&db)?)?;
    print_tasks(&tasks, json)
}
