use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use crate::models::{
    Comment, Dependency, PendingComment, Project, Status, Task, Workspace, validate_close_reason,
};

/// Latest schema version known to this build.
const LATEST_SCHEMA_VERSION: i32 = 6;

/// Which tasks a list-type query should cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFilter {
    /// Every task in the database.
    All,
    /// Tasks whose workspace belongs to the given project id.
    Project(i64),
    /// Tasks in exactly the given workspace id.
    Workspace(i64),
    /// Tasks with no workspace (`workspace_id IS NULL`).
    Unscoped,
    /// Every task except those of archived workspaces (unscoped tasks stay visible).
    /// Used by the web UI; the CLI uses `All`.
    AllVisible,
    /// Like `Project`, but skips archived workspaces of the project.
    ProjectVisible(i64),
    /// Matches no task at all (e.g. a scope that is registered nowhere).
    Nothing,
}

impl ScopeFilter {
    /// Build a SQL condition on `column` (e.g. `t.workspace_id`) using positional
    /// parameter `?{idx}` when a value needs binding. Returns `None` for `All`.
    /// The bound value, if any, is returned alongside the condition.
    fn condition(&self, column: &str, idx: usize) -> Option<(String, Option<i64>)> {
        match self {
            ScopeFilter::All => None,
            ScopeFilter::Unscoped => Some((format!("{column} IS NULL"), None)),
            ScopeFilter::Nothing => Some(("1 = 0".to_string(), None)),
            ScopeFilter::AllVisible => Some((
                format!(
                    "({column} IS NULL OR {column} NOT IN \
                     (SELECT id FROM workspaces WHERE archived_at IS NOT NULL))"
                ),
                None,
            )),
            ScopeFilter::ProjectVisible(id) => Some((
                format!(
                    "{column} IN (SELECT id FROM workspaces \
                     WHERE project_id = ?{idx} AND archived_at IS NULL)"
                ),
                Some(*id),
            )),
            ScopeFilter::Workspace(id) => Some((format!("{column} = ?{idx}"), Some(*id))),
            ScopeFilter::Project(id) => Some((
                format!("{column} IN (SELECT id FROM workspaces WHERE project_id = ?{idx})"),
                Some(*id),
            )),
        }
    }
}

/// Per-workspace task counts, used for the workspaces overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTaskCounts {
    /// Workspace id, or `None` for the unscoped bucket.
    pub workspace_id: Option<i64>,
    pub open: i64,
    pub in_progress: i64,
    pub blocked: i64,
    pub done: i64,
}

