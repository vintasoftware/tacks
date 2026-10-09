//! Workspaces overview rows shared by `tk workspaces` and `GET /api/workspaces`.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::db::{Database, WorkspaceTaskCounts};

/// One line of the workspaces overview (also the JSON shape of `tk workspaces --json`
/// and `GET /api/workspaces`).
///
/// The unscoped bucket is a row with `workspace_id: None` and `project_id: None`.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRow {
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub workspace_id: Option<i64>,
    pub name: String,
    pub path: Option<String>,
    /// True when the workspace path no longer exists on disk (checked at read time).
    pub missing: bool,
    /// True for the workspace the caller is in (always false for the web API).
    pub current: bool,
    pub open: i64,
    pub in_progress: i64,
    pub blocked: i64,
    pub done: i64,
    /// When the workspace was archived (hidden from the web UI), else null.
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Build the workspaces overview rows: every workspace with its task counts, plus an
/// "(unscoped)" row when tasks without a workspace exist. `current` marks one workspace id.
pub fn workspace_rows(db: &Database, current: Option<i64>) -> Result<Vec<WorkspaceRow>, String> {
    let projects = db.list_projects()?;
    let project_names: HashMap<i64, &str> =
        projects.iter().map(|p| (p.id, p.name.as_str())).collect();
    let counts: HashMap<Option<i64>, WorkspaceTaskCounts> = db
        .workspace_task_counts()?
        .into_iter()
        .map(|c| (c.workspace_id, c))
        .collect();

    let mut rows = Vec::new();
    for w in db.list_workspaces()? {
        let c = counts.get(&Some(w.id));
        rows.push(WorkspaceRow {
            project_id: Some(w.project_id),
            project_name: Some(
                project_names
                    .get(&w.project_id)
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ),
            workspace_id: Some(w.id),
            name: w.name,
            missing: !Path::new(&w.path).exists(),
            current: current == Some(w.id),
            path: Some(w.path),
            open: c.map_or(0, |c| c.open),
            in_progress: c.map_or(0, |c| c.in_progress),
            blocked: c.map_or(0, |c| c.blocked),
            done: c.map_or(0, |c| c.done),
            archived_at: w.archived_at,
        });
    }
    if let Some(c) = counts.get(&None).filter(|c| c.total() > 0) {
        rows.push(WorkspaceRow {
            project_id: None,
            project_name: None,
            workspace_id: None,
            name: "(unscoped)".to_string(),
            path: None,
            missing: false,
            current: false,
            open: c.open,
            in_progress: c.in_progress,
            blocked: c.blocked,
            done: c.done,
            archived_at: None,
        });
    }
    Ok(rows)
}
