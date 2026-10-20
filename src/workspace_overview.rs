//! Workspaces overview rows shared by `tk workspaces` and `GET /api/workspaces`, and the
//! project backlog rows shown next to them (`tk workspaces` and `GET /api/backlogs`).

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::db::{BacklogTaskCounts, Database, WorkspaceTaskCounts};

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

/// Task counts of one project's backlog (JSON shape of `GET /api/backlogs`).
///
/// Kept apart from [`WorkspaceRow`] on purpose: a backlog is not a workspace, and a row with
/// `workspace_id: null` and a project would be mistaken for the unscoped bucket by
/// consumers of `tk workspaces --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BacklogRow {
    pub project_id: i64,
    pub project_name: String,
    pub open: i64,
    pub in_progress: i64,
    pub blocked: i64,
    pub done: i64,
}

/// One row per project that has at least one backlog task (any status), ordered by project
/// name then id.
pub fn backlog_rows(db: &Database) -> Result<Vec<BacklogRow>, String> {
    let counts: HashMap<i64, BacklogTaskCounts> = db
        .backlog_task_counts()?
        .into_iter()
        .map(|c| (c.project_id, c))
        .collect();
    Ok(db
        .list_projects()?
        .into_iter()
        .filter_map(|p| {
            let c = counts.get(&p.id).filter(|c| c.total() > 0)?;
            Some(BacklogRow {
                project_id: p.id,
                project_name: p.name,
                open: c.open,
                in_progress: c.in_progress,
                blocked: c.blocked,
                done: c.done,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Status, Task};

    fn task(id: &str, status: Status) -> Task {
        let now = chrono::Utc::now();
        Task {
            id: id.to_string(),
            title: id.to_string(),
            description: None,
            status,
            priority: 2,
            assignee: None,
            parent_id: None,
            tags: vec![],
            created_at: now,
            updated_at: now,
            close_reason: None,
            notes: None,
            workspace_id: None,
            project_id: None,
        }
    }

    #[test]
    fn test_backlog_rows_lists_only_projects_with_backlog_tasks() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        let a = db.upsert_project("/tmp/a-proj", "a-proj").unwrap();
        let b = db.upsert_project("/tmp/b-proj", "b-proj").unwrap();
        assert!(backlog_rows(&db).unwrap().is_empty());

        db.insert_backlog_task(&task("tk-1", Status::Open), b.id)
            .unwrap();
        db.insert_backlog_task(&task("tk-2", Status::Done), b.id)
            .unwrap();
        let rows = backlog_rows(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_id, b.id);
        assert_eq!(rows[0].project_name, "b-proj");
        assert_eq!((rows[0].open, rows[0].done), (1, 1));
        assert!(rows.iter().all(|r| r.project_id != a.id));
        // Workspace rows never contain backlog tasks.
        assert!(workspace_rows(&db, None).unwrap().is_empty());
    }
}