impl WorkspaceTaskCounts {
    /// Total number of tasks across all statuses.
    pub fn total(&self) -> i64 {
        self.open + self.in_progress + self.blocked + self.done
    }
}

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the database at the given path and bring its schema up to date.
    ///
    /// Creates the parent directory if missing and runs [`Database::migrate`] on every
    /// open. Migration is idempotent and takes a fast path (one small query) when the
    /// schema is already current.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create database directory: {e}"))?;
        }

        let conn = Connection::open(path).map_err(|e| format!("failed to open database: {e}"))?;

        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| format!("failed to set busy timeout: {e}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("failed to set pragmas: {e}"))?;

        let db = Database { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open an existing database read-only: no creation, no pragmas that write, no migration.
    ///
    /// Intended for cheap probes (the PostToolUse hook). The schema may be older than
    /// current, so callers must tolerate missing columns.
    pub fn open_read_only(path: &Path) -> Result<Self, String> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("failed to open database: {e}"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| format!("failed to set busy timeout: {e}"))?;
        Ok(Database { conn })
    }

    /// Create the schema tables if they don't exist, then run any pending version-gated migrations.
    pub fn migrate(&self) -> Result<(), String> {
        // Fast path: schema already current (config table may not exist yet, which errors).
        if matches!(get_schema_version(&self.conn), Ok(v) if v >= LATEST_SCHEMA_VERSION) {
            return Ok(());
        }

        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS config (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tasks (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                description TEXT,
                status      TEXT NOT NULL DEFAULT 'open',
                priority    INTEGER NOT NULL DEFAULT 2,
                assignee    TEXT,
                parent_id   TEXT REFERENCES tasks(id),
                tags        TEXT NOT NULL DEFAULT '',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS dependencies (
                child_id  TEXT NOT NULL REFERENCES tasks(id),
                parent_id TEXT NOT NULL REFERENCES tasks(id),
                PRIMARY KEY (child_id, parent_id),
                CHECK (child_id != parent_id)
            );

            CREATE TABLE IF NOT EXISTS comments (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL REFERENCES tasks(id),
                body       TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority);
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
            CREATE INDEX IF NOT EXISTS idx_deps_child ON dependencies(child_id);
            CREATE INDEX IF NOT EXISTS idx_deps_parent ON dependencies(parent_id);
            CREATE INDEX IF NOT EXISTS idx_comments_task ON comments(task_id);
            ",
            )
            .map_err(|e| format!("migration failed: {e}"))?;

        // Ensure schema_version exists in config (fresh databases get version 0).
        self.conn
            .execute(
                "INSERT OR IGNORE INTO config (key, value) VALUES ('schema_version', '0')",
                [],
            )
            .map_err(|e| format!("failed to seed schema_version: {e}"))?;

        run_migrations(&self.conn)
    }

    // -- Config --

    pub fn set_config(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(|e| format!("failed to set config: {e}"))?;
        Ok(())
    }

    pub fn get_config(&self, key: &str) -> Result<Option<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM config WHERE key = ?1")
            .map_err(|e| format!("query error: {e}"))?;
        let mut rows = stmt
            .query_map(params![key], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query error: {e}"))?;
        match rows.next() {
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => Err(format!("query error: {e}")),
            None => Ok(None),
        }
    }

    // -- Tasks --

    pub fn insert_task(&self, task: &Task) -> Result<(), String> {
        let tags_str = task.tags.join(",");
        self.conn
            .execute(
                "INSERT INTO tasks (id, title, description, status, priority, assignee, parent_id, tags, created_at, updated_at, close_reason, notes, workspace_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    task.id,
                    task.title,
                    task.description,
                    task.status.as_str(),
                    task.priority,
                    task.assignee,
                    task.parent_id,
                    tags_str,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                    task.close_reason,
                    task.notes,
                    task.workspace_id,
                ],
            )
            .map_err(|e| format!("failed to insert task: {e}"))?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, title, description, status, priority, assignee, parent_id, tags, created_at, updated_at, close_reason, notes, workspace_id
                 FROM tasks WHERE id = ?1",
            )
            .map_err(|e| format!("query error: {e}"))?;

        let mut rows = stmt
            .query_map(params![id], |row| Ok(row_to_task(row)))
            .map_err(|e| format!("query error: {e}"))?;

        match rows.next() {
            Some(Ok(task)) => Ok(Some(task)),
            Some(Err(e)) => Err(format!("query error: {e}")),
            None => Ok(None),
        }
    }

    /// List tasks with optional filters.
    ///
    /// - `include_done`: when `true`, done tasks are included even without a status_filter
    /// - `status_filter`: exact status match (overrides include_done)
    /// - `priority_filter`: exact priority match
    /// - `tag_filter`: task must contain this tag
    /// - `parent_filter`: task must have this parent_id
    /// - `search`: case-insensitive substring match on title
    /// - `completed_after`: RFC3339 timestamp; when `Some`, only tasks with `updated_at > ts` are returned
    /// - `scope`: restrict to a workspace, project, the unscoped bucket, or everything
    #[allow(clippy::too_many_arguments)]
    pub fn list_tasks(
        &self,
        include_done: bool,
        status_filter: Option<&str>,
        priority_filter: Option<u8>,
        tag_filter: Option<&str>,
        parent_filter: Option<&str>,
        search: Option<&str>,
        completed_after: Option<&str>,
        scope: &ScopeFilter,
    ) -> Result<Vec<Task>, String> {
        let mut sql = String::from(
            "SELECT id, title, description, status, priority, assignee, parent_id, tags, created_at, updated_at, close_reason, notes, workspace_id FROM tasks WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        if let Some(status) = status_filter {
            sql.push_str(&format!(" AND status = ?{param_idx}"));
            param_values.push(Box::new(status.to_string()));
            param_idx += 1;
        } else if !include_done {
            sql.push_str(&format!(" AND status != ?{param_idx}"));
            param_values.push(Box::new("done".to_string()));
            param_idx += 1;
        }

        if let Some(p) = priority_filter {
            sql.push_str(&format!(" AND priority = ?{param_idx}"));
            param_values.push(Box::new(p));
            param_idx += 1;
        }

        if let Some(tag) = tag_filter {
            sql.push_str(&format!(
                " AND (',' || tags || ',') LIKE '%,' || ?{param_idx} || ',%'"
            ));
            param_values.push(Box::new(tag.to_string()));
            param_idx += 1;
        }

        if let Some(parent) = parent_filter {
            sql.push_str(&format!(" AND parent_id = ?{param_idx}"));
            param_values.push(Box::new(parent.to_string()));
            param_idx += 1;
        }

        if let Some(s) = search {
            sql.push_str(&format!(
                " AND title LIKE '%' || ?{param_idx} || '%' COLLATE NOCASE"
            ));
            param_values.push(Box::new(s.to_string()));
            param_idx += 1;
        }

        if let Some(ts) = completed_after {
            sql.push_str(&format!(" AND updated_at > ?{param_idx}"));
            param_values.push(Box::new(ts.to_string()));
            param_idx += 1;
        }

        if let Some((cond, value)) = scope.condition("workspace_id", param_idx) {
            sql.push_str(&format!(" AND {cond}"));
            if let Some(v) = value {
                param_values.push(Box::new(v));
            }
        }

        sql.push_str(" ORDER BY priority ASC, created_at ASC");

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("query error: {e}"))?;

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| Ok(row_to_task(row)))
            .map_err(|e| format!("query error: {e}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(tasks)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_task(
        &self,
        id: &str,
        title: Option<&str>,
        priority: Option<u8>,
        status: Option<&str>,
        description: Option<&str>,
        assignee: Option<&str>,
        close_reason: Option<&str>,
        notes: Option<&str>,
    ) -> Result<(), String> {
        self.update_task_with_parent(
            id,
            title,
            priority,
            status,
            description,
            assignee,
            close_reason,
            notes,
            None,
        )
    }

    /// Update task fields, optionally reparenting to a new parent.
    ///
    /// `new_parent` uses a two-level Option:
    /// - `None` — do not change parent_id
    /// - `Some(Some("tk-xxxx"))` — set parent_id to the given task
    /// - `Some(None)` — clear parent_id (promote to top-level; keeps the task's workspace)
    ///
    /// Reparenting under a task moves the task and its whole subtree into the new
    /// parent's workspace. The call is atomic: on any error nothing is changed.
    #[allow(clippy::too_many_arguments)]
    pub fn update_task_with_parent(
        &self,
        id: &str,
        title: Option<&str>,
        priority: Option<u8>,
        status: Option<&str>,
        description: Option<&str>,
        assignee: Option<&str>,
        close_reason: Option<&str>,
        notes: Option<&str>,
        new_parent: Option<Option<&str>>,
    ) -> Result<(), String> {
        self.with_savepoint(|| {
            self.update_task_with_parent_inner(
                id,
                title,
                priority,
                status,
                description,
                assignee,
                close_reason,
                notes,
                new_parent,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn update_task_with_parent_inner(
        &self,
        id: &str,
        title: Option<&str>,
        priority: Option<u8>,
        status: Option<&str>,
        description: Option<&str>,
        assignee: Option<&str>,
        close_reason: Option<&str>,
        notes: Option<&str>,
        new_parent: Option<Option<&str>>,
    ) -> Result<(), String> {
        let mut new_parent_workspace: Option<Option<i64>> = None;
        let mut sets = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;

        if let Some(t) = title {
            sets.push(format!("title = ?{idx}"));
            param_values.push(Box::new(t.to_string()));
            idx += 1;
        }
        if let Some(p) = priority {
            sets.push(format!("priority = ?{idx}"));
            param_values.push(Box::new(p));
            idx += 1;
        }
        if let Some(s) = status {
            // Validate status
            Status::from_str(s)?;
            sets.push(format!("status = ?{idx}"));
            param_values.push(Box::new(s.to_string()));
            idx += 1;
        }
        if let Some(d) = description {
            sets.push(format!("description = ?{idx}"));
            param_values.push(Box::new(d.to_string()));
            idx += 1;
        }
        if let Some(a) = assignee {
            sets.push(format!("assignee = ?{idx}"));
            param_values.push(Box::new(a.to_string()));
            idx += 1;
        }
        if let Some(r) = close_reason {
            validate_close_reason(r)?;
            sets.push(format!("close_reason = ?{idx}"));
            param_values.push(Box::new(r.to_string()));
            idx += 1;
        }
        if let Some(n) = notes {
            sets.push(format!("notes = ?{idx}"));
            param_values.push(Box::new(n.to_string()));
            idx += 1;
        }

        // Handle reparenting
        if let Some(maybe_parent) = new_parent {
            match maybe_parent {
                Some(new_pid) => {
                    // Validate: no self-parenting
                    if new_pid == id {
                        return Err("cannot reparent a task under itself".to_string());
                    }
                    // Validate: new parent must exist
                    self.get_task(new_pid)?
                        .ok_or_else(|| format!("parent task not found: {new_pid}"))?;
                    // Validate: new parent must not itself be a subtask (max depth 1)
                    let parent_task = self.get_task(new_pid)?.unwrap();
                    new_parent_workspace = Some(parent_task.workspace_id);
                    if parent_task.parent_id.is_some() {
                        return Err(format!(
                            "cannot reparent under {new_pid}: it is already a subtask (max depth is 1)"
                        ));
                    }
                    // Validate: no circular parenting (child is not an ancestor of new parent)
                    // Check if new_pid is a child of id (which would create a cycle)
                    let my_children = self.get_children(id)?;
                    for child in &my_children {
                        if child.id == new_pid {
                            return Err(
                                "circular parenting: the new parent is a child of this task"
                                    .to_string(),
                            );
                        }
                    }

                    sets.push(format!("parent_id = ?{idx}"));
                    param_values.push(Box::new(new_pid.to_string()));
                    idx += 1;

                    // Auto-tag new parent as epic
                    let mut parent_tags = self.get_task_tags(new_pid)?;
                    if !parent_tags.contains(&"epic".to_string()) {
                        parent_tags.push("epic".to_string());
                        self.update_tags(new_pid, &parent_tags)?;
                    }
                }
                None => {
                    // Clear parent_id (promote to top-level)
                    sets.push("parent_id = NULL".to_string());
                    // No param needed for NULL
                }
            }
        }

        if sets.is_empty() {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();
        sets.push(format!("updated_at = ?{idx}"));
        param_values.push(Box::new(now));
        idx += 1;

        let sql = format!("UPDATE tasks SET {} WHERE id = ?{idx}", sets.join(", "));
        param_values.push(Box::new(id.to_string()));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows_changed = self
            .conn
            .execute(&sql, params_ref.as_slice())
            .map_err(|e| format!("update failed: {e}"))?;

        if rows_changed == 0 {
            return Err(format!("task not found: {id}"));
        }
        // Subtasks live in their parent's workspace: move the task and its subtree.
        if let Some(ws) = new_parent_workspace {
            self.set_task_workspace(id, ws)?;
        }
        Ok(())
    }

    /// Close a task: set status to done and record the close_reason.
    ///
    /// Automatically syncs the parent epic's status if this task is a subtask.
    pub fn close_task(&self, id: &str, reason: Option<&str>) -> Result<(), String> {
        self.update_task(id, None, None, Some("done"), None, None, reason, None)?;
        self.sync_parent_epic(id)?;
        Ok(())
    }

    /// If the given task has a parent, recalculate and update the parent epic's status.
    fn sync_parent_epic(&self, task_id: &str) -> Result<(), String> {
        if let Some(task) = self.get_task(task_id)?
            && let Some(pid) = &task.parent_id
        {
            self.sync_epic_status(pid)?;
        }
        Ok(())
    }

    pub fn update_tags(&self, id: &str, tags: &[String]) -> Result<(), String> {
        let tags_str = tags.join(",");
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE tasks SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                params![tags_str, now, id],
            )
            .map_err(|e| format!("tag update failed: {e}"))?;
        Ok(())
    }

    pub fn get_task_tags(&self, id: &str) -> Result<Vec<String>, String> {
        let task = self
            .get_task(id)?
            .ok_or_else(|| format!("task not found: {id}"))?;
        Ok(task.tags)
    }

    // -- Dependencies --

    pub fn add_dependency(&self, child_id: &str, parent_id: &str) -> Result<(), String> {
        // Verify both tasks exist
        self.get_task(child_id)?
            .ok_or_else(|| format!("task not found: {child_id}"))?;
        self.get_task(parent_id)?
            .ok_or_else(|| format!("task not found: {parent_id}"))?;

        // Detect duplicate before inserting
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM dependencies WHERE child_id = ?1 AND parent_id = ?2",
                params![child_id, parent_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(|e| format!("query error: {e}"))?;

        if exists {
            return Err(format!(
                "dependency already exists: {child_id} is already blocked by {parent_id}"
            ));
        }

        // Guard against cycles: check whether parent_id transitively depends on child_id
        if would_create_cycle(&self.conn, child_id, parent_id)? {
            return Err(
                "circular dependency detected: adding this dependency would create a cycle"
                    .to_string(),
            );
        }

        self.conn
            .execute(
                "INSERT INTO dependencies (child_id, parent_id) VALUES (?1, ?2)",
                params![child_id, parent_id],
            )
            .map_err(|e| format!("failed to add dependency: {e}"))?;
        Ok(())
    }

    pub fn remove_dependency(&self, child_id: &str, parent_id: &str) -> Result<(), String> {
        // Verify both tasks exist
        self.get_task(child_id)?
            .ok_or_else(|| format!("task not found: {child_id}"))?;
        self.get_task(parent_id)?
            .ok_or_else(|| format!("task not found: {parent_id}"))?;

        let rows = self
            .conn
            .execute(
                "DELETE FROM dependencies WHERE child_id = ?1 AND parent_id = ?2",
                params![child_id, parent_id],
            )
            .map_err(|e| format!("failed to remove dependency: {e}"))?;

        if rows == 0 {
            return Err(format!(
                "no dependency found: {child_id} is not blocked by {parent_id}"
            ));
        }
        Ok(())
    }

    pub fn get_blockers(&self, task_id: &str) -> Result<Vec<Dependency>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT child_id, parent_id FROM dependencies WHERE child_id = ?1")
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(Dependency {
                    child_id: row.get(0)?,
                    parent_id: row.get(1)?,
                })
            })
            .map_err(|e| format!("query error: {e}"))?;

        let mut deps = Vec::new();
        for row in rows {
            deps.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(deps)
    }

    /// Get all tasks that are blocked by the given task (reverse of `get_blockers`).
    ///
    /// Returns every task whose work cannot proceed until `task_id` is resolved.
    /// This is the "dependents" direction: `task_id` is the blocker, and the
    /// returned tasks are the ones waiting on it.
    pub fn get_dependents(&self, task_id: &str) -> Result<Vec<Task>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT t.id, t.title, t.description, t.status, t.priority, t.assignee,
                        t.parent_id, t.tags, t.created_at, t.updated_at, t.close_reason, t.notes, t.workspace_id
                 FROM tasks t
                 JOIN dependencies d ON t.id = d.child_id
                 WHERE d.parent_id = ?1
                 ORDER BY t.priority ASC, t.created_at ASC",
            )
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(params![task_id], |row| Ok(row_to_task(row)))
            .map_err(|e| format!("query error: {e}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(tasks)
    }

    /// Get tasks that are ready: open and have no open/in_progress blockers.
    /// If `limit` is `Some(n)`, return at most `n` tasks.
    pub fn get_ready_tasks(
        &self,
        limit: Option<u32>,
        scope: &ScopeFilter,
    ) -> Result<Vec<Task>, String> {
        let (scope_sql, scope_val) = match scope.condition("t.workspace_id", 1) {
            Some((c, v)) => (format!(" AND {c}"), v),
            None => (String::new(), None),
        };
        let mut sql = String::from(
            "
            SELECT t.id, t.title, t.description, t.status, t.priority, t.assignee, t.parent_id, t.tags, t.created_at, t.updated_at, t.close_reason, t.notes, t.workspace_id
            FROM tasks t
            WHERE t.status = 'open'
              {SCOPE}
              AND NOT EXISTS (
                SELECT 1 FROM dependencies d
                JOIN tasks blocker ON d.parent_id = blocker.id
                WHERE d.child_id = t.id
                  AND blocker.status IN ('open', 'in_progress', 'blocked')
              )
            ORDER BY t.priority ASC, t.created_at ASC
        ",
        )
        .replace("{SCOPE}", &scope_sql);

        if let Some(n) = limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("query error: {e}"))?;

        let bound: Vec<i64> = scope_val.into_iter().collect();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                Ok(row_to_task(row))
            })
            .map_err(|e| format!("query error: {e}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(tasks)
    }

    /// Get tasks that have at least one open/in_progress blocker.
    pub fn get_blocked_tasks(&self, scope: &ScopeFilter) -> Result<Vec<Task>, String> {
        let (scope_sql, scope_val) = match scope.condition("t.workspace_id", 1) {
            Some((c, v)) => (format!(" AND {c}"), v),
            None => (String::new(), None),
        };
        let sql = format!(
            "SELECT DISTINCT t.id, t.title, t.description, t.status, t.priority, t.assignee,
                    t.parent_id, t.tags, t.created_at, t.updated_at, t.close_reason, t.notes, t.workspace_id
             FROM tasks t
             JOIN dependencies d ON t.id = d.child_id
             JOIN tasks blocker ON d.parent_id = blocker.id
             WHERE t.status != 'done'
               AND blocker.status IN ('open', 'in_progress', 'blocked'){scope_sql}
             ORDER BY t.priority ASC, t.created_at ASC"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("query error: {e}"))?;

        let bound: Vec<i64> = scope_val.into_iter().collect();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                Ok(row_to_task(row))
            })
            .map_err(|e| format!("query error: {e}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(tasks)
    }

    // -- Comments --

    /// Add a comment with no recorded author (legacy behaviour; treated as agent-authored).
    pub fn add_comment(&self, task_id: &str, body: &str) -> Result<Comment, String> {
        self.add_comment_by(task_id, body, None)
    }

    /// Add a comment attributed to `author` (`"user"` for the web UI, `"agent"` for the CLI).
    pub fn add_comment_by(
        &self,
        task_id: &str,
        body: &str,
        author: Option<&str>,
    ) -> Result<Comment, String> {
        // Verify task exists
        self.get_task(task_id)?
            .ok_or_else(|| format!("task not found: {task_id}"))?;

        let now = Utc::now();
        self.conn
            .execute(
                "INSERT INTO comments (task_id, body, created_at, author) VALUES (?1, ?2, ?3, ?4)",
                params![task_id, body, now.to_rfc3339(), author],
            )
            .map_err(|e| format!("failed to add comment: {e}"))?;

        let id = self.conn.last_insert_rowid();
        Ok(Comment {
            id,
            task_id: task_id.to_string(),
            body: body.to_string(),
            created_at: now,
            author: author.map(str::to_string),
        })
    }

    /// All comments on a task, oldest first.
    pub fn get_comments(&self, task_id: &str) -> Result<Vec<Comment>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, task_id, body, created_at, author FROM comments WHERE task_id = ?1 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(params![task_id], |row| Ok(row_to_comment(row, 0)))
            .map_err(|e| format!("query error: {e}"))?;

        let mut comments = Vec::new();
        for row in rows {
            comments.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(comments)
    }

    /// Shared query for pending user comments. A comment is pending when it is authored by
    /// `user`, its task is not done, and no non-user comment (NULL author counts as
    /// non-user) on the same task has a higher id (ids are monotonic, so this is
    /// "created after" without timestamp ties).
    fn pending_query(
        &self,
        scope: &ScopeFilter,
        task_id: Option<&str>,
        extra: &str,
    ) -> Result<Vec<PendingComment>, String> {
        let mut sql = format!(
            "SELECT c.id, c.task_id, c.body, c.created_at, c.author, t.title, t.status
             FROM comments c JOIN tasks t ON t.id = c.task_id
             WHERE c.author = 'user' AND t.status != 'done'
               AND NOT EXISTS (SELECT 1 FROM comments r WHERE r.task_id = c.task_id
                               AND r.id > c.id AND (r.author IS NULL OR r.author != 'user')){extra}"
        );
        let mut bound: Vec<rusqlite::types::Value> = Vec::new();
        if let Some((cond, value)) = scope.condition("t.workspace_id", 1) {
            sql.push_str(&format!(" AND {cond}"));
            bound.extend(value.into_iter().map(rusqlite::types::Value::Integer));
        }
        if let Some(id) = task_id {
            bound.push(rusqlite::types::Value::Text(id.to_string()));
            sql.push_str(&format!(" AND c.task_id = ?{}", bound.len()));
        }
        sql.push_str(" ORDER BY c.created_at ASC, c.id ASC");

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("query error: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                Ok(PendingComment {
                    task_id: row.get(1)?,
                    task_title: row.get(5)?,
                    task_status: row.get(6)?,
                    comment: row_to_comment(row, 0),
                })
            })
            .map_err(|e| format!("query error: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(out)
    }

    /// User comments the agent has not replied to yet, within `scope`, oldest first.
    pub fn pending_user_comments(
        &self,
        scope: &ScopeFilter,
    ) -> Result<Vec<PendingComment>, String> {
        self.pending_query(scope, None, "")
    }

    /// Number of pending user comments per task id (tasks with none are absent).
    pub fn pending_user_comment_count_by_task(
        &self,
        scope: &ScopeFilter,
    ) -> Result<HashMap<String, i64>, String> {
        let mut counts = HashMap::new();
        for p in self.pending_user_comments(scope)? {
            *counts.entry(p.task_id).or_insert(0) += 1;
        }
        Ok(counts)
    }

    /// Pending user comments for one task (empty when the task is done or missing).
    pub fn task_pending_user_comments(&self, task_id: &str) -> Result<Vec<Comment>, String> {
        Ok(self
            .pending_query(&ScopeFilter::All, Some(task_id), "")?
            .into_iter()
            .map(|p| p.comment)
            .collect())
    }

    /// Pending user comments that have not yet been pushed into an agent session.
    pub fn undelivered_pending_user_comments(
        &self,
        scope: &ScopeFilter,
    ) -> Result<Vec<PendingComment>, String> {
        self.pending_query(scope, None, " AND c.delivered_at IS NULL")
    }

    /// Cheap probe: is there any pending user comment, anywhere, that was not delivered yet?
    ///
    /// One query, no scope resolution. Works on a read-only connection; an older schema
    /// without the `author`/`delivered_at` columns yields `false` instead of an error.
    pub fn any_undelivered_pending_user_comment(&self) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM comments c JOIN tasks t ON t.id = c.task_id
                 WHERE c.author = 'user' AND c.delivered_at IS NULL AND t.status != 'done'
                   AND NOT EXISTS (SELECT 1 FROM comments r WHERE r.task_id = c.task_id
                                   AND r.id > c.id AND (r.author IS NULL OR r.author != 'user'))
                 LIMIT 1",
                [],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Atomically claim undelivered pending user comments of `scope` for delivery.
    ///
    /// Inside one write transaction (`BEGIN IMMEDIATE` when not already in one) the
    /// undelivered pending comments are listed oldest first; `take` is called for each in
    /// order and claiming stops at the first `false` (so only what will be shown is claimed).
    /// Each claimed row is marked delivered with `UPDATE ... WHERE delivered_at IS NULL`
    /// and kept only when exactly one row changed, so concurrent callers never both get the
    /// same comment. Returns the comments this call claimed and how many undelivered
    /// pending comments remain unclaimed.
    pub fn claim_undelivered_pending_user_comments(
        &self,
        scope: &ScopeFilter,
        mut take: impl FnMut(&PendingComment) -> bool,
    ) -> Result<(Vec<PendingComment>, usize), String> {
        let own_tx = self.conn.is_autocommit();
        if own_tx {
            self.conn
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| format!("failed to begin transaction: {e}"))?;
        }
        let mut work = || -> Result<(Vec<PendingComment>, usize), String> {
            let all = self.undelivered_pending_user_comments(scope)?;
            let now = Utc::now().to_rfc3339();
            let total = all.len();
            let mut claimed = Vec::new();
            for p in all {
                if !take(&p) {
                    break;
                }
                let changed = self
                    .conn
                    .execute(
                        "UPDATE comments SET delivered_at = ?1 WHERE id = ?2 AND delivered_at IS NULL",
                        params![now, p.comment.id],
                    )
                    .map_err(|e| format!("failed to claim comment: {e}"))?;
                if changed == 1 {
                    claimed.push(p);
                }
            }
            let remaining = total - claimed.len();
            Ok((claimed, remaining))
        };
        let result = work();
        if own_tx {
            match &result {
                Ok(_) => self
                    .conn
                    .execute_batch("COMMIT")
                    .map_err(|e| format!("failed to commit: {e}"))?,
                Err(_) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                }
            }
        }
        result
    }

    // -- Stats --

    /// Count tasks grouped by status.
    pub fn task_count_by_status(&self, scope: &ScopeFilter) -> Result<Vec<(String, i64)>, String> {
        let (where_sql, bound) = scope_where(scope);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT status, COUNT(*) FROM tasks{where_sql} GROUP BY status ORDER BY status"
            ))
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("query error: {e}"))?;

        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(counts)
    }

    /// Count tasks grouped by priority.
    pub fn task_count_by_priority(&self, scope: &ScopeFilter) -> Result<Vec<(u8, i64)>, String> {
        let (where_sql, bound) = scope_where(scope);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT priority, COUNT(*) FROM tasks{where_sql} GROUP BY priority ORDER BY priority"
            ))
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                Ok((row.get::<_, u8>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("query error: {e}"))?;

        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(counts)
    }

    /// Count tasks grouped by tag (tasks with multiple tags are counted once per tag).
    pub fn task_count_by_tag(&self, scope: &ScopeFilter) -> Result<Vec<(String, i64)>, String> {
        // Pull all non-empty tags columns and split them in Rust
        let (scope_sql, bound) = match scope.condition("workspace_id", 1) {
            Some((c, v)) => (format!(" AND {c}"), v.into_iter().collect::<Vec<_>>()),
            None => (String::new(), Vec::new()),
        };
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT tags FROM tasks WHERE tags != ''{scope_sql}"
            ))
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("query error: {e}"))?;

        let mut map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for row in rows {
            let tags_str = row.map_err(|e| format!("row error: {e}"))?;
            for tag in tags_str.split(',') {
                let tag = tag.trim();
                if !tag.is_empty() {
                    *map.entry(tag.to_string()).or_insert(0) += 1;
                }
            }
        }

        let mut counts: Vec<(String, i64)> = map.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(counts)
    }

    // -- Projects and workspaces --

    /// Insert a project with the given path and display name, or return the existing
    /// row when a project with that path is already registered (name is not updated).
    pub fn upsert_project(&self, path: &str, name: &str) -> Result<Project, String> {
        // Look up first: a conflicting INSERT on an AUTOINCREMENT table still consumes an id.
        if let Some(existing) = self.get_project_by_path(path)? {
            return Ok(existing);
        }
        self.conn
            .execute(
                "INSERT INTO projects (path, name, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(path) DO NOTHING",
                params![path, name, Utc::now().to_rfc3339()],
            )
            .map_err(|e| format!("failed to upsert project: {e}"))?;
        self.get_project_by_path(path)?
            .ok_or_else(|| "project missing after upsert".to_string())
    }

    /// Insert a workspace under `project_id`, or return the existing row when a workspace
    /// with that path is already registered (project and name are not updated). An archived
    /// workspace is restored (its id stays the same).
    pub fn upsert_workspace(
        &self,
        project_id: i64,
        path: &str,
        name: &str,
    ) -> Result<Workspace, String> {
        // Look up first: a conflicting INSERT on an AUTOINCREMENT table still consumes an id.
        if let Some(existing) = self.find_workspace_by_path(path)? {
            if existing.archived_at.is_some() {
                // Registering an archived path again brings it (and its tasks) back.
                return self.restore_workspace(existing.id);
            }
            return Ok(existing);
        }
        self.conn
            .execute(
                "INSERT INTO workspaces (project_id, path, name, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(path) DO NOTHING",
                params![project_id, path, name, Utc::now().to_rfc3339()],
            )
            .map_err(|e| format!("failed to upsert workspace: {e}"))?;
        self.find_workspace_by_path(path)?
            .ok_or_else(|| "workspace missing after upsert".to_string())
    }

    fn get_project_by_path(&self, path: &str) -> Result<Option<Project>, String> {
        self.query_optional(
            "SELECT id, path, name, created_at FROM projects WHERE path = ?1",
            params![path],
            row_to_project,
        )
    }

    /// Look up a workspace by its canonical path.
    pub fn find_workspace_by_path(&self, path: &str) -> Result<Option<Workspace>, String> {
        self.query_optional(
            "SELECT id, project_id, path, name, created_at, archived_at, restored_at FROM workspaces WHERE path = ?1",
            params![path],
            row_to_workspace,
        )
    }

    /// Look up a workspace by id.
    pub fn get_workspace(&self, id: i64) -> Result<Option<Workspace>, String> {
        self.query_optional(
            "SELECT id, project_id, path, name, created_at, archived_at, restored_at FROM workspaces WHERE id = ?1",
            params![id],
            row_to_workspace,
        )
    }

    /// Look up a project by id.
    pub fn get_project(&self, id: i64) -> Result<Option<Project>, String> {
        self.query_optional(
            "SELECT id, path, name, created_at FROM projects WHERE id = ?1",
            params![id],
            row_to_project,
        )
    }

    fn query_optional<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        mapper: fn(&rusqlite::Row) -> rusqlite::Result<T>,
    ) -> Result<Option<T>, String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("query error: {e}"))?;
        let mut rows = stmt
            .query_map(params, mapper)
            .map_err(|e| format!("query error: {e}"))?;
        match rows.next() {
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => Err(format!("query error: {e}")),
            None => Ok(None),
        }
    }

    /// List all projects ordered by name, then id.
    pub fn list_projects(&self) -> Result<Vec<Project>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, name, created_at FROM projects ORDER BY name, id")
            .map_err(|e| format!("query error: {e}"))?;
        let rows = stmt
            .query_map([], row_to_project)
            .map_err(|e| format!("query error: {e}"))?;
        rows.map(|r| r.map_err(|e| format!("row error: {e}")))
            .collect()
    }

    /// List all workspaces ordered by project (name, id), then workspace name, then id.
    pub fn list_workspaces(&self) -> Result<Vec<Workspace>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT w.id, w.project_id, w.path, w.name, w.created_at, w.archived_at, w.restored_at
                 FROM workspaces w JOIN projects p ON p.id = w.project_id
                 ORDER BY p.name, p.id, w.name, w.id",
            )
            .map_err(|e| format!("query error: {e}"))?;
        let rows = stmt
            .query_map([], row_to_workspace)
            .map_err(|e| format!("query error: {e}"))?;
        rows.map(|r| r.map_err(|e| format!("row error: {e}")))
            .collect()
    }

    /// Task counts per status for every workspace that has tasks, plus one entry with
    /// `workspace_id: None` for the unscoped bucket when it has tasks. Workspaces with
    /// no tasks are not listed; callers should default them to zero.
    pub fn workspace_task_counts(&self) -> Result<Vec<WorkspaceTaskCounts>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT workspace_id,
                        SUM(status = 'open'), SUM(status = 'in_progress'),
                        SUM(status = 'blocked'), SUM(status = 'done')
                 FROM tasks GROUP BY workspace_id ORDER BY workspace_id",
            )
            .map_err(|e| format!("query error: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(WorkspaceTaskCounts {
                    workspace_id: row.get(0)?,
                    open: row.get(1)?,
                    in_progress: row.get(2)?,
                    blocked: row.get(3)?,
                    done: row.get(4)?,
                })
            })
            .map_err(|e| format!("query error: {e}"))?;
        rows.map(|r| r.map_err(|e| format!("row error: {e}")))
            .collect()
    }

    /// Move a task and all of its descendants (via `parent_id`) to `workspace_id`
    /// (`None` = unscoped bucket) atomically. Tasks whose workspace already equals the
    /// target are untouched. `updated_at` is bumped on moved tasks that are not `done`
    /// (the board uses `updated_at` of done tasks as their completion time).
    /// Returns the number of tasks moved.
    pub fn set_task_workspace(
        &self,
        task_id: &str,
        workspace_id: Option<i64>,
    ) -> Result<usize, String> {
        if self.get_task(task_id)?.is_none() {
            return Err(format!("task not found: {task_id}"));
        }
        if let Some(ws) = workspace_id
            && self.get_workspace(ws)?.is_none()
        {
            return Err(format!("workspace not found: {ws}"));
        }

        self.with_savepoint(|| {
            self.conn
                .execute(
                    "WITH RECURSIVE subtree(id) AS (
                         SELECT ?1
                         UNION
                         SELECT t.id FROM tasks t JOIN subtree s ON t.parent_id = s.id
                     )
                     UPDATE tasks SET workspace_id = ?2,
                         updated_at = CASE WHEN status = 'done' THEN updated_at ELSE ?3 END
                     WHERE id IN (SELECT id FROM subtree) AND workspace_id IS NOT ?2",
                    params![task_id, workspace_id, Utc::now().to_rfc3339()],
                )
                .map_err(|e| format!("failed to move task: {e}"))
        })
    }

    /// Archive a workspace (soft, reversible): sets `archived_at` when it is NULL, so
    /// repeating the call keeps the original timestamp. Tasks are never modified.
    pub fn archive_workspace(&self, id: i64) -> Result<Workspace, String> {
        self.get_workspace(id)?
            .ok_or_else(|| format!("workspace not found: {id}"))?;
        self.conn
            .execute(
                "UPDATE workspaces SET archived_at = ?1 WHERE id = ?2 AND archived_at IS NULL",
                params![Utc::now().to_rfc3339(), id],
            )
            .map_err(|e| format!("failed to archive workspace: {e}"))?;
        self.get_workspace(id)?
            .ok_or_else(|| format!("workspace not found: {id}"))
    }

    /// Restore an archived workspace (clears `archived_at` and sets `restored_at` so
    /// auto-archive skips it). Only changes anything when it was archived; idempotent.
    pub fn restore_workspace(&self, id: i64) -> Result<Workspace, String> {
        self.get_workspace(id)?
            .ok_or_else(|| format!("workspace not found: {id}"))?;
        self.conn
            .execute(
                "UPDATE workspaces SET archived_at = NULL, restored_at = ?1\n                 WHERE id = ?2 AND archived_at IS NOT NULL",
                params![Utc::now().to_rfc3339(), id],
            )
            .map_err(|e| format!("failed to restore workspace: {e}"))?;
        self.get_workspace(id)?
            .ok_or_else(|| format!("workspace not found: {id}"))
    }

    /// Archive every active, never-restored workspace whose tasks are all `done` (or that
    /// has no tasks) and whose path `is_missing` reports as gone. Sets `archived_at` like
    /// [`Database::archive_workspace`]; tasks are never modified. Returns the archived ids.
    pub fn auto_archive_removed_workspaces(
        &self,
        is_missing: impl Fn(&str) -> bool,
    ) -> Result<Vec<i64>, String> {
        let candidates: Vec<(i64, String)> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT w.id, w.path FROM workspaces w
                     WHERE w.archived_at IS NULL AND w.restored_at IS NULL
                       AND NOT EXISTS (
                           SELECT 1 FROM tasks t
                           WHERE t.workspace_id = w.id AND t.status != 'done')
                     ORDER BY w.id",
                )
                .map_err(|e| format!("failed to query workspaces: {e}"))?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| format!("failed to query workspaces: {e}"))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("failed to read workspaces: {e}"))?
        };
        let now = Utc::now().to_rfc3339();
        let mut archived = Vec::new();
        for (id, path) in candidates {
            if !is_missing(&path) {
                continue;
            }
            let n = self
                .conn
                .execute(
                    "UPDATE workspaces SET archived_at = ?1
                     WHERE id = ?2 AND archived_at IS NULL AND restored_at IS NULL",
                    params![now, id],
                )
                .map_err(|e| format!("failed to archive workspace: {e}"))?;
            if n > 0 {
                archived.push(id);
            }
        }
        Ok(archived)
    }

    /// Number of tasks in the workspace that are not done (what
    /// [`Database::close_workspace_tasks`] would close).
    pub fn count_open_workspace_tasks(&self, workspace_id: i64) -> Result<i64, String> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE workspace_id = ?1 AND status != 'done'",
                params![workspace_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("query error: {e}"))
    }

    /// Close every not-done task of a workspace in one transaction, with `reason` as the
    /// close reason. When `comment` is given it is added (with `author`) to each closed
    /// task. Parent epic statuses are synced like `close_task` does. Already-done tasks are
    /// untouched. Returns the number of tasks closed.
    pub fn close_workspace_tasks(
        &self,
        workspace_id: i64,
        reason: &str,
        comment: Option<&str>,
        author: Option<&str>,
    ) -> Result<usize, String> {
        validate_close_reason(reason)?;
        self.get_workspace(workspace_id)?
            .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;

        self.with_savepoint(|| {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id FROM tasks WHERE workspace_id = ?1 AND status != 'done' ORDER BY id",
                )
                .map_err(|e| format!("query error: {e}"))?;
            let ids = stmt
                .query_map(params![workspace_id], |row| row.get::<_, String>(0))
                .map_err(|e| format!("query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("row error: {e}"))?;
            drop(stmt);

            for id in &ids {
                self.close_task(id, Some(reason))?;
                if let Some(body) = comment {
                    self.add_comment_by(id, body, author)?;
                }
            }
            Ok(ids.len())
        })
    }

    /// Run `f` inside a SAVEPOINT: commit on `Ok`, roll everything back on `Err`.
    ///
    /// Savepoints nest, so this is safe to call from code that is already inside
    /// another `with_savepoint` (or inside no transaction at all).
    pub fn with_savepoint<T>(&self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.conn
            .execute_batch("SAVEPOINT tk_sp")
            .map_err(|e| format!("failed to begin transaction: {e}"))?;
        match f() {
            Ok(v) => {
                self.conn
                    .execute_batch("RELEASE tk_sp")
                    .map_err(|e| format!("failed to commit: {e}"))?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK TO tk_sp; RELEASE tk_sp");
                Err(e)
            }
        }
    }

    /// Generate a short hash-based ID with the configured prefix.
    pub fn generate_id(&self) -> Result<String, String> {
        let prefix = self
            .get_config("prefix")?
            .unwrap_or_else(|| "tk".to_string());
        let uuid = uuid::Uuid::new_v4();
        let hash = &format!("{:x}", uuid.as_u128())[..4];
        Ok(format!("{prefix}-{hash}"))
    }

    /// Generate a child ID under a parent.
    pub fn generate_child_id(&self, parent_id: &str) -> Result<String, String> {
        // Count existing children to determine next index
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM tasks WHERE parent_id = ?1")
            .map_err(|e| format!("query error: {e}"))?;
        let count: i64 = stmt
            .query_row(params![parent_id], |row| row.get(0))
            .map_err(|e| format!("query error: {e}"))?;
        Ok(format!("{parent_id}.{}", count + 1))
    }

    /// Return the current SQLite `PRAGMA data_version` value.
    ///
    /// This integer increments whenever the database is modified by any connection,
    /// making it suitable as a lightweight change-detection signal for polling clients.
    pub fn data_version(&self) -> Result<i64, String> {
        self.conn
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .map_err(|e| format!("failed to read data_version: {e}"))
    }

    pub fn get_children(&self, parent_id: &str) -> Result<Vec<Task>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, title, description, status, priority, assignee, parent_id, tags, created_at, updated_at, close_reason, notes, workspace_id
                 FROM tasks WHERE parent_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| format!("query error: {e}"))?;

        let rows = stmt
            .query_map(params![parent_id], |row| Ok(row_to_task(row)))
            .map_err(|e| format!("query error: {e}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(tasks)
    }

    /// Traverse the dependency graph from a starting task and return all reachable tasks with depth.
    ///
    /// `direction` controls which edges are followed:
    /// - `"up"` — follows blocker edges (what does this task depend on, transitively)
    /// - `"down"` — follows dependent edges (what tasks depend on this task, transitively)
    /// - `"both"` — follows both directions
    ///
    /// The root task itself (depth 0) is **not** included in the result — the caller already holds it.
    /// All other reachable tasks are returned at their BFS depth (minimum hops from root), ordered
    /// breadth-first. Diamond dependencies are handled correctly: a task reachable via multiple paths
    /// appears only once, at the shallowest depth at which it was first discovered.
    ///
    /// A visited set guards against infinite loops even though cycles are rejected at write time.
    pub fn get_dependency_chain(
        &self,
        task_id: &str,
        direction: &str,
    ) -> Result<Vec<(Task, usize)>, String> {
        use std::collections::{HashSet, VecDeque};

        if direction != "up" && direction != "down" && direction != "both" {
            return Err(format!(
                "invalid direction '{direction}': must be 'up', 'down', or 'both'"
            ));
        }

        // Helper: fetch direct blocker IDs for a given task (up direction)
        let fetch_up = |current: &str| -> Result<Vec<String>, String> {
            let mut stmt = self
                .conn
                .prepare("SELECT parent_id FROM dependencies WHERE child_id = ?1")
                .map_err(|e| format!("query error: {e}"))?;
            let ids: Vec<String> = stmt
                .query_map(params![current], |row| row.get::<_, String>(0))
                .map_err(|e| format!("query error: {e}"))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(ids)
        };

        // Helper: fetch direct dependent IDs for a given task (down direction)
        let fetch_down = |current: &str| -> Result<Vec<String>, String> {
            let mut stmt = self
                .conn
                .prepare("SELECT child_id FROM dependencies WHERE parent_id = ?1")
                .map_err(|e| format!("query error: {e}"))?;
            let ids: Vec<String> = stmt
                .query_map(params![current], |row| row.get::<_, String>(0))
                .map_err(|e| format!("query error: {e}"))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(ids)
        };

        // BFS in one direction; returns (id, depth) pairs ordered by discovery
        let bfs = |start: &str, go_up: bool| -> Result<Vec<(String, usize)>, String> {
            let mut visited: HashSet<String> = HashSet::new();
            let mut queue: VecDeque<(String, usize)> = VecDeque::new();
            let mut result: Vec<(String, usize)> = Vec::new();

            visited.insert(start.to_string());
            queue.push_back((start.to_string(), 0));

            while let Some((current, depth)) = queue.pop_front() {
                let neighbors = if go_up {
                    fetch_up(&current)?
                } else {
                    fetch_down(&current)?
                };

                for neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor.clone());
                        result.push((neighbor.clone(), depth + 1));
                        queue.push_back((neighbor, depth + 1));
                    }
                }
            }

            Ok(result)
        };

        // Collect (id, depth) pairs according to direction, deduplicating for "both"
        let id_depth_pairs: Vec<(String, usize)> = match direction {
            "up" => bfs(task_id, true)?,
            "down" => bfs(task_id, false)?,
            "both" => {
                let mut seen: HashSet<String> = HashSet::new();
                let mut combined: Vec<(String, usize)> = Vec::new();

                for (id, depth) in bfs(task_id, true)?.into_iter().chain(bfs(task_id, false)?) {
                    if !seen.contains(&id) {
                        seen.insert(id.clone());
                        combined.push((id, depth));
                    }
                }
                combined
            }
            _ => unreachable!(),
        };

        // Resolve IDs to Task structs
        let mut result = Vec::with_capacity(id_depth_pairs.len());
        for (id, depth) in id_depth_pairs {
            if let Some(task) = self.get_task(&id)? {
                result.push((task, depth));
            }
        }

        Ok(result)
    }

    /// Recalculate and update an epic's status based on its children's statuses.
    ///
    /// - All children open/blocked → epic status = open
    /// - Any children done or in_progress (but not all done) → epic status = in_progress
    /// - All children done → epic status = done
    /// - No children → no change
    ///
    /// Call this after any operation that changes a child task's status.
    pub fn sync_epic_status(&self, epic_id: &str) -> Result<(), String> {
        let children = self.get_children(epic_id)?;
        if children.is_empty() {
            return Ok(());
        }

        let all_done = children
            .iter()
            .all(|c| c.status == crate::models::Status::Done);
        let any_done = children
            .iter()
            .any(|c| c.status == crate::models::Status::Done);
        let any_in_progress = children
            .iter()
            .any(|c| c.status == crate::models::Status::InProgress);

        let new_status = if all_done {
            "done"
        } else if any_done || any_in_progress {
            "in_progress"
        } else {
            "open"
        };

        // Only update if status actually changed
        let epic = self.get_task(epic_id)?;
        if let Some(epic) = epic {
            let current = epic.status.as_str();
            if current != new_status {
                self.update_task(
                    epic_id,
                    None,
                    None,
                    Some(new_status),
                    None,
                    None,
                    None,
                    None,
                )?;
            }
        }

        Ok(())
    }
}

