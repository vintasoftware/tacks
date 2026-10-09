/// Database layer: open, migrate, CRUD, cycle detection.
pub mod db;
/// Data types: Task, Comment, Dependency, Status, CloseReason.
pub mod models;
/// Axum-based web server and router.
pub mod web;
/// Workspaces overview rows (shared by the CLI and the web API).
pub mod workspace_overview;
