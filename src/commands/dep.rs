use std::path::Path;

use crate::db::Database;
use crate::scope::{self, Scope};

/// Add a dependency; both tasks must be in scope.
pub fn add(db_path: &Path, child: &str, parent: &str, scope: &Scope) -> Result<(), String> {
    let db = Database::open(db_path)?;
    scope::ensure_id_in_scope(&db, child, scope)?;
    scope::ensure_id_in_scope(&db, parent, scope)?;
    db.add_dependency(child, parent)?;
    println!("Added dependency: {child} is blocked by {parent}");
    Ok(())
}

/// Remove a dependency; both tasks must be in scope.
pub fn remove(db_path: &Path, child: &str, parent: &str, scope: &Scope) -> Result<(), String> {
    let db = Database::open(db_path)?;
    scope::ensure_id_in_scope(&db, child, scope)?;
    scope::ensure_id_in_scope(&db, parent, scope)?;
    db.remove_dependency(child, parent)?;
    println!("Removed dependency: {child} no longer blocked by {parent}");
    Ok(())
}