/// Read the current schema version from the config table.
fn get_schema_version(conn: &Connection) -> Result<i32, String> {
    let mut stmt = conn
        .prepare("SELECT value FROM config WHERE key = 'schema_version'")
        .map_err(|e| format!("failed to read schema_version: {e}"))?;
    let mut rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("failed to query schema_version: {e}"))?;
    match rows.next() {
        Some(Ok(v)) => v
            .parse::<i32>()
            .map_err(|e| format!("invalid schema_version value: {e}")),
        Some(Err(e)) => Err(format!("failed to read schema_version row: {e}")),
        None => Ok(0),
    }
}

/// Persist the schema version to the config table.
fn set_schema_version(conn: &Connection, version: i32) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('schema_version', ?1)",
        params![version.to_string()],
    )
    .map_err(|e| format!("failed to set schema_version: {e}"))?;
    Ok(())
}

/// Run all pending schema migrations in order.
///
/// Each migration should be wrapped in a transaction so that a partial failure
/// does not leave the schema in an inconsistent state. Version 0 is the
/// baseline created by the `CREATE TABLE IF NOT EXISTS` block in `migrate()`;
/// future migrations (v1, v2, ...) will be added as additional `if version < N`
/// blocks here.
fn run_migrations(conn: &Connection) -> Result<(), String> {
    let version = get_schema_version(conn)?;

    // v0 is the baseline -- no ALTER TABLE statements needed.

    if version < 1 {
        apply_migration(conn, 1, "ALTER TABLE tasks ADD COLUMN close_reason TEXT;")?;
    }

    if version < 2 {
        apply_migration(conn, 2, "ALTER TABLE tasks ADD COLUMN notes TEXT;")?;
    }

    if version < 3 {
        apply_migration(
            conn,
            3,
            "CREATE TABLE IF NOT EXISTS projects (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 path       TEXT NOT NULL UNIQUE,
                 name       TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workspaces (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 project_id INTEGER NOT NULL REFERENCES projects(id),
                 path       TEXT NOT NULL UNIQUE,
                 name       TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             ALTER TABLE tasks ADD COLUMN workspace_id INTEGER REFERENCES workspaces(id);
             CREATE INDEX IF NOT EXISTS idx_tasks_workspace ON tasks(workspace_id);",
        )?;
    }

    if version < 4 {
        apply_migration(
            conn,
            4,
            "ALTER TABLE comments ADD COLUMN author TEXT;
             ALTER TABLE comments ADD COLUMN delivered_at TEXT;
             CREATE INDEX IF NOT EXISTS idx_comments_author ON comments(task_id, author);",
        )?;
    }

    if version < 5 {
        apply_migration(
            conn,
            5,
            "ALTER TABLE workspaces ADD COLUMN archived_at TEXT;",
        )?;
    }

    if version < 6 {
        apply_migration(
            conn,
            6,
            "ALTER TABLE workspaces ADD COLUMN restored_at TEXT;",
        )?;
    }

    Ok(())
}

/// Apply one migration atomically: take the write lock, re-check the version (another
/// process may have migrated meanwhile), run `sql`, record the version, commit.
/// Rolls back on any failure so a partial migration never persists.
fn apply_migration(conn: &Connection, target: i32, sql: &str) -> Result<(), String> {
    let fail = |e: rusqlite::Error| format!("migration v{target} failed: {e}");
    conn.execute_batch("BEGIN IMMEDIATE").map_err(fail)?;
    let result = (|| {
        if get_schema_version(conn)? >= target {
            return Ok(());
        }
        conn.execute_batch(sql).map_err(fail)?;
        set_schema_version(conn, target)
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(fail),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Build a ` WHERE ...` clause (or empty string) for a scope on `tasks.workspace_id`,
/// with its bound parameters.
fn scope_where(scope: &ScopeFilter) -> (String, Vec<i64>) {
    match scope.condition("workspace_id", 1) {
        Some((c, v)) => (format!(" WHERE {c}"), v.into_iter().collect()),
        None => (String::new(), Vec::new()),
    }
}

/// Map a comment row whose columns (id, task_id, body, created_at, author) start at `base`.
fn row_to_comment(row: &rusqlite::Row, base: usize) -> Comment {
    let created_str: String = row.get(base + 3).unwrap_or_default();
    Comment {
        id: row.get(base).unwrap_or_default(),
        task_id: row.get(base + 1).unwrap_or_default(),
        body: row.get(base + 2).unwrap_or_default(),
        created_at: parse_ts(&created_str),
        author: row.get(base + 4).unwrap_or_default(),
    }
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        created_at: parse_ts(&row.get::<_, String>(3)?),
    })
}

fn row_to_workspace(row: &rusqlite::Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: row.get(0)?,
        project_id: row.get(1)?,
        path: row.get(2)?,
        name: row.get(3)?,
        created_at: parse_ts(&row.get::<_, String>(4)?),
        archived_at: row.get::<_, Option<String>>(5)?.map(|t| parse_ts(&t)),
        restored_at: row.get::<_, Option<String>>(6)?.map(|t| parse_ts(&t)),
    })
}

