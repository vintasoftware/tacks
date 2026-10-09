use std::path::Path;

use super::print_tasks;
use crate::db::Database;
use crate::scope::Scope;

#[allow(clippy::too_many_arguments)]
pub fn run(
    db_path: &Path,
    all: bool,
    status: Option<&str>,
    priority: Option<u8>,
    tag: Option<&str>,
    parent: Option<&str>,
    scope: &Scope,
    json: bool,
) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let tasks = db.list_tasks(
        all,
        status,
        priority,
        tag,
        parent,
        None,
        None,
        &scope.filter(&db)?,
    )?;
    print_tasks(&tasks, json)
}