/// Return `true` if inserting the edge `child_id → parent_id` would create a cycle.
///
/// The dependency table records that `child_id` is blocked by `parent_id`.  A
/// cycle exists when `parent_id` already transitively depends on `child_id`
/// (i.e. `child_id` is reachable by following dependency edges starting from
/// `parent_id`).
///
/// The BFS walks from `parent_id` through its own blockers (rows where
/// `child_id = current`), looking for `child_id` in the visited set.  The
/// search is bounded by the total number of distinct nodes in the graph, so it
/// always terminates even on a large but acyclic graph.
fn would_create_cycle(conn: &Connection, child_id: &str, parent_id: &str) -> Result<bool, String> {
    use std::collections::{HashSet, VecDeque};

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    visited.insert(parent_id.to_string());
    queue.push_back(parent_id.to_string());

    while let Some(current) = queue.pop_front() {
        // Fetch all tasks that `current` depends on (its direct blockers)
        let mut stmt = conn
            .prepare("SELECT parent_id FROM dependencies WHERE child_id = ?1")
            .map_err(|e| format!("query error: {e}"))?;

        let blocker_ids: Vec<String> = stmt
            .query_map(params![current], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        for blocker in blocker_ids {
            if blocker == child_id {
                return Ok(true);
            }
            if !visited.contains(&blocker) {
                visited.insert(blocker.clone());
                queue.push_back(blocker);
            }
        }
    }

    Ok(false)
}

fn row_to_task(row: &rusqlite::Row) -> Task {
    let status_str: String = row.get(3).unwrap_or_default();
    let tags_str: String = row.get(7).unwrap_or_default();
    let created_str: String = row.get(8).unwrap_or_default();
    let updated_str: String = row.get(9).unwrap_or_default();
    let close_reason: Option<String> = row.get(10).unwrap_or(None);
    let notes: Option<String> = row.get(11).unwrap_or(None);

    Task {
        id: row.get(0).unwrap_or_default(),
        title: row.get(1).unwrap_or_default(),
        description: row.get(2).ok(),
        status: Status::from_str(&status_str).unwrap_or(Status::Open),
        priority: row.get::<_, u8>(4).unwrap_or(2),
        assignee: row.get(5).ok().filter(|v: &String| !v.is_empty()),
        parent_id: row.get(6).ok().filter(|v: &String| !v.is_empty()),
        tags: if tags_str.is_empty() {
            Vec::new()
        } else {
            tags_str.split(',').map(|s| s.trim().to_string()).collect()
        },
        created_at: DateTime::parse_from_rfc3339(&created_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: DateTime::parse_from_rfc3339(&updated_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        close_reason,
        notes,
        workspace_id: row.get(12).unwrap_or(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create an in-memory-like temp database and return it along with the TempDir
    /// (must keep TempDir alive for the test's duration).
    fn open_test_db() -> (Database, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).expect("open");
        db.migrate().expect("migrate");
        db.set_config("prefix", "tk").expect("set prefix");
        (db, dir)
    }

    fn make_task(db: &Database, title: &str) -> Task {
        let id = db.generate_id().expect("generate_id");
        let now = Utc::now();
        let task = Task {
            id,
            title: title.to_string(),
            description: None,
            status: crate::models::Status::Open,
            priority: 2,
            assignee: None,
            parent_id: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            close_reason: None,
            notes: None,
            workspace_id: None,
        };
        db.insert_task(&task).expect("insert_task");
        task
    }

    #[test]
    fn test_dependency_chain_empty_no_deps() {
        let (db, _dir) = open_test_db();
        let a = make_task(&db, "A");

        let up = db.get_dependency_chain(&a.id, "up").expect("chain up");
        let down = db.get_dependency_chain(&a.id, "down").expect("chain down");
        let both = db.get_dependency_chain(&a.id, "both").expect("chain both");

        assert!(up.is_empty(), "no blockers expected");
        assert!(down.is_empty(), "no dependents expected");
        assert!(both.is_empty(), "no connections expected");
    }

    #[test]
    fn test_dependency_chain_single_dep() {
        // A is blocked by B (A depends on B)
        let (db, _dir) = open_test_db();
        let a = make_task(&db, "A");
        let b = make_task(&db, "B");
        db.add_dependency(&a.id, &b.id).expect("add dep A->B");

        let up = db.get_dependency_chain(&a.id, "up").expect("chain up");
        assert_eq!(up.len(), 1, "A has one blocker");
        assert_eq!(up[0].0.id, b.id);
        assert_eq!(up[0].1, 1, "depth should be 1");

        let down = db.get_dependency_chain(&b.id, "down").expect("chain down");
        assert_eq!(down.len(), 1, "B has one dependent");
        assert_eq!(down[0].0.id, a.id);
        assert_eq!(down[0].1, 1, "depth should be 1");
    }

    #[test]
    fn test_dependency_chain_linear_chain() {
        // C is blocked by B, B is blocked by A
        // Up from C: [B@1, A@2]
        // Down from A: [B@1, C@2]
        let (db, _dir) = open_test_db();
        let a = make_task(&db, "A");
        let b = make_task(&db, "B");
        let c = make_task(&db, "C");
        db.add_dependency(&b.id, &a.id).expect("B dep A");
        db.add_dependency(&c.id, &b.id).expect("C dep B");

        let up_from_c = db.get_dependency_chain(&c.id, "up").expect("up from C");
        assert_eq!(up_from_c.len(), 2);
        // BFS order: B at depth 1, A at depth 2
        let b_entry = up_from_c.iter().find(|(t, _)| t.id == b.id).expect("B");
        let a_entry = up_from_c.iter().find(|(t, _)| t.id == a.id).expect("A");
        assert_eq!(b_entry.1, 1);
        assert_eq!(a_entry.1, 2);

        let down_from_a = db.get_dependency_chain(&a.id, "down").expect("down from A");
        assert_eq!(down_from_a.len(), 2);
        let b_entry = down_from_a.iter().find(|(t, _)| t.id == b.id).expect("B");
        let c_entry = down_from_a.iter().find(|(t, _)| t.id == c.id).expect("C");
        assert_eq!(b_entry.1, 1);
        assert_eq!(c_entry.1, 2);
    }

    #[test]
    fn test_dependency_chain_diamond() {
        // Diamond: A and B both depend on X.
        // C depends on A and B.
        // Graph (up): C -> A -> X, C -> B -> X
        //             X should appear only once in "up from C"
        let (db, _dir) = open_test_db();
        let x = make_task(&db, "X");
        let a = make_task(&db, "A");
        let b = make_task(&db, "B");
        let c = make_task(&db, "C");
        db.add_dependency(&a.id, &x.id).expect("A dep X");
        db.add_dependency(&b.id, &x.id).expect("B dep X");
        db.add_dependency(&c.id, &a.id).expect("C dep A");
        db.add_dependency(&c.id, &b.id).expect("C dep B");

        let up_from_c = db.get_dependency_chain(&c.id, "up").expect("up from C");

        // Should contain A, B (depth 1) and X (depth 2) — X only once
        assert_eq!(up_from_c.len(), 3, "A, B, X — X appears only once");

        let ids: Vec<&str> = up_from_c.iter().map(|(t, _)| t.id.as_str()).collect();
        assert!(ids.contains(&a.id.as_str()));
        assert!(ids.contains(&b.id.as_str()));
        assert!(ids.contains(&x.id.as_str()));

        // X appears exactly once
        let x_count = up_from_c.iter().filter(|(t, _)| t.id == x.id).count();
        assert_eq!(x_count, 1, "X must appear exactly once (diamond dedup)");

        // X depth should be 2 (reached via A or B)
        let x_depth = up_from_c.iter().find(|(t, _)| t.id == x.id).unwrap().1;
        assert_eq!(x_depth, 2);
    }

    #[test]
    fn test_dependency_chain_invalid_direction() {
        let (db, _dir) = open_test_db();
        let a = make_task(&db, "A");
        let result = db.get_dependency_chain(&a.id, "sideways");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid direction"));
    }

    #[test]
    fn test_dependency_chain_both_direction() {
        // B depends on A, C depends on B
        // From B with "both": should see A (up@1) and C (down@1)
        let (db, _dir) = open_test_db();
        let a = make_task(&db, "A");
        let b = make_task(&db, "B");
        let c = make_task(&db, "C");
        db.add_dependency(&b.id, &a.id).expect("B dep A");
        db.add_dependency(&c.id, &b.id).expect("C dep B");

        let both = db.get_dependency_chain(&b.id, "both").expect("both");
        assert_eq!(both.len(), 2);
        let ids: Vec<&str> = both.iter().map(|(t, _)| t.id.as_str()).collect();
        assert!(ids.contains(&a.id.as_str()));
        assert!(ids.contains(&c.id.as_str()));
    }

    #[test]
    fn test_list_tasks_completed_after_none_returns_all() {
        // completed_after = None should not change results vs. the baseline query
        let (db, _dir) = open_test_db();
        let a = make_task(&db, "Task A");
        let b = make_task(&db, "Task B");

        let all = db
            .list_tasks(true, None, None, None, None, None, None, &ScopeFilter::All)
            .expect("list_tasks with no filter");

        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&a.id.as_str()), "Task A should be returned");
        assert!(ids.contains(&b.id.as_str()), "Task B should be returned");
    }

    #[test]
    fn test_list_tasks_completed_after_filters_by_updated_at() {
        // Only tasks with updated_at > completed_after should be returned
        let (db, _dir) = open_test_db();

        // Insert a task and immediately close it (sets updated_at to now)
        let task = make_task(&db, "Recent Done Task");
        db.update_task(&task.id, None, None, Some("done"), None, None, None, None)
            .expect("close task");

        // A timestamp well in the past — the task updated_at should be after this
        let past_ts = "2000-01-01T00:00:00+00:00";
        // A timestamp well in the future — the task updated_at should be before this
        let future_ts = "2099-01-01T00:00:00+00:00";

        let after_past = db
            .list_tasks(
                true,
                Some("done"),
                None,
                None,
                None,
                None,
                Some(past_ts),
                &ScopeFilter::All,
            )
            .expect("list_tasks completed_after past");
        let ids_past: Vec<&str> = after_past.iter().map(|t| t.id.as_str()).collect();
        assert!(
            ids_past.contains(&task.id.as_str()),
            "done task should be returned when completed_after is in the past"
        );

        let after_future = db
            .list_tasks(
                true,
                Some("done"),
                None,
                None,
                None,
                None,
                Some(future_ts),
                &ScopeFilter::All,
            )
            .expect("list_tasks completed_after future");
        let ids_future: Vec<&str> = after_future.iter().map(|t| t.id.as_str()).collect();
        assert!(
            !ids_future.contains(&task.id.as_str()),
            "done task should NOT be returned when completed_after is in the future"
        );
    }

    // -- Workspace scoping --

    /// Two projects: p1 has w1 and w2; p2 has w3. Returns (p1, p2, w1, w2, w3).
    fn setup_scopes(db: &Database) -> (Project, Project, Workspace, Workspace, Workspace) {
        let p1 = db.upsert_project("/r/one", "one").expect("p1");
        let p2 = db.upsert_project("/r/two", "two").expect("p2");
        let w1 = db.upsert_workspace(p1.id, "/r/one", "one").expect("w1");
        let w2 = db
            .upsert_workspace(p1.id, "/r/one-wt", "one-wt")
            .expect("w2");
        let w3 = db.upsert_workspace(p2.id, "/r/two", "two").expect("w3");
        (p1, p2, w1, w2, w3)
    }

    fn task_in(db: &Database, title: &str, ws: Option<i64>) -> Task {
        let t = make_task(db, title);
        if ws.is_some() {
            db.set_task_workspace(&t.id, ws).expect("assign");
        }
        t
    }

    fn ids(tasks: &[Task]) -> Vec<String> {
        let mut v: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
        v.sort();
        v
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    #[test]
    fn test_latest_schema_version_matches_fresh_migration() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("fresh.db");
        // A fresh DB has no config table, so open() cannot take the fast path: the
        // version it ends at comes from run_migrations alone.
        let db = Database::open(&path).expect("open");
        assert_eq!(
            get_schema_version(&db.conn).expect("version"),
            LATEST_SCHEMA_VERSION
        );
    }

    #[test]
    fn test_reparent_moves_subtree_to_parent_workspace_and_is_atomic() {
        let (db, _dir) = open_test_db();
        let p1 = db.upsert_project("/p1", "p1").expect("p1");
        let w1 = db.upsert_workspace(p1.id, "/p1/w1", "w1").expect("w1");
        let w2 = db.upsert_workspace(p1.id, "/p1/w2", "w2").expect("w2");
        let epic = task_in(&db, "epic", Some(w2.id));
        let x = task_in(&db, "x", Some(w1.id));
        db.update_task_with_parent(
            &x.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Some(&epic.id)),
        )
        .expect("reparent");
        assert_eq!(
            db.get_task(&x.id).unwrap().unwrap().workspace_id,
            Some(w2.id)
        );

        // promote keeps workspace
        db.update_task_with_parent(&x.id, None, None, None, None, None, None, None, Some(None))
            .expect("promote");
        assert_eq!(
            db.get_task(&x.id).unwrap().unwrap().workspace_id,
            Some(w2.id)
        );

        // bad status: nothing changes
        let before = db.get_task(&x.id).unwrap().unwrap();
        let r = db.update_task_with_parent(
            &x.id,
            Some("new"),
            None,
            Some("bogus"),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(r.is_err());
        assert_eq!(db.get_task(&x.id).unwrap().unwrap().title, before.title);
    }

    #[test]
    fn test_set_task_workspace_keeps_updated_at_of_done_tasks() {
        let (db, _dir) = open_test_db();
        let p1 = db.upsert_project("/p1", "p1").expect("p1");
        let w1 = db.upsert_workspace(p1.id, "/p1/w1", "w1").expect("w1");
        let done = make_task(&db, "done");
        db.close_task(&done.id, None).expect("close");
        let open = make_task(&db, "open");
        let d0 = db.get_task(&done.id).unwrap().unwrap().updated_at;
        let o0 = db.get_task(&open.id).unwrap().unwrap().updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        db.set_task_workspace(&done.id, Some(w1.id)).expect("move");
        db.set_task_workspace(&open.id, Some(w1.id)).expect("move");
        assert_eq!(db.get_task(&done.id).unwrap().unwrap().updated_at, d0);
        assert_ne!(db.get_task(&open.id).unwrap().unwrap().updated_at, o0);
    }

    #[test]
    fn test_migration_from_v2_keeps_tasks_unscoped() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("old.db");
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE tasks (
                     id TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT,
                     status TEXT NOT NULL DEFAULT 'open', priority INTEGER NOT NULL DEFAULT 2,
                     assignee TEXT, parent_id TEXT REFERENCES tasks(id),
                     tags TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL, close_reason TEXT, notes TEXT);
                 CREATE TABLE dependencies (child_id TEXT NOT NULL, parent_id TEXT NOT NULL,
                     PRIMARY KEY (child_id, parent_id));
                 CREATE TABLE comments (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     task_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL);
                 INSERT INTO config VALUES ('schema_version', '2');
                 INSERT INTO tasks (id, title, created_at, updated_at)
                     VALUES ('tk-old1', 'old', '2024-01-01T00:00:00+00:00', '2024-01-01T00:00:00+00:00');",
            )
            .expect("seed v2");
        }

        let db = Database::open(&path).expect("open migrates");
        let task = db.get_task("tk-old1").expect("get").expect("exists");
        assert_eq!(task.workspace_id, None);
        assert_eq!(
            get_schema_version(&db.conn).expect("version"),
            LATEST_SCHEMA_VERSION
        );
        assert!(db.list_projects().expect("projects").is_empty());
        let unscoped = db
            .list_tasks(
                true,
                None,
                None,
                None,
                None,
                None,
                None,
                &ScopeFilter::Unscoped,
            )
            .expect("list");
        assert_eq!(unscoped.len(), 1);
    }

    #[test]
    fn test_open_creates_parent_dir_and_is_idempotent() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("nested/deeper/tacks.db");
        let db = Database::open(&path).expect("first open");
        drop(db);
        let db = Database::open(&path).expect("second open");
        assert_eq!(
            get_schema_version(&db.conn).expect("version"),
            LATEST_SCHEMA_VERSION
        );
    }

    #[test]
    fn test_upserts_do_not_consume_ids() {
        let (db, _dir) = open_test_db();
        let mut p = db.upsert_project("/r/one", "one").expect("p");
        let mut w = db.upsert_workspace(p.id, "/r/one", "one").expect("w");
        for _ in 0..10 {
            p = db.upsert_project("/r/one", "one").expect("p again");
            w = db.upsert_workspace(p.id, "/r/one", "one").expect("w again");
        }
        assert_eq!((p.id, w.id), (1, 1));
        let p2 = db.upsert_project("/r/two", "two").expect("p2");
        let w2 = db.upsert_workspace(p2.id, "/r/two", "two").expect("w2");
        assert_eq!((p2.id, w2.id), (2, 2));
    }

    #[test]
    fn test_upserts_are_idempotent() {
        let (db, _dir) = open_test_db();
        let p = db.upsert_project("/r/one", "one").expect("p");
        let p_again = db.upsert_project("/r/one", "renamed").expect("p again");
        assert_eq!(p.id, p_again.id);
        assert_eq!(p_again.name, "one");

        let w = db.upsert_workspace(p.id, "/r/one", "one").expect("w");
        let w_again = db.upsert_workspace(p.id, "/r/one", "x").expect("w again");
        assert_eq!(w.id, w_again.id);
        assert_eq!(db.list_projects().expect("list").len(), 1);
        assert_eq!(db.list_workspaces().expect("list").len(), 1);

        let found = db.find_workspace_by_path("/r/one").expect("find");
        assert_eq!(found.map(|f| f.id), Some(w.id));
        assert!(db.find_workspace_by_path("/nope").expect("find").is_none());
        assert_eq!(
            db.get_workspace(w.id).expect("get").map(|f| f.project_id),
            Some(p.id)
        );
        assert_eq!(
            db.get_project(p.id).expect("get").map(|f| f.name),
            Some("one".into())
        );
    }

    #[test]
    fn test_scope_filters_in_list_ready_blocked_and_counts() {
        let (db, _dir) = open_test_db();
        let (p1, _p2, w1, w2, w3) = setup_scopes(&db);
        let a = task_in(&db, "a", Some(w1.id));
        let b = task_in(&db, "b", Some(w2.id));
        let c = task_in(&db, "c", Some(w3.id));
        let u = task_in(&db, "u", None);

        let list = |scope: ScopeFilter| {
            ids(&db
                .list_tasks(true, None, None, None, None, None, None, &scope)
                .expect("list"))
        };
        assert_eq!(
            list(ScopeFilter::All),
            sorted(vec![a.id.clone(), b.id.clone(), c.id.clone(), u.id.clone()])
        );
        assert_eq!(list(ScopeFilter::Workspace(w1.id)), vec![a.id.clone()]);
        assert_eq!(
            list(ScopeFilter::Project(p1.id)),
            sorted(vec![a.id.clone(), b.id.clone()])
        );
        assert_eq!(list(ScopeFilter::Unscoped), vec![u.id.clone()]);

        // Scope combines with other filters (parameter numbering).
        let filtered = db
            .list_tasks(
                true,
                Some("open"),
                Some(2),
                None,
                None,
                Some("b"),
                Some("2000-01-01T00:00:00+00:00"),
                &ScopeFilter::Project(p1.id),
            )
            .expect("combined");
        assert_eq!(ids(&filtered), vec![b.id.clone()]);

        // ready
        let ready =
            |scope: ScopeFilter, limit| ids(&db.get_ready_tasks(limit, &scope).expect("ready"));
        assert_eq!(ready(ScopeFilter::All, None).len(), 4);
        assert_eq!(
            ready(ScopeFilter::Workspace(w3.id), None),
            vec![c.id.clone()]
        );
        assert_eq!(ready(ScopeFilter::Project(p1.id), Some(10)).len(), 2);
        assert_eq!(ready(ScopeFilter::Project(p1.id), Some(1)).len(), 1);
        assert_eq!(ready(ScopeFilter::Unscoped, None), vec![u.id.clone()]);

        // blocked: b blocked by c (cross-workspace blocker still counts)
        db.add_dependency(&b.id, &c.id).expect("dep");
        let blocked = |scope: ScopeFilter| ids(&db.get_blocked_tasks(&scope).expect("blocked"));
        assert_eq!(blocked(ScopeFilter::All), vec![b.id.clone()]);
        assert_eq!(blocked(ScopeFilter::Workspace(w2.id)), vec![b.id.clone()]);
        assert!(blocked(ScopeFilter::Workspace(w1.id)).is_empty());
        assert_eq!(blocked(ScopeFilter::Project(p1.id)), vec![b.id.clone()]);
        assert!(blocked(ScopeFilter::Unscoped).is_empty());

        // counts
        db.update_tags(&a.id, &["x".to_string()]).expect("tag");
        db.update_tags(&c.id, &["x".to_string(), "y".to_string()])
            .expect("tag");
        let total = |v: Vec<(String, i64)>| v.iter().map(|(_, n)| n).sum::<i64>();
        assert_eq!(
            total(db.task_count_by_status(&ScopeFilter::All).unwrap()),
            4
        );
        assert_eq!(
            total(
                db.task_count_by_status(&ScopeFilter::Project(p1.id))
                    .unwrap()
            ),
            2
        );
        assert_eq!(
            total(db.task_count_by_status(&ScopeFilter::Unscoped).unwrap()),
            1
        );
        let prio = db
            .task_count_by_priority(&ScopeFilter::Workspace(w1.id))
            .unwrap();
        assert_eq!(prio, vec![(2, 1)]);
        assert_eq!(
            db.task_count_by_tag(&ScopeFilter::All).unwrap(),
            vec![("x".to_string(), 2), ("y".to_string(), 1)]
        );
        assert_eq!(
            db.task_count_by_tag(&ScopeFilter::Workspace(w3.id))
                .unwrap(),
            vec![("x".to_string(), 1), ("y".to_string(), 1)]
        );
        assert!(
            db.task_count_by_tag(&ScopeFilter::Unscoped)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_workspace_task_counts() {
        let (db, _dir) = open_test_db();
        let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        let a = task_in(&db, "a", Some(w1.id));
        task_in(&db, "b", Some(w1.id));
        task_in(&db, "u", None);
        db.close_task(&a.id, None).expect("close");

        let counts = db.workspace_task_counts().expect("counts");
        let w = counts
            .iter()
            .find(|c| c.workspace_id == Some(w1.id))
            .expect("w1");
        assert_eq!((w.open, w.done, w.total()), (1, 1, 2));
        let un = counts
            .iter()
            .find(|c| c.workspace_id.is_none())
            .expect("unscoped");
        assert_eq!(un.total(), 1);
    }

    #[test]
    fn test_set_task_workspace_moves_descendants() {
        let (db, _dir) = open_test_db();
        let (_p1, _p2, w1, w2, _w3) = setup_scopes(&db);
        let parent = task_in(&db, "parent", Some(w1.id));
        let child_id = db.generate_child_id(&parent.id).expect("child id");
        let now = Utc::now();
        let child = Task {
            id: child_id,
            title: "child".into(),
            description: None,
            status: Status::Open,
            priority: 2,
            assignee: None,
            parent_id: Some(parent.id.clone()),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            close_reason: None,
            notes: None,
            workspace_id: Some(w1.id),
        };
        db.insert_task(&child).expect("insert child");
        let other = task_in(&db, "other", Some(w1.id));

        let moved = db
            .set_task_workspace(&parent.id, Some(w2.id))
            .expect("move");
        assert_eq!(moved, 2);
        let get = |id: &str| db.get_task(id).unwrap().unwrap();
        assert_eq!(get(&parent.id).workspace_id, Some(w2.id));
        assert_eq!(get(&child.id).workspace_id, Some(w2.id));
        assert_eq!(get(&other.id).workspace_id, Some(w1.id));
        assert!(get(&parent.id).updated_at >= parent.updated_at);

        // Moving a child alone does not touch the parent.
        db.set_task_workspace(&child.id, None)
            .expect("unscope child");
        assert_eq!(get(&child.id).workspace_id, None);
        assert_eq!(get(&parent.id).workspace_id, Some(w2.id));

        assert!(db.set_task_workspace("tk-nope", None).is_err());
        assert!(db.set_task_workspace(&parent.id, Some(9999)).is_err());
    }

    #[test]
    fn test_migration_from_v3_adds_comment_columns() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("v3.db");
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE tasks (
                     id TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT,
                     status TEXT NOT NULL DEFAULT 'open', priority INTEGER NOT NULL DEFAULT 2,
                     assignee TEXT, parent_id TEXT, tags TEXT NOT NULL DEFAULT '',
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     close_reason TEXT, notes TEXT, workspace_id INTEGER);
                 CREATE TABLE projects (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     path TEXT NOT NULL UNIQUE, name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     project_id INTEGER NOT NULL, path TEXT NOT NULL UNIQUE,
                     name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE dependencies (child_id TEXT NOT NULL, parent_id TEXT NOT NULL,
                     PRIMARY KEY (child_id, parent_id));
                 CREATE TABLE comments (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     task_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL);
                 INSERT INTO config VALUES ('schema_version', '3');
                 INSERT INTO tasks (id, title, created_at, updated_at)
                     VALUES ('tk-old1', 'old', '2024-01-01T00:00:00+00:00', '2024-01-01T00:00:00+00:00');
                 INSERT INTO comments (task_id, body, created_at)
                     VALUES ('tk-old1', 'legacy', '2024-01-01T00:00:00+00:00');",
            )
            .expect("seed v3");
        }
        let db = Database::open(&path).expect("open migrates");
        assert_eq!(get_schema_version(&db.conn).unwrap(), LATEST_SCHEMA_VERSION);
        let comments = db.get_comments("tk-old1").expect("comments");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, None);
        assert!(db.task_pending_user_comments("tk-old1").unwrap().is_empty());
    }

    #[test]
    fn test_pending_user_comments_logic() {
        let (db, _d) = open_test_db();
        let t = make_task(&db, "t");
        let all = ScopeFilter::All;
        assert!(db.pending_user_comments(&all).unwrap().is_empty());

        let u1 = db.add_comment_by(&t.id, "u1", Some("user")).unwrap();
        let u2 = db.add_comment_by(&t.id, "u2", Some("user")).unwrap();
        let pending = db.pending_user_comments(&all).unwrap();
        assert_eq!(
            pending.iter().map(|p| p.comment.id).collect::<Vec<_>>(),
            vec![u1.id, u2.id]
        );
        assert_eq!(pending[0].task_title, "t");
        assert_eq!(pending[0].task_status, "open");
        assert_eq!(
            db.pending_user_comment_count_by_task(&all).unwrap()[&t.id],
            2
        );
        assert_eq!(db.task_pending_user_comments(&t.id).unwrap().len(), 2);

        // Agent reply clears all earlier user comments.
        db.add_comment_by(&t.id, "reply", Some("agent")).unwrap();
        assert!(db.pending_user_comments(&all).unwrap().is_empty());
        assert!(
            db.pending_user_comment_count_by_task(&all)
                .unwrap()
                .is_empty()
        );

        // New user comment after the reply is pending again.
        let u3 = db.add_comment_by(&t.id, "u3", Some("user")).unwrap();
        let p = db.task_pending_user_comments(&t.id).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, u3.id);
        assert_eq!(p[0].author.as_deref(), Some("user"));

        // NULL-author comment also counts as a reply.
        db.add_comment(&t.id, "legacy reply").unwrap();
        assert!(db.task_pending_user_comments(&t.id).unwrap().is_empty());

        // Done tasks are excluded.
        let d = make_task(&db, "d");
        db.add_comment_by(&d.id, "late", Some("user")).unwrap();
        assert_eq!(db.pending_user_comments(&all).unwrap().len(), 1);
        db.close_task(&d.id, None).unwrap();
        assert!(db.pending_user_comments(&all).unwrap().is_empty());
        assert!(db.task_pending_user_comments(&d.id).unwrap().is_empty());
    }

    #[test]
    fn test_pending_user_comments_scope_filtering() {
        let (db, _d) = open_test_db();
        let (p1, _p2, w1, _w2, w3) = setup_scopes(&db);
        let a = task_in(&db, "a", Some(w1.id));
        let b = task_in(&db, "b", Some(w3.id));
        let u = task_in(&db, "u", None);
        for t in [&a, &b, &u] {
            db.add_comment_by(&t.id, "hi", Some("user")).unwrap();
        }
        let ids_for = |s: ScopeFilter| {
            let mut v: Vec<String> = db
                .pending_user_comments(&s)
                .unwrap()
                .into_iter()
                .map(|p| p.task_id)
                .collect();
            v.sort();
            v
        };
        assert_eq!(ids_for(ScopeFilter::All).len(), 3);
        assert_eq!(ids_for(ScopeFilter::Workspace(w1.id)), vec![a.id.clone()]);
        assert_eq!(ids_for(ScopeFilter::Project(p1.id)), vec![a.id.clone()]);
        assert_eq!(ids_for(ScopeFilter::Unscoped), vec![u.id.clone()]);
        assert!(ids_for(ScopeFilter::Nothing).is_empty());
        assert_eq!(
            db.pending_user_comment_count_by_task(&ScopeFilter::Workspace(w3.id))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_claim_undelivered_is_idempotent() {
        let (db, _d) = open_test_db();
        let t = make_task(&db, "t");
        let c1 = db.add_comment_by(&t.id, "1", Some("user")).unwrap();
        let c2 = db.add_comment_by(&t.id, "2", Some("user")).unwrap();
        let all = ScopeFilter::All;
        assert_eq!(db.undelivered_pending_user_comments(&all).unwrap().len(), 2);

        let mut first = true;
        let (claimed, remaining) = db
            .claim_undelivered_pending_user_comments(&all, |_| std::mem::take(&mut first))
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].comment.id, c1.id);
        assert_eq!(remaining, 1);
        let und = db.undelivered_pending_user_comments(&all).unwrap();
        assert_eq!(und.len(), 1);
        assert_eq!(und[0].comment.id, c2.id);
        // Still pending overall.
        assert_eq!(db.pending_user_comments(&all).unwrap().len(), 2);

        let (again, _) = db
            .claim_undelivered_pending_user_comments(&all, |_| true)
            .unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].comment.id, c2.id);
        let (none, remaining) = db
            .claim_undelivered_pending_user_comments(&all, |_| true)
            .unwrap();
        assert!(none.is_empty());
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_claim_undelivered_is_exclusive_and_budgeted() {
        let (db, d) = open_test_db();
        let t = make_task(&db, "t");
        for i in 0..3 {
            db.add_comment_by(&t.id, &format!("c{i}"), Some("user"))
                .unwrap();
        }
        let all = ScopeFilter::All;
        assert!(db.any_undelivered_pending_user_comment());
        // budget: take only two
        let mut n = 0;
        let (got, rest) = db
            .claim_undelivered_pending_user_comments(&all, |_| {
                n += 1;
                n <= 2
            })
            .unwrap();
        assert_eq!((got.len(), rest), (2, 1));
        // second claim gets only the remainder, third gets nothing
        let (got, rest) = db
            .claim_undelivered_pending_user_comments(&all, |_| true)
            .unwrap();
        assert_eq!((got.len(), rest), (1, 0));
        let (got, _) = db
            .claim_undelivered_pending_user_comments(&all, |_| true)
            .unwrap();
        assert!(got.is_empty());
        assert!(!db.any_undelivered_pending_user_comment());
        // still pending (shown by prime)
        assert_eq!(db.pending_user_comments(&all).unwrap().len(), 3);
        drop(d);
    }

    #[test]
    fn test_claim_is_exclusive_across_threads() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("race.db");
        {
            let db = Database::open(&path).unwrap();
            let t = make_task(&db, "t");
            for i in 0..20 {
                db.add_comment_by(&t.id, &format!("c{i}"), Some("user"))
                    .unwrap();
            }
        }
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let db = Database::open(&path).unwrap();
                    db.claim_undelivered_pending_user_comments(&ScopeFilter::All, |_| true)
                        .unwrap()
                        .0
                        .into_iter()
                        .map(|p| p.comment.id)
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut ids: Vec<i64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, 20);
        assert_eq!(ids.len(), 20);
    }

    #[test]
    fn test_read_only_probe_tolerates_old_schema() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE tasks (id TEXT PRIMARY KEY, status TEXT);
                 CREATE TABLE comments (id INTEGER PRIMARY KEY, task_id TEXT, body TEXT);",
            )
            .unwrap();
        }
        let ro = Database::open_read_only(&path).unwrap();
        assert!(!ro.any_undelivered_pending_user_comment());
        // read-only open must not create a missing database
        assert!(Database::open_read_only(&dir.path().join("nope.db")).is_err());
    }

    #[test]
    fn test_migration_from_v4_adds_archived_at() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("v4.db");
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE projects (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     path TEXT NOT NULL UNIQUE, name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     project_id INTEGER NOT NULL, path TEXT NOT NULL UNIQUE,
                     name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE tasks (
                     id TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT,
                     status TEXT NOT NULL DEFAULT 'open', priority INTEGER NOT NULL DEFAULT 2,
                     assignee TEXT, parent_id TEXT, tags TEXT NOT NULL DEFAULT '',
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     close_reason TEXT, notes TEXT, workspace_id INTEGER);
                 CREATE TABLE dependencies (child_id TEXT NOT NULL, parent_id TEXT NOT NULL,
                     PRIMARY KEY (child_id, parent_id));
                 CREATE TABLE comments (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     task_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL,
                     author TEXT, delivered_at TEXT);
                 INSERT INTO config VALUES ('schema_version', '4');
                 INSERT INTO projects VALUES (1, '/r', 'r', '2024-01-01T00:00:00+00:00');
                 INSERT INTO workspaces VALUES (1, 1, '/r/w', 'w', '2024-01-01T00:00:00+00:00');",
            )
            .expect("seed v4");
        }
        let db = Database::open(&path).expect("open migrates");
        assert_eq!(get_schema_version(&db.conn).unwrap(), LATEST_SCHEMA_VERSION);
        let ws = db.get_workspace(1).unwrap().expect("workspace kept");
        assert!(ws.archived_at.is_none());
    }

    #[test]
    fn test_migration_from_v5_adds_restored_at() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("v5.db");
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE projects (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     path TEXT NOT NULL UNIQUE, name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     project_id INTEGER NOT NULL, path TEXT NOT NULL UNIQUE,
                     name TEXT NOT NULL, created_at TEXT NOT NULL, archived_at TEXT);
                 CREATE TABLE tasks (
                     id TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT,
                     status TEXT NOT NULL DEFAULT 'open', priority INTEGER NOT NULL DEFAULT 2,
                     assignee TEXT, parent_id TEXT, tags TEXT NOT NULL DEFAULT '',
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     close_reason TEXT, notes TEXT, workspace_id INTEGER);
                 CREATE TABLE dependencies (child_id TEXT NOT NULL, parent_id TEXT NOT NULL,
                     PRIMARY KEY (child_id, parent_id));
                 CREATE TABLE comments (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     task_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL,
                     author TEXT, delivered_at TEXT);
                 INSERT INTO config VALUES ('schema_version', '5');
                 INSERT INTO projects VALUES (1, '/r', 'r', '2024-01-01T00:00:00+00:00');
                 INSERT INTO workspaces VALUES (1, 1, '/r/w', 'w', '2024-01-01T00:00:00+00:00', NULL);",
            )
            .expect("seed v5");
        }
        let db = Database::open(&path).expect("open migrates");
        assert_eq!(get_schema_version(&db.conn).unwrap(), LATEST_SCHEMA_VERSION);
        let ws = db.get_workspace(1).unwrap().expect("workspace kept");
        assert!(ws.restored_at.is_none());
    }

    fn set_status(db: &Database, id: &str, status: &str) {
        db.conn
            .execute(
                "UPDATE tasks SET status = ?1 WHERE id = ?2",
                params![status, id],
            )
            .expect("set status");
    }

    #[test]
    fn test_auto_archive_missing_all_done() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        let t = task_in(&db, "a", Some(w1.id));
        set_status(&db, &t.id, "done");
        let before = db.get_task(&t.id).unwrap().unwrap();
        let ids = db
            .auto_archive_removed_workspaces(|p| p == w1.path)
            .unwrap();
        assert_eq!(ids, vec![w1.id]);
        assert!(
            db.get_workspace(w1.id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_some()
        );
        let after = db.get_task(&t.id).unwrap().unwrap();
        assert_eq!(after.updated_at, before.updated_at);
    }

    #[test]
    fn test_auto_archive_skips_workspace_with_unfinished_tasks() {
        for status in ["open", "in_progress", "blocked"] {
            let (db, _d) = open_test_db();
            let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
            let a = task_in(&db, "a", Some(w1.id));
            let b = task_in(&db, "b", Some(w1.id));
            set_status(&db, &a.id, "done");
            set_status(&db, &b.id, status);
            let ids = db.auto_archive_removed_workspaces(|_| true).unwrap();
            assert!(!ids.contains(&w1.id), "status {status}");
            assert!(
                db.get_workspace(w1.id)
                    .unwrap()
                    .unwrap()
                    .archived_at
                    .is_none()
            );
        }
    }

    #[test]
    fn test_auto_archive_skips_present_workspace() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        let ids = db.auto_archive_removed_workspaces(|_| false).unwrap();
        assert!(ids.is_empty());
        assert!(
            db.get_workspace(w1.id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_none()
        );
    }

    #[test]
    fn test_auto_archive_keeps_existing_archived_timestamp() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        let stamp = db.archive_workspace(w1.id).unwrap().archived_at;
        let ids = db.auto_archive_removed_workspaces(|_| true).unwrap();
        assert!(!ids.contains(&w1.id));
        assert_eq!(db.get_workspace(w1.id).unwrap().unwrap().archived_at, stamp);
    }

    #[test]
    fn test_auto_archive_skips_restored_workspace() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        db.archive_workspace(w1.id).unwrap();
        let restored = db.restore_workspace(w1.id).unwrap();
        assert!(restored.restored_at.is_some());
        let stamp = restored.restored_at;
        // idempotent restore keeps the original restored_at
        assert_eq!(db.restore_workspace(w1.id).unwrap().restored_at, stamp);
        let ids = db.auto_archive_removed_workspaces(|_| true).unwrap();
        assert!(!ids.contains(&w1.id));
        assert!(
            db.get_workspace(w1.id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_none()
        );
    }

    #[test]
    fn test_auto_archive_zero_tasks_and_returns_ids() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, w2, w3) = setup_scopes(&db);
        let t = task_in(&db, "a", Some(w2.id));
        set_status(&db, &t.id, "open");
        let ids = db
            .auto_archive_removed_workspaces(|p| p == w1.path || p == w2.path)
            .unwrap();
        assert_eq!(ids, vec![w1.id]);
        assert!(
            db.get_workspace(w3.id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_none()
        );
    }

    #[test]
    fn test_archive_and_restore_are_idempotent() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        let t = task_in(&db, "a", Some(w1.id));
        let t = db.get_task(&t.id).unwrap().unwrap();

        let first = db.archive_workspace(w1.id).unwrap();
        let stamp = first.archived_at.expect("archived");
        let again = db.archive_workspace(w1.id).unwrap();
        assert_eq!(
            again.archived_at,
            Some(stamp),
            "timestamp kept on re-archive"
        );
        // tasks are untouched
        let after = db.get_task(&t.id).unwrap().unwrap();
        assert_eq!(after.updated_at, t.updated_at);
        assert_eq!(after.workspace_id, Some(w1.id));
        // list_workspaces still returns it
        assert!(db.list_workspaces().unwrap().iter().any(|w| w.id == w1.id));

        assert!(db.restore_workspace(w1.id).unwrap().archived_at.is_none());
        assert!(db.restore_workspace(w1.id).unwrap().archived_at.is_none());
        assert!(db.archive_workspace(9999).is_err());
        assert!(db.restore_workspace(9999).is_err());
    }

    #[test]
    fn test_upsert_workspace_restores_archived_and_keeps_id() {
        let (db, _d) = open_test_db();
        let (p1, _p2, w1, _w2, _w3) = setup_scopes(&db);
        db.archive_workspace(w1.id).unwrap();
        let again = db.upsert_workspace(p1.id, &w1.path, &w1.name).unwrap();
        assert_eq!(again.id, w1.id);
        assert!(again.archived_at.is_none());
        assert!(
            db.get_workspace(w1.id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_none()
        );
    }

    #[test]
    fn test_visible_scopes_exclude_archived_workspaces() {
        let (db, _d) = open_test_db();
        let (p1, _p2, w1, w2, w3) = setup_scopes(&db);
        let a = task_in(&db, "a", Some(w1.id));
        let b = task_in(&db, "b", Some(w2.id));
        let c = task_in(&db, "c", Some(w3.id));
        let u = task_in(&db, "u", None);
        let blocker = task_in(&db, "blocker", Some(w1.id));
        db.add_dependency(&b.id, &blocker.id).unwrap();
        for t in [&a, &b, &c, &u] {
            db.add_comment_by(&t.id, "hi", Some("user")).unwrap();
        }
        db.archive_workspace(w1.id).unwrap();

        let list = |s: ScopeFilter| {
            ids(&db
                .list_tasks(true, None, None, None, None, None, None, &s)
                .unwrap())
        };
        assert_eq!(list(ScopeFilter::All).len(), 5, "CLI scope sees everything");
        assert_eq!(
            list(ScopeFilter::AllVisible),
            sorted(vec![b.id.clone(), c.id.clone(), u.id.clone()])
        );
        assert_eq!(list(ScopeFilter::ProjectVisible(p1.id)), vec![b.id.clone()]);
        assert_eq!(list(ScopeFilter::Project(p1.id)).len(), 3);

        let ready = db.get_ready_tasks(None, &ScopeFilter::AllVisible).unwrap();
        assert_eq!(ids(&ready), sorted(vec![c.id.clone(), u.id.clone()]));
        // the blocked task's blocker is archived but b itself is visible
        let blocked = db.get_blocked_tasks(&ScopeFilter::AllVisible).unwrap();
        assert_eq!(ids(&blocked), vec![b.id.clone()]);

        let total = |v: Vec<(String, i64)>| v.iter().map(|(_, n)| n).sum::<i64>();
        assert_eq!(
            total(db.task_count_by_status(&ScopeFilter::AllVisible).unwrap()),
            3
        );
        assert_eq!(
            total(
                db.task_count_by_status(&ScopeFilter::ProjectVisible(p1.id))
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            db.task_count_by_priority(&ScopeFilter::AllVisible)
                .unwrap()
                .iter()
                .map(|(_, n)| n)
                .sum::<i64>(),
            3
        );

        let pending = |s: ScopeFilter| {
            let mut v: Vec<String> = db
                .pending_user_comments(&s)
                .unwrap()
                .into_iter()
                .map(|p| p.task_id)
                .collect();
            v.sort();
            v
        };
        assert_eq!(pending(ScopeFilter::All).len(), 4);
        assert_eq!(
            pending(ScopeFilter::AllVisible),
            sorted(vec![b.id.clone(), c.id.clone(), u.id.clone()])
        );
        assert_eq!(
            db.pending_user_comment_count_by_task(&ScopeFilter::ProjectVisible(p1.id))
                .unwrap()
                .len(),
            1
        );

        db.restore_workspace(w1.id).unwrap();
        assert_eq!(list(ScopeFilter::AllVisible).len(), 5);
    }

    #[test]
    fn test_close_workspace_tasks_closes_only_that_workspaces_open_tasks() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, w2, _w3) = setup_scopes(&db);
        let open = task_in(&db, "open", Some(w1.id));
        let wip = task_in(&db, "wip", Some(w1.id));
        db.update_task(
            &wip.id,
            None,
            None,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let done = task_in(&db, "done", Some(w1.id));
        db.update_task(
            &done.id,
            None,
            None,
            Some("done"),
            None,
            None,
            Some("stale"),
            None,
        )
        .unwrap();
        let done_before = db.get_task(&done.id).unwrap().unwrap();
        let other = task_in(&db, "other", Some(w2.id));
        let unscoped = task_in(&db, "unscoped", None);

        assert_eq!(db.count_open_workspace_tasks(w1.id).unwrap(), 2);
        assert!(
            db.close_workspace_tasks(w1.id, "bogus", None, None)
                .is_err()
        );
        assert!(db.close_workspace_tasks(9999, "done", None, None).is_err());

        let n = db
            .close_workspace_tasks(w1.id, "done", Some("wrapping up"), Some("user"))
            .unwrap();
        assert_eq!(n, 2);
        for t in [&open, &wip] {
            let got = db.get_task(&t.id).unwrap().unwrap();
            assert_eq!(got.status, Status::Done);
            assert_eq!(got.close_reason.as_deref(), Some("done"));
            let c = db.get_comments(&t.id).unwrap();
            assert_eq!(c.len(), 1);
            assert_eq!(c[0].body, "wrapping up");
            assert_eq!(c[0].author.as_deref(), Some("user"));
        }
        let done_after = db.get_task(&done.id).unwrap().unwrap();
        assert_eq!(done_after.updated_at, done_before.updated_at);
        assert_eq!(done_after.close_reason.as_deref(), Some("stale"));
        assert!(db.get_comments(&done.id).unwrap().is_empty());
        for t in [&other, &unscoped] {
            assert_eq!(db.get_task(&t.id).unwrap().unwrap().status, Status::Open);
        }
        assert_eq!(db.count_open_workspace_tasks(w1.id).unwrap(), 0);
        assert_eq!(
            db.close_workspace_tasks(w1.id, "done", None, None).unwrap(),
            0
        );
    }

    #[test]
    fn test_close_workspace_tasks_syncs_epic_in_other_workspace() {
        let (db, _d) = open_test_db();
        let (_p1, _p2, w1, w2, _w3) = setup_scopes(&db);
        let epic = task_in(&db, "epic", Some(w1.id));
        let child = task_in(&db, "child", Some(w1.id));
        db.update_task_with_parent(
            &child.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Some(&epic.id)),
        )
        .unwrap();
        db.set_task_workspace(&epic.id, Some(w2.id)).unwrap();
        // moving the epic moved its subtree; move the child back to w1 explicitly
        db.conn
            .execute(
                "UPDATE tasks SET workspace_id = ?1 WHERE id = ?2",
                params![w1.id, child.id],
            )
            .unwrap();
        assert_eq!(
            db.close_workspace_tasks(w1.id, "done", None, None).unwrap(),
            1
        );
        assert_eq!(db.get_task(&epic.id).unwrap().unwrap().status, Status::Done);
    }
}
