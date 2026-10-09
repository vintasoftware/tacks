use askama::Template;
use axum::Form;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::atomic::Ordering;

use crate::models::{Comment, Task, validate_close_reason};
use crate::web::AppState;
use crate::web::errors::AppError;
use crate::web::render_markdown;
use crate::web::scope::{ApiScope, Layout, Scope, WsGroup, WsInfo};

/// Render an askama template into an axum HTML response.
fn render_template<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template error: {e}"),
        )
            .into_response(),
    }
}

/// Index page handler: the board is the default view. A temporary redirect, so browsers
/// that cached the former `/` to `/tasks` redirect do not keep following it.
pub async fn index() -> Redirect {
    Redirect::temporary("/board")
}

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

/// Request body for POST /api/tasks.
#[derive(Debug, Deserialize)]
pub struct CreateTaskBody {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub tags: Option<Vec<String>>,
    pub parent_id: Option<String>,
    /// Workspace to create the task in. Ignored for subtasks (they inherit the parent's).
    pub workspace_id: Option<i64>,
}

/// Request body for PATCH /api/tasks/:id.
#[derive(Debug, Deserialize)]
pub struct UpdateTaskBody {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub priority: Option<u8>,
    pub assignee: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    /// Reparent: set to a task ID to move under that parent, or "none" to promote to top-level.
    pub parent_id: Option<String>,
    /// Move the task (and its subtasks) to this workspace id; `null` moves it to the
    /// unscoped bucket; absent leaves the workspace unchanged.
    #[serde(default, deserialize_with = "deserialize_present_or_null")]
    pub workspace_id: Option<Option<i64>>,
}

/// Deserialize a field so that "absent" (via `serde(default)`) stays `None`, an explicit
/// `null` becomes `Some(None)` and a value becomes `Some(Some(v))`.
fn deserialize_present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Request body for POST /api/tasks/:id/close.
#[derive(Debug, Deserialize)]
pub struct CloseTaskBody {
    pub reason: Option<String>,
    pub comment: Option<String>,
}

/// Request body for POST /api/tasks/:id/deps.
#[derive(Debug, Deserialize)]
pub struct AddDepBody {
    pub parent_id: String,
}

/// Request body for POST /api/tasks/:id/comments.
#[derive(Debug, Deserialize)]
pub struct AddCommentBody {
    pub body: String,
    /// Optional author label (max 64 chars after trimming); empty or absent means `"user"`.
    #[serde(default)]
    pub author: Option<String>,
}

/// Maximum length of a comment author label, in characters.
const MAX_AUTHOR_CHARS: usize = 64;

/// Resolve the author of a web comment: trimmed, empty/absent becomes `"user"`.
fn resolve_author(author: Option<&str>) -> Result<String, AppError> {
    let a = author.map(str::trim).unwrap_or_default();
    if a.is_empty() {
        return Ok("user".to_string());
    }
    if a.chars().count() > MAX_AUTHOR_CHARS {
        return Err(AppError::Validation(format!(
            "author must be at most {MAX_AUTHOR_CHARS} characters"
        )));
    }
    Ok(a.to_string())
}

// ---------------------------------------------------------------------------
// Query parameter types
// ---------------------------------------------------------------------------

/// Deserialize an optional string field, treating empty strings as `None`.
fn deserialize_empty_string_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s.as_deref() {
        None | Some("") => Ok(None),
        Some(_) => Ok(s),
    }
}

/// Query parameters for GET /api/tasks.
///
/// `status` and `priority` accept comma-separated values for multi-select OR filtering
/// (e.g. `status=open,in_progress` or `priority=1,2`).
#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    /// Comma-separated status values for multi-select OR filtering.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub status: Option<String>,
    /// Comma-separated priority values for multi-select OR filtering.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub priority: Option<String>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub tag: Option<String>,
    pub all: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub parent: Option<String>,
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub search: Option<String>,
}

/// Query parameters for GET /api/tasks/ready.
#[derive(Debug, Deserialize)]
pub struct ReadyTasksQuery {
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Stats response type
// ---------------------------------------------------------------------------

/// Response body for GET /api/stats.
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub by_status: Map<String, Value>,
    pub by_priority: Map<String, Value>,
    pub by_tag: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

/// POST /api/tasks — Create a new task (201).
pub async fn api_create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskBody>,
) -> Result<impl IntoResponse, AppError> {
    // title is required and must not be empty after trimming
    let title = body
        .title
        .ok_or_else(|| AppError::Validation("title is required".to_string()))?;
    let title = {
        let t = title.trim().to_string();
        if t.is_empty() {
            return Err(AppError::Validation("title is required".to_string()));
        }
        t
    };

    check_priority(body.priority)?;
    let priority = body.priority.unwrap_or(2);
    let description = body.description.clone();
    let tags = body.tags.clone().unwrap_or_default();
    let parent_id = body.parent_id.clone();
    let requested_workspace = body.workspace_id;

    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Task, AppError> {
        let db = db.lock().unwrap();

        // Generate ID; subtasks inherit the parent's workspace
        let (id, workspace_id) = if let Some(ref pid) = parent_id {
            // Verify parent exists
            let parent = db
                .get_task(pid)
                .map_err(AppError::Internal)?
                .ok_or_else(|| AppError::NotFound(format!("parent task not found: {pid}")))?;
            (
                db.generate_child_id(pid).map_err(AppError::Internal)?,
                parent.workspace_id,
            )
        } else {
            if let Some(ws) = requested_workspace
                && db.get_workspace(ws).map_err(AppError::Internal)?.is_none()
            {
                return Err(AppError::NotFound(format!("workspace not found: {ws}")));
            }
            (
                db.generate_id().map_err(AppError::Internal)?,
                requested_workspace,
            )
        };

        let now = chrono::Utc::now();
        let task = Task {
            id: id.clone(),
            title: title.clone(),
            description: description.clone(),
            status: crate::models::Status::Open,
            priority,
            assignee: None,
            parent_id: parent_id.clone(),
            tags: tags.clone(),
            created_at: now,
            updated_at: now,
            close_reason: None,
            notes: None,
            workspace_id,
        };

        db.insert_task(&task).map_err(AppError::Internal)?;

        // Auto-tag parent as epic when a child is created, and sync epic status
        if let Some(ref pid) = parent_id {
            let mut parent_tags = db.get_task_tags(pid).map_err(AppError::Internal)?;
            if !parent_tags.contains(&"epic".to_string()) {
                parent_tags.push("epic".to_string());
                db.update_tags(pid, &parent_tags)
                    .map_err(AppError::Internal)?;
            }
            db.sync_epic_status(pid).map_err(AppError::Internal)?;
        }

        Ok(task)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    Ok((StatusCode::CREATED, Json(result)))
}

/// Parse a comma-separated tag query param into a list of trimmed, non-empty tags.
fn parse_tags(tag_param: Option<&str>) -> Vec<String> {
    match tag_param {
        None => vec![],
        Some(s) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    }
}

/// Filter a task list to only tasks that have at least one of the given tags (OR semantics).
fn filter_by_tags(tasks: Vec<Task>, tags: &[String]) -> Vec<Task> {
    if tags.is_empty() {
        return tasks;
    }
    tasks
        .into_iter()
        .filter(|t| tags.iter().any(|tag| t.tags.contains(tag)))
        .collect()
}

/// Highest accepted priority value (0 = critical ... 4 = trivial).
const MAX_PRIORITY: u8 = 4;

/// Reject priorities outside 0..=4 with a 400.
fn check_priority(p: Option<u8>) -> Result<(), AppError> {
    match p {
        Some(v) if v > MAX_PRIORITY => Err(AppError::BadRequest(format!(
            "invalid priority: {v} (expected 0-{MAX_PRIORITY})"
        ))),
        _ => Ok(()),
    }
}

/// Parse comma-separated priority values into a `Vec<u8>`.
fn parse_priority_values(s: &Option<String>) -> Vec<u8> {
    match s.as_deref() {
        None | Some("") => vec![],
        Some(v) => v
            .split(',')
            .filter_map(|p| p.trim().parse::<u8>().ok())
            .collect(),
    }
}

/// Parse comma-separated status values into a `Vec<String>`.
fn parse_status_values(s: &Option<String>) -> Vec<String> {
    match s.as_deref() {
        None | Some("") => vec![],
        Some(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

/// GET /api/tasks — List tasks with optional filters (200).
///
/// `status` and `priority` accept comma-separated values for multi-select OR filtering.
pub async fn api_list_tasks(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
    Query(query): Query<ListTasksQuery>,
) -> Result<impl IntoResponse, AppError> {
    let show_all = query.all.unwrap_or(false);
    let status_values = parse_status_values(&query.status);
    let priority_values = parse_priority_values(&query.priority);
    let tag_param = query.tag.clone();
    let parent_filter = query.parent.clone();
    let search_filter = query.search.clone();

    // Parse comma-separated tags for multi-tag OR filtering
    let tags = parse_tags(tag_param.as_deref());
    // For DB query: use a single tag when exactly one is selected (uses indexed LIKE);
    // when multiple tags, skip DB tag filter and post-filter in Rust.
    let db_tag_filter = if tags.len() == 1 {
        tags.first().cloned()
    } else {
        None
    };
    let multi_tags = if tags.len() > 1 { tags } else { vec![] };

    let db = state.db.clone();
    let tasks = tokio::task::spawn_blocking(move || -> Result<Vec<Task>, String> {
        let db = db.lock().unwrap();
        // For single status/priority, pass directly to DB for efficiency.
        // For multi-value, load without that filter then post-filter in Rust.
        let (db_status, db_priority) = match (status_values.len(), priority_values.len()) {
            (0 | 1, 0 | 1) => (
                status_values.first().map(|s| s.as_str()),
                priority_values.first().copied(),
            ),
            _ => (None, None),
        };
        let mut tasks = db.list_tasks(
            show_all || !status_values.is_empty(),
            db_status,
            db_priority,
            db_tag_filter.as_deref(),
            parent_filter.as_deref(),
            search_filter.as_deref(),
            None,
            &scope,
        )?;
        // Post-filter for multi-value OR semantics
        if status_values.len() > 1 {
            let status_strs: Vec<&str> = status_values.iter().map(|s| s.as_str()).collect();
            tasks.retain(|t| {
                let s = match t.status {
                    crate::models::Status::Open => "open",
                    crate::models::Status::InProgress => "in_progress",
                    crate::models::Status::Done => "done",
                    crate::models::Status::Blocked => "blocked",
                };
                status_strs.contains(&s)
            });
        }
        if priority_values.len() > 1 {
            tasks.retain(|t| priority_values.contains(&t.priority));
        }
        let tasks = filter_by_tags(tasks, &multi_tags);
        Ok(tasks)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(tasks))
}

/// GET /api/tasks/ready — Tasks with no open blockers (200).
pub async fn api_ready_tasks(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
    Query(query): Query<ReadyTasksQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit;
    let db = state.db.clone();
    let tasks = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.get_ready_tasks(limit, &scope)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(tasks))
}

/// GET /api/tasks/blocked — Tasks with open blockers (200).
pub async fn api_blocked_tasks(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let tasks = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.get_blocked_tasks(&scope)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(tasks))
}

/// GET /api/tasks/:id — Show a task by ID (200 or 404).
pub async fn api_show_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let task = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.get_task(&id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    match task {
        Some(t) => Ok(Json(t)),
        None => Err(AppError::NotFound("task not found".to_string())),
    }
}

/// PATCH /api/tasks/:id — Update task fields (200, 400 for invalid input, or 404).
pub async fn api_update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<impl IntoResponse, AppError> {
    check_priority(body.priority)?;
    if let Some(ref s) = body.status {
        s.parse::<crate::models::Status>()
            .map_err(AppError::BadRequest)?;
    }
    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Task, String> {
        let db = db.lock().unwrap();

        // Verify task exists
        db.get_task(&id)?
            .ok_or_else(|| format!("task not found: {id}"))?;

        // Validate everything that can be checked up front, so a bad request changes nothing.
        if let Some(Some(ws)) = body.workspace_id
            && db.get_workspace(ws)?.is_none()
        {
            return Err(format!("workspace not found: {ws}"));
        }
        if let Some(ref s) = body.status {
            s.parse::<crate::models::Status>()?;
        }

        // Convert parent_id field to two-level Option for reparenting
        let new_parent: Option<Option<&str>> = body.parent_id.as_deref().map(|p| {
            if p.eq_ignore_ascii_case("none") || p.is_empty() {
                None
            } else {
                Some(p)
            }
        });

        // One transaction: any failure leaves the task unchanged. Order: fields and
        // reparent (which moves the subtree to the new parent's workspace), then the
        // explicit workspace move, which therefore wins when both are given.
        db.with_savepoint(|| {
            if let Some(ref tags) = body.tags {
                db.update_tags(&id, tags)?;
            }

            db.update_task_with_parent(
                &id,
                body.title.as_deref(),
                body.priority,
                body.status.as_deref(),
                body.description.as_deref(),
                body.assignee.as_deref(),
                None,
                body.notes.as_deref(),
                new_parent,
            )?;

            // Sync parent epic status if this task's status changed
            if body.status.is_some()
                && let Some(task) = db.get_task(&id)?
                && let Some(ref pid) = task.parent_id
            {
                db.sync_epic_status(pid)?;
            }

            if let Some(ws) = body.workspace_id {
                db.set_task_workspace(&id, ws)?;
            }
            Ok(())
        })?;

        // Return the updated task
        db.get_task(&id)?
            .ok_or_else(|| format!("task not found after update: {id}"))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    match result {
        Ok(task) => Ok(Json(task)),
        Err(e) if e.starts_with("cannot reparent") || e.starts_with("circular parenting") => {
            Err(AppError::BadRequest(e))
        }
        Err(e) if e.contains("not found") => Err(AppError::NotFound(e)),
        Err(e) => Err(AppError::Internal(e)),
    }
}

/// POST /api/tasks/:id/close — Close a task (200, 404, or 422).
pub async fn api_close_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CloseTaskBody>,
) -> Result<impl IntoResponse, AppError> {
    // Validate reason if provided
    let reason = body.reason.as_deref();
    if let Some(r) = reason {
        validate_close_reason(r).map_err(AppError::Validation)?;
    }

    let reason_owned = body.reason.clone();
    let comment_owned = body.comment.clone();

    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Task, String> {
        let db = db.lock().unwrap();

        // Verify task exists
        db.get_task(&id)?
            .ok_or_else(|| format!("task not found: {id}"))?;

        // Close the task
        db.close_task(&id, reason_owned.as_deref())?;

        // Add comment if provided
        if let Some(ref comment) = comment_owned {
            db.add_comment_by(&id, comment, Some("user"))?;
        }

        // Return the updated task
        db.get_task(&id)?
            .ok_or_else(|| format!("task not found after close: {id}"))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    match result {
        Ok(task) => Ok(Json(task)),
        Err(e) if e.contains("not found") => Err(AppError::NotFound(e)),
        Err(e) => Err(AppError::Internal(e)),
    }
}

/// POST /api/tasks/:id/deps — Add a dependency (201 or 409).
pub async fn api_add_dep(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AddDepBody>,
) -> Result<impl IntoResponse, AppError> {
    let parent_id = body.parent_id.clone();
    let db = state.db.clone();

    let result = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.add_dependency(&id, &parent_id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    match result {
        Ok(()) => Ok(StatusCode::CREATED),
        Err(e) if e.contains("circular") || e.contains("already exists") => {
            Err(AppError::Conflict(e))
        }
        Err(e) => Err(AppError::Internal(e)),
    }
}

/// DELETE /api/tasks/:child_id/deps/:parent_id — Remove a dependency (204).
pub async fn api_remove_dep(
    State(state): State<AppState>,
    Path((child_id, parent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();

    let result = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.remove_dependency(&child_id, &parent_id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    match result {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) if e.contains("not found") => Err(AppError::NotFound(e)),
        Err(e) => Err(AppError::Internal(e)),
    }
}

/// POST /api/tasks/:id/comments — Add a comment (201). The author defaults to `"user"`.
pub async fn api_add_comment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AddCommentBody>,
) -> Result<impl IntoResponse, AppError> {
    let comment_body = body.body.clone();
    let author = resolve_author(body.author.as_deref())?;
    let db = state.db.clone();

    let comment = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.add_comment_by(&id, &comment_body, Some(&author))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok((StatusCode::CREATED, Json(comment)))
}

/// GET /api/tasks/:id/comments — List comments on a task (200).
pub async fn api_list_comments(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let comments: Vec<Comment> = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.get_comments(&id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(comments))
}

/// GET /api/tasks/:id/children — List subtasks (200).
pub async fn api_children(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let tasks: Vec<Task> = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.get_children(&id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(tasks))
}

/// GET /api/tasks/:id/blockers — List blockers for a task as full Task objects (200).
pub async fn api_blockers(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let tasks: Vec<Task> = tokio::task::spawn_blocking(move || -> Result<Vec<Task>, String> {
        let db = db.lock().unwrap();
        let deps = db.get_blockers(&id)?;
        let mut tasks = Vec::with_capacity(deps.len());
        for dep in deps {
            if let Some(t) = db.get_task(&dep.parent_id)? {
                tasks.push(t);
            }
        }
        Ok(tasks)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(tasks))
}

/// GET /api/tasks/:id/dependents — List tasks that depend on this task (200).
pub async fn api_dependents(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let tasks: Vec<Task> = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.get_dependents(&id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(tasks))
}

/// Response body for GET /api/epics — epic task with child progress counts.
#[derive(Debug, Serialize)]
pub struct EpicProgress {
    pub task: Task,
    pub children_total: usize,
    pub children_done: usize,
    pub children_in_progress: usize,
    pub children_open: usize,
}

/// GET /api/epics — List epics with child completion progress (200).
pub async fn api_epics(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let result: Vec<EpicProgress> =
        tokio::task::spawn_blocking(move || -> Result<Vec<EpicProgress>, String> {
            let db = db.lock().unwrap();
            let epics = db.list_tasks(true, None, None, Some("epic"), None, None, None, &scope)?;
            let mut out = Vec::with_capacity(epics.len());
            for epic in epics {
                let children = db.get_children(&epic.id)?;
                let children_total = children.len();
                let children_done = children
                    .iter()
                    .filter(|c| matches!(c.status, crate::models::Status::Done))
                    .count();
                let children_in_progress = children
                    .iter()
                    .filter(|c| matches!(c.status, crate::models::Status::InProgress))
                    .count();
                let children_open = children_total - children_done - children_in_progress;
                out.push(EpicProgress {
                    task: epic,
                    children_total,
                    children_done,
                    children_in_progress,
                    children_open,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Internal)?;

    Ok(Json(result))
}

/// Response body for GET /api/prime — AI context output.
#[derive(Debug, Serialize)]
pub struct PrimeResponse {
    pub stats: StatsResponse,
    pub in_progress: Vec<Task>,
    pub ready: Vec<Task>,
}

/// GET /api/prime — AI context: stats + in-progress tasks + ready queue (200).
pub async fn api_prime(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<PrimeResponse, String> {
        let db = db.lock().unwrap();

        let by_status_vec = db.task_count_by_status(&scope)?;
        let by_priority_vec = db.task_count_by_priority(&scope)?;
        let by_tag_vec = db.task_count_by_tag(&scope)?;

        let by_status: Map<String, Value> = by_status_vec
            .into_iter()
            .map(|(k, v)| (k, Value::Number(v.into())))
            .collect();

        let by_priority: Map<String, Value> = by_priority_vec
            .into_iter()
            .map(|(k, v)| (k.to_string(), Value::Number(v.into())))
            .collect();

        let by_tag: Map<String, Value> = by_tag_vec
            .into_iter()
            .map(|(k, v)| (k, Value::Number(v.into())))
            .collect();

        let stats = StatsResponse {
            by_status,
            by_priority,
            by_tag,
        };

        let in_progress = db.list_tasks(
            false,
            Some("in_progress"),
            None,
            None,
            None,
            None,
            None,
            &scope,
        )?;
        let ready = db.get_ready_tasks(Some(5), &scope)?;

        Ok(PrimeResponse {
            stats,
            in_progress,
            ready,
        })
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(result))
}

/// GET /api/workspaces — Projects' workspaces with task counts (200).
///
/// Same rows as `tk workspaces --json`; `current` is always false for the web API.
pub async fn api_workspaces(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        crate::workspace_overview::workspace_rows(&db, None)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(rows))
}

/// Parse an id path segment of the workspace endpoints (digits only), else 404.
fn workspace_path_id(raw: &str) -> Result<i64, AppError> {
    crate::web::scope::parse_id(raw)
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))
}

/// Map a database error of a workspace operation to an HTTP error.
fn workspace_db_error(e: String) -> AppError {
    if e.contains("not found") {
        AppError::NotFound("workspace not found".to_string())
    } else {
        AppError::Internal(e)
    }
}

/// GET /api/workspaces/:id — One workspace (200 or 404): the workspace fields plus
/// `project_name` and `open_count` (tasks not done, what close-all would close).
pub async fn api_get_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let id = workspace_path_id(&id)?;
    let db = state.db.clone();
    let value = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let db = db.lock().unwrap();
        let ws = db
            .get_workspace(id)?
            .ok_or_else(|| format!("workspace not found: {id}"))?;
        let project_name = db.get_project(ws.project_id)?.map(|p| p.name);
        let open_count = db.count_open_workspace_tasks(id)?;
        let mut value = serde_json::to_value(&ws).map_err(|e| format!("json error: {e}"))?;
        if let Value::Object(map) = &mut value {
            map.insert("project_name".to_string(), project_name.into());
            map.insert("open_count".to_string(), open_count.into());
        }
        Ok(value)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(workspace_db_error)?;
    Ok(Json(value))
}

/// POST /api/workspaces/:id/archive — Hide a workspace and its tasks from the web UI
/// (200 with the workspace, 404). Tasks are not modified; repeating the call is harmless.
pub async fn api_archive_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let id = workspace_path_id(&id)?;
    let db = state.db.clone();
    let ws = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.archive_workspace(id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(workspace_db_error)?;
    Ok(Json(ws))
}

/// POST /api/workspaces/:id/restore — Bring an archived workspace back (200 with the
/// workspace, 404). Idempotent.
pub async fn api_restore_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let id = workspace_path_id(&id)?;
    let db = state.db.clone();
    let ws = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.restore_workspace(id)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(workspace_db_error)?;
    Ok(Json(ws))
}

/// Optional JSON body of POST /api/workspaces/:id/close-all.
#[derive(Debug, Default, Deserialize)]
pub struct CloseAllBody {
    /// Close reason (default `done`).
    pub reason: Option<String>,
    /// Comment added to every closed task (author `user`).
    pub comment: Option<String>,
}

/// POST /api/workspaces/:id/close-all — Close every task of the workspace that is not
/// done (200 with `{"closed": N}`, 404, 422 for an invalid reason). The JSON body is
/// optional: `{"reason": "done", "comment": "..."}`.
pub async fn api_close_all_workspace_tasks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    let id = workspace_path_id(&id)?;
    let body: CloseAllBody = if body.iter().all(u8::is_ascii_whitespace) {
        CloseAllBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| AppError::BadRequest(e.to_string()))?
    };
    let reason = body.reason.unwrap_or_else(|| "done".to_string());
    validate_close_reason(&reason).map_err(AppError::Validation)?;
    let comment = body
        .comment
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let db = state.db.clone();
    let closed = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.close_workspace_tasks(id, &reason, comment.as_deref(), Some("user"))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(workspace_db_error)?;
    Ok(Json(serde_json::json!({ "closed": closed })))
}

/// GET /api/poll — Lightweight change-detection endpoint for HTMX polling.
///
/// Compares the current SQLite `PRAGMA data_version` against the last known value
/// stored in `AppState`. Returns:
/// - `304 Not Modified` when nothing has changed (HTMX treats this as no-swap)
/// - `200 OK` with `HX-Trigger: data-changed` header when data has changed
pub async fn api_poll(State(state): State<AppState>) -> Response {
    let db = state.db.clone();
    let version = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.data_version()
    })
    .await;

    let current = match version {
        Ok(Ok(v)) => v,
        _ => {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let last = state.last_data_version.load(Ordering::Relaxed);

    if current == last {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        state.last_data_version.store(current, Ordering::Relaxed);
        (
            StatusCode::OK,
            [(axum_htmx::headers::HX_TRIGGER, "data-changed")],
            "",
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// HTML view handlers
// ---------------------------------------------------------------------------

/// A task row enriched with optional parent info and child counts for list/board views.
struct TaskRow {
    task: Task,
    /// Parent epic ID, if this task is a subtask.
    parent_id: Option<String>,
    /// Parent epic title, if this task is a subtask.
    parent_title: Option<String>,
    /// Number of done subtasks (0 if this task has no children).
    children_done: usize,
    /// Total number of subtasks (0 if this task has no children).
    children_total: usize,
    /// Number of open (non-done) blockers for this task.
    blocked_count: usize,
    /// Workspace of the task, set only in project and All scopes (shown as a badge).
    workspace: Option<WsInfo>,
    /// Number of user comments awaiting an agent reply.
    pending_count: i64,
}

impl TaskRow {
    /// Build a `TaskRow` from a task, looking up parent title from the provided map,
    /// child counts from the provided counts map, and blocker counts from the blocker map.
    fn from_task(
        task: Task,
        parents: &std::collections::HashMap<String, Task>,
        child_counts: &std::collections::HashMap<String, (usize, usize)>,
        blocker_counts: &std::collections::HashMap<String, usize>,
    ) -> Self {
        let parent_id = task.parent_id.clone();
        let parent_title = parent_id
            .as_deref()
            .and_then(|pid| parents.get(pid))
            .map(|p| p.title.clone());
        let (children_done, children_total) = child_counts.get(&task.id).copied().unwrap_or((0, 0));
        let blocked_count = blocker_counts.get(&task.id).copied().unwrap_or(0);
        TaskRow {
            task,
            parent_id,
            parent_title,
            children_done,
            children_total,
            blocked_count,
            workspace: None,
            pending_count: 0,
        }
    }

    /// Attach the pending user-comment count from a per-task count map.
    fn with_pending(mut self, pending: &std::collections::HashMap<String, i64>) -> Self {
        self.pending_count = pending.get(&self.task.id).copied().unwrap_or(0);
        self
    }

    /// Attach the workspace badge data when the scope shows badges.
    fn with_workspace(mut self, scope: &Scope) -> Self {
        if scope.show_workspace_badge() {
            self.workspace = self.task.workspace_id.and_then(|id| scope.ws(id)).cloned();
        }
        self
    }
}

/// Fetch a map of task_id -> Task for a set of IDs (used to batch-load parent epics).
fn fetch_parent_map(
    db: &crate::db::Database,
    ids: impl Iterator<Item = String>,
) -> Result<std::collections::HashMap<String, Task>, String> {
    let mut map = std::collections::HashMap::new();
    for id in ids {
        if let Some(t) = db.get_task(&id)? {
            map.insert(id, t);
        }
    }
    Ok(map)
}

/// Batch-fetch child counts for a list of task IDs.
///
/// Returns a map of `task_id -> (done_count, total_count)` for tasks that have
/// at least one child.  Tasks with no children are absent from the map (callers
/// should default to `(0, 0)`).
fn fetch_child_counts(
    db: &crate::db::Database,
    task_ids: &[String],
) -> Result<std::collections::HashMap<String, (usize, usize)>, String> {
    let mut map = std::collections::HashMap::new();
    for id in task_ids {
        let children = db.get_children(id)?;
        if !children.is_empty() {
            let done = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::Done))
                .count();
            map.insert(id.clone(), (done, children.len()));
        }
    }
    Ok(map)
}

/// Batch-fetch the count of open (non-done) blockers for a list of task IDs.
///
/// Returns a map of `task_id -> open_blocker_count` for tasks that have at least one
/// open blocker.  Tasks with no open blockers are absent from the map (callers should
/// default to `0`).
fn fetch_blocker_counts(
    db: &crate::db::Database,
    task_ids: &[String],
) -> Result<std::collections::HashMap<String, usize>, String> {
    let mut map = std::collections::HashMap::new();
    for id in task_ids {
        let blocker_deps = db.get_blockers(id)?;
        let mut open_count = 0usize;
        for dep in &blocker_deps {
            if let Some(blocker) = db.get_task(&dep.parent_id)?
                && !matches!(blocker.status, crate::models::Status::Done)
            {
                open_count += 1;
            }
        }
        if open_count > 0 {
            map.insert(id.clone(), open_count);
        }
    }
    Ok(map)
}

/// Template for the task list page at GET /tasks.
#[derive(Template)]
#[template(path = "tasks_list.html")]
struct TaskListTemplate {
    layout: Layout,
    tasks: Vec<TaskRow>,
    status_filter: Option<String>,
    priority_filter: Option<String>,
    /// Comma-separated tag filter string (may contain multiple tags).
    tag_filter: Option<String>,
    /// Parsed list of selected tags (for rendering pills).
    selected_tags: Vec<String>,
    /// All available tags for the dropdown.
    all_tags: Vec<String>,
    search_filter: Option<String>,
    /// True when any filter (status, priority, tag, search) is active.
    has_filters: bool,
    /// Pre-built query string for HTMX polling (preserves current filters).
    poll_query: String,
}

/// A comment prepared for display: sanitized HTML body and pending state.
struct CommentView {
    comment: Comment,
    /// Sanitized markdown HTML (safe to output unescaped).
    html: String,
    /// True when this is a user comment still awaiting an agent reply.
    pending: bool,
    /// True when the comment was written by the user (author `"user"`).
    is_user: bool,
}

/// Render comments and mark the ones listed in `pending_ids`.
fn build_comment_views(comments: Vec<Comment>, pending_ids: &[i64]) -> Vec<CommentView> {
    comments
        .into_iter()
        .map(|c| CommentView {
            html: render_markdown(&c.body),
            pending: pending_ids.contains(&c.id),
            is_user: c.author.as_deref() == Some("user"),
            comment: c,
        })
        .collect()
}

/// Template for the comments list (GET /tasks/:id/comments, re-rendered after posting).
#[derive(Template)]
#[template(path = "partials/comment_list.html")]
struct CommentListTemplate {
    task: Task,
    comment_views: Vec<CommentView>,
}

/// GET /tasks/:id/comments — HTML fragment with the comments list (200 or 404).
pub async fn task_comments(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let db = state.db.clone();
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CommentListTemplate>, String> {
            let db = db.lock().unwrap();
            let Some(task) = db.get_task(&id)? else {
                return Ok(None);
            };
            let pending: Vec<i64> = db
                .task_pending_user_comments(&id)?
                .iter()
                .map(|c| c.id)
                .collect();
            let comments = db.get_comments(&id)?;
            Ok(Some(CommentListTemplate {
                task,
                comment_views: build_comment_views(comments, &pending),
            }))
        })
        .await
        .unwrap();
    match result {
        Ok(Some(t)) => render_template(t),
        Ok(None) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// Template for the task detail page at GET /tasks/:id.
#[derive(Template)]
#[template(path = "task_detail.html")]
#[allow(dead_code)]
struct TaskDetailTemplate {
    layout: Layout,
    /// Workspace of the task, if any.
    ws: Option<WsInfo>,
    /// Workspace `<option>`s grouped by project for the move selector.
    ws_groups: Vec<WsGroup>,
    task: Task,
    /// Parent epic, if this task is a subtask.
    parent: Option<Task>,
    blockers: Vec<Task>,
    dependents: Vec<Task>,
    /// Comments with sanitized HTML bodies and pending markers.
    comment_views: Vec<CommentView>,
    /// Pre-rendered HTML for the task description (markdown → sanitized HTML).
    description_html: Option<String>,
}

/// Template for the task detail modal fragment loaded via HTMX.
#[derive(Template)]
#[template(path = "task_detail_fragment.html")]
#[allow(dead_code)]
struct TaskDetailFragmentTemplate {
    /// Workspace of the task, if any.
    ws: Option<WsInfo>,
    /// Workspace `<option>`s grouped by project for the move selector.
    ws_groups: Vec<WsGroup>,
    task: Task,
    /// Parent epic, if this task is a subtask.
    parent: Option<Task>,
    blockers: Vec<Task>,
    dependents: Vec<Task>,
    /// Comments with sanitized HTML bodies and pending markers.
    comment_views: Vec<CommentView>,
    /// Pre-rendered HTML for the task description (markdown → sanitized HTML).
    description_html: Option<String>,
}

/// Template for the kanban board page at GET /board.
#[derive(Template)]
#[template(path = "board.html")]
#[allow(dead_code)]
struct BoardTemplate {
    layout: Layout,
    open_tasks: Vec<TaskRow>,
    in_progress_tasks: Vec<TaskRow>,
    blocked_tasks: Vec<TaskRow>,
    done_tasks: Vec<TaskRow>,
    /// All epics available in the dropdown filter.
    epics: Vec<Task>,
    /// Currently selected epic ID filter (empty string = none).
    selected_epic: String,
    /// Currently selected priority filter (empty string = none).
    selected_priority: String,
    /// Currently selected done_since filter value (e.g. "3d", "7d", "30d", "all").
    done_since: String,
    /// Pre-built query string for HTMX polling (preserves current filters).
    poll_query: String,
}

/// Query parameters for GET /board.
#[derive(Debug, Deserialize)]
pub struct BoardQuery {
    /// Comma-separated epic IDs for multi-select OR filtering (e.g. `epic=tk-abc1,tk-def2`).
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub epic: Option<String>,
    /// Comma-separated priority values for multi-select OR filtering (e.g. `priority=1,2`).
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub priority: Option<String>,
    /// How far back to show completed tasks: "3d" (default), "7d", "30d", or "all".
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub done_since: Option<String>,
}

/// Template struct for one row in the epics view.
#[allow(dead_code)]
struct EpicRow {
    task: Task,
    workspace: Option<WsInfo>,
    children_total: usize,
    children_done: usize,
    children_in_progress: usize,
    children_open: usize,
}

/// Template for the epics page at GET /epics.
#[derive(Template)]
#[template(path = "epics.html")]
struct EpicsTemplate {
    layout: Layout,
    epics: Vec<EpicRow>,
    /// Currently active status filter (empty = none).
    status_filter: Option<String>,
    /// Currently active priority filter (empty = none).
    priority_filter: Option<String>,
    /// Pre-built query string for HTMX polling (preserves current filters).
    poll_query: String,
}

/// Query parameters for GET /epics.
#[derive(Debug, Deserialize)]
pub struct EpicsQuery {
    /// Comma-separated status values for multi-select OR filtering.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub status: Option<String>,
    /// Comma-separated priority values for multi-select OR filtering.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub priority: Option<String>,
}

/// Template for the epic detail page at GET /epics/:id.
#[derive(Template)]
#[template(path = "epic_detail.html")]
#[allow(dead_code)]
struct EpicDetailTemplate {
    layout: Layout,
    task: Task,
    children: Vec<Task>,
    children_done: usize,
    children_in_progress: usize,
    children_open: usize,
    children_total: usize,
    /// Pre-computed per-status counts for board view column headers.
    board_open_count: usize,
    board_in_progress_count: usize,
    board_blocked_count: usize,
    /// Done count visible on the board column (filtered by done_since); stats use children_done.
    board_done_count: usize,
    /// Current view mode: "list" (default) or "board".
    view: String,
    /// Currently selected done_since filter value (e.g. "3d", "7d", "30d", "all").
    done_since: String,
    /// Pre-built query string for HTMX polling (preserves view + done_since).
    poll_query: String,
    /// Pre-rendered HTML for the epic description (markdown → HTML, safe to output unescaped).
    description_html: Option<String>,
}

/// Query parameters for GET /epics/:id.
#[derive(Debug, Deserialize)]
pub struct EpicDetailQuery {
    pub view: Option<String>,
    /// How far back to show completed tasks on the board: "3d" (default), "7d", "30d", or "all".
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    pub done_since: Option<String>,
}

/// Template for the create task form at GET /tasks/new.
#[derive(Template)]
#[template(path = "task_new.html")]
struct TaskNewTemplate {
    layout: Layout,
    /// URL prefix of the current scope (form action and redirect target).
    prefix: String,
    /// Workspace to pre-select (and the only choice in workspace scope).
    default_workspace: Option<i64>,
    ws_groups: Vec<WsGroup>,
}

/// Template for the task creation modal fragment at GET /tasks/new/modal.
#[derive(Template)]
#[template(path = "task_create_modal.html")]
struct TaskCreateModalTemplate {
    /// Epics available for selection as parent task.
    epics: Vec<Task>,
    /// The workspace in workspace scope; new tasks go there (no selector shown).
    fixed_workspace: Option<WsInfo>,
    /// Workspace `<option>`s (non-missing only) grouped by project, with the default selected.
    ws_groups: Vec<WsGroup>,
}

/// Build a query string from current filter params for HTMX polling.
fn build_poll_query(
    status: &Option<String>,
    priority: &Option<String>,
    tag: &Option<String>,
    search: &Option<String>,
) -> String {
    let mut parts = Vec::new();
    if let Some(s) = status {
        parts.push(format!("status={s}"));
    }
    if let Some(p) = priority {
        parts.push(format!("priority={p}"));
    }
    if let Some(t) = tag {
        parts.push(format!("tag={t}"));
    }
    if let Some(q) = search {
        parts.push(format!("search={q}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// GET /tasks — Task list page with optional filter query params.
pub async fn task_list(
    State(state): State<AppState>,
    scope: Scope,
    Query(params): Query<ListTasksQuery>,
) -> Response {
    let has_filter = params.status.is_some()
        || params.priority.is_some()
        || params.tag.is_some()
        || params.search.as_deref().is_some_and(|s| !s.is_empty());
    let status_values = parse_status_values(&params.status);
    let priority_values = parse_priority_values(&params.priority);
    let show_all = has_filter || params.all.unwrap_or(false);
    let tag_param = params.tag.clone();
    let search_filter = params.search.clone();

    // Parse comma-separated tags for multi-tag OR filtering
    let selected_tags = parse_tags(tag_param.as_deref());
    let db_tag_filter = if selected_tags.len() == 1 {
        selected_tags.first().cloned()
    } else {
        None
    };
    let multi_tags = if selected_tags.len() > 1 {
        selected_tags.clone()
    } else {
        vec![]
    };

    let db = state.db.clone();
    let scope_filter = scope.filter;
    let row_scope = scope.clone();
    let (task_rows, all_tags) =
        tokio::task::spawn_blocking(move || -> Result<(Vec<TaskRow>, Vec<String>), String> {
            let db = db.lock().unwrap();
            // For single status/priority, pass directly to DB for efficiency.
            // For multi-value, load without that filter then post-filter in Rust.
            let (db_status, db_priority) = match (status_values.len(), priority_values.len()) {
                (0 | 1, 0 | 1) => (
                    status_values.first().map(|s| s.as_str()),
                    priority_values.first().copied(),
                ),
                _ => (None, None),
            };
            let mut tasks = db.list_tasks(
                show_all,
                db_status,
                db_priority,
                db_tag_filter.as_deref(),
                None,
                search_filter.as_deref(),
                None,
                &scope_filter,
            )?;
            // Post-filter for multi-value OR semantics
            if status_values.len() > 1 {
                let status_strs: Vec<&str> = status_values.iter().map(|s| s.as_str()).collect();
                tasks.retain(|t| {
                    let s = match t.status {
                        crate::models::Status::Open => "open",
                        crate::models::Status::InProgress => "in_progress",
                        crate::models::Status::Done => "done",
                        crate::models::Status::Blocked => "blocked",
                    };
                    status_strs.contains(&s)
                });
            }
            if priority_values.len() > 1 {
                tasks.retain(|t| priority_values.contains(&t.priority));
            }
            let tasks = filter_by_tags(tasks, &multi_tags);
            // Batch-load parent epics (avoids N+1: one lookup per unique parent_id)
            let parent_ids: std::collections::HashSet<String> =
                tasks.iter().filter_map(|t| t.parent_id.clone()).collect();
            let parents = fetch_parent_map(&db, parent_ids.into_iter())?;
            // Batch-fetch child counts so we can show subtask progress on parent tasks
            let task_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
            let child_counts = fetch_child_counts(&db, &task_ids)?;
            let blocker_counts = fetch_blocker_counts(&db, &task_ids)?;
            let pending = db.pending_user_comment_count_by_task(&scope_filter)?;
            let rows: Vec<TaskRow> = tasks
                .into_iter()
                .map(|t| {
                    TaskRow::from_task(t, &parents, &child_counts, &blocker_counts)
                        .with_workspace(&row_scope)
                        .with_pending(&pending)
                })
                .collect();
            // Fetch all tags for the dropdown
            let all_tags: Vec<String> = db
                .task_count_by_tag(&scope_filter)?
                .into_iter()
                .map(|(tag, _count)| tag)
                .collect();
            Ok((rows, all_tags))
        })
        .await
        .unwrap()
        .unwrap_or_else(|_| (vec![], vec![]));

    let poll_query = build_poll_query(
        &params.status,
        &params.priority,
        &params.tag,
        &params.search,
    );

    render_template(TaskListTemplate {
        layout: scope.layout("tasks"),
        tasks: task_rows,
        status_filter: params.status,
        priority_filter: params.priority,
        tag_filter: params.tag,
        selected_tags,
        all_tags,
        search_filter: params.search,
        has_filters: has_filter,
        poll_query,
    })
}

/// GET /tasks/new — Create task form.
pub async fn task_new(scope: Scope) -> Response {
    let default_workspace = scope.default_workspace_id();
    render_template(TaskNewTemplate {
        layout: scope.layout("tasks"),
        prefix: scope.prefix.clone(),
        default_workspace,
        ws_groups: scope.groups(default_workspace, false),
    })
}

/// GET /tasks/new/modal — Task creation form as an HTML fragment for HTMX modal loading.
///
/// Returns a standalone HTML fragment (no DOCTYPE, no `<html>` wrapper) containing
/// the create-task form.  The fragment is intended to be loaded into the `<dialog
/// id="task-modal">` element via HTMX.  The parent dropdown is populated with all
/// tasks that carry the "epic" tag.
pub async fn task_create_modal(State(state): State<AppState>, scope: Scope) -> Response {
    let db = state.db.clone();
    let scope_filter = scope.filter;
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<Task>, String> {
        let db = db.lock().unwrap();
        db.list_tasks(
            true,
            None,
            None,
            Some("epic"),
            None,
            None,
            None,
            &scope_filter,
        )
    })
    .await;

    let epics = match result {
        Ok(Ok(epics)) => epics,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("database error: {e}"),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawn error: {e}"),
            )
                .into_response();
        }
    };

    // Workspace scope: new tasks go to that workspace. Project scope: default to the first
    // non-missing workspace of the project. All scope: default to none (unscoped).
    let default_workspace = scope.default_workspace_id();
    let fixed_workspace = if scope.workspace_id.is_some() {
        scope.workspace().cloned()
    } else {
        None
    };
    render_template(TaskCreateModalTemplate {
        epics,
        fixed_workspace,
        ws_groups: scope.groups(default_workspace, false),
    })
}

/// Form body for POST /tasks (HTML form submission from task_new.html).
#[derive(Debug, Deserialize)]
pub struct CreateTaskFormBody {
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<u8>,
    /// Workspace id; empty string means none. Absent: the scope's workspace, if any.
    pub workspace_id: Option<String>,
}

/// POST /tasks — Handle HTML form submission from the create-task form.
///
/// Accepts `application/x-www-form-urlencoded` body, creates the task, then
/// issues a 303 redirect to `/tasks` so the user lands on the task list.
/// The JSON API at `POST /api/tasks` is unchanged and continues to return 201.
pub async fn task_create_form(
    State(state): State<AppState>,
    scope: Scope,
    Form(body): Form<CreateTaskFormBody>,
) -> Result<impl IntoResponse, AppError> {
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::Validation("title is required".to_string()));
    }
    let priority = body.priority.unwrap_or(2);
    let description = body
        .description
        .filter(|d| !d.trim().is_empty())
        .map(|d| d.trim().to_string());

    let workspace_id = match body.workspace_id.as_deref() {
        None => scope.workspace_id,
        Some("") => None,
        Some(s) => Some(
            crate::web::scope::parse_id(s)
                .ok_or_else(|| AppError::Validation("invalid workspace_id".to_string()))?,
        ),
    };

    let db = state.db.clone();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let db = db.lock().unwrap();
        if let Some(ws) = workspace_id
            && db.get_workspace(ws).map_err(AppError::Internal)?.is_none()
        {
            return Err(AppError::NotFound(format!("workspace not found: {ws}")));
        }
        let id = db.generate_id().map_err(AppError::Internal)?;
        let now = chrono::Utc::now();
        let task = Task {
            id,
            title,
            description,
            status: crate::models::Status::Open,
            priority,
            assignee: None,
            parent_id: None,
            tags: vec![],
            created_at: now,
            updated_at: now,
            close_reason: None,
            notes: None,
            workspace_id,
        };
        db.insert_task(&task).map_err(AppError::Internal)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    Ok(Redirect::to(&format!("{}/tasks", scope.prefix)))
}

/// All data needed to render a task detail view (full page or modal fragment).
struct TaskDetailData {
    task: Task,
    /// Parent epic, if this task is a subtask.
    parent: Option<Task>,
    blockers: Vec<Task>,
    dependents: Vec<Task>,
    comments: Vec<Comment>,
    /// Ids of user comments awaiting an agent reply.
    pending_ids: Vec<i64>,
}

/// GET /tasks/:id — Task detail page (200 or 404).
///
/// When called from HTMX (`HX-Request: true`), renders a modal fragment.
/// When called via direct browser navigation, renders the full page template.
pub async fn task_detail(
    State(state): State<AppState>,
    scope: Scope,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let is_htmx = headers.contains_key("HX-Request");
    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<TaskDetailData>, String> {
        let db = db.lock().unwrap();
        let task = match db.get_task(&id)? {
            Some(t) => t,
            None => return Ok(None),
        };
        // Fetch parent epic if this task is a subtask
        let parent = if let Some(ref pid) = task.parent_id {
            db.get_task(pid)?
        } else {
            None
        };
        // Resolve blocker dependency records to full Task objects
        let blocker_deps = db.get_blockers(&id)?;
        let mut blockers = Vec::with_capacity(blocker_deps.len());
        for dep in blocker_deps {
            if let Some(t) = db.get_task(&dep.parent_id)? {
                blockers.push(t);
            }
        }
        let dependents = db.get_dependents(&id)?;
        let comments = db.get_comments(&id)?;
        let pending_ids = db
            .task_pending_user_comments(&id)?
            .iter()
            .map(|c| c.id)
            .collect();
        Ok(Some(TaskDetailData {
            task,
            parent,
            blockers,
            dependents,
            comments,
            pending_ids,
        }))
    })
    .await
    .unwrap();

    match result {
        Ok(Some(data)) => {
            // Pre-render description and comment bodies from markdown to HTML.
            // The |safe filter in the template prevents double-escaping.
            let description_html = data.task.description.as_deref().map(render_markdown);
            let comment_views = build_comment_views(data.comments, &data.pending_ids);
            let ws = data.task.workspace_id.and_then(|id| scope.ws(id)).cloned();
            let ws_groups = scope.groups(data.task.workspace_id, true);
            if is_htmx {
                render_template(TaskDetailFragmentTemplate {
                    ws,
                    ws_groups,
                    task: data.task,
                    parent: data.parent,
                    blockers: data.blockers,
                    dependents: data.dependents,
                    comment_views,
                    description_html,
                })
            } else {
                render_template(TaskDetailTemplate {
                    layout: scope.layout("tasks"),
                    ws,
                    ws_groups,
                    task: data.task,
                    parent: data.parent,
                    blockers: data.blockers,
                    dependents: data.dependents,
                    comment_views,
                    description_html,
                })
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// Parse a `done_since` query value into an RFC 3339 cutoff timestamp, or `None` for "all".
///
/// Recognised values: `"3d"`, `"7d"`, `"30d"`, `"all"`.
/// Falls back to `"3d"` for unrecognised values; callers should resolve their view-specific
/// default before calling (e.g. global board → "3d", epic board → "all").
/// Returns `None` when the caller should not apply any date filter.
fn parse_done_since(done_since: &Option<String>) -> Option<chrono::DateTime<chrono::Utc>> {
    let value = done_since.as_deref().unwrap_or("3d");
    let days: i64 = match value {
        "all" => return None,
        "7d" => 7,
        "30d" => 30,
        _ => 3, // "3d" and any unrecognised value default to 3 days
    };
    Some(chrono::Utc::now() - chrono::Duration::days(days))
}

/// Normalise a `done_since` option to a canonical string value.
fn done_since_label(done_since: &Option<String>) -> String {
    match done_since.as_deref().unwrap_or("3d") {
        "7d" => "7d".to_string(),
        "30d" => "30d".to_string(),
        "all" => "all".to_string(),
        _ => "3d".to_string(),
    }
}

/// Build a poll query string for the board view (preserves epic + priority + done_since filters).
fn build_board_poll_query(
    epic: &Option<String>,
    priority: &Option<String>,
    done_since: &str,
) -> String {
    let mut parts = Vec::new();
    if let Some(e) = epic {
        parts.push(format!("epic={e}"));
    }
    if let Some(p) = priority {
        parts.push(format!("priority={p}"));
    }
    // Always include done_since so the poll URL is fully self-contained.
    parts.push(format!("done_since={done_since}"));
    format!("?{}", parts.join("&"))
}

/// Build a poll query string for the epic detail view (preserves view + done_since).
fn build_epic_detail_poll_query(view: &str, done_since: &str) -> String {
    format!("?view={view}&done_since={done_since}")
}

/// GET /board — Kanban board view grouped by status, with optional epic and priority filters.
pub async fn board(
    State(state): State<AppState>,
    scope: Scope,
    Query(query): Query<BoardQuery>,
) -> Response {
    let scope_filter = scope.filter;
    let layout = scope.layout("board");
    let epic_filter = query.epic.clone();
    let priority_filter = query.priority.clone();
    let done_since_param = query.done_since.clone();

    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<BoardTemplate, String> {
        let db = db.lock().unwrap();

        // Fetch all epics for the dropdown.
        let epics = db.list_tasks(
            true,
            None,
            None,
            Some("epic"),
            None,
            None,
            None,
            &scope_filter,
        )?;

        // Parse multi-select values.
        let epic_values = parse_status_values(&epic_filter); // epic IDs are strings
        let priority_values = parse_priority_values(&priority_filter);

        // Helper: fetch tasks for a given status, applying epic and priority filters.
        // For single values, pass directly to DB for efficiency; for multi-values, post-filter.
        let fetch = |status: &str, show_done: bool| -> Result<Vec<Task>, String> {
            let (db_parent, db_priority) = match (epic_values.len(), priority_values.len()) {
                (1, 1) => (
                    epic_values.first().map(|s| s.as_str()),
                    priority_values.first().copied(),
                ),
                (1, _) => (epic_values.first().map(|s| s.as_str()), None),
                (_, 1) => (None, priority_values.first().copied()),
                _ => (None, None),
            };
            db.list_tasks(
                show_done,
                Some(status),
                db_priority,
                None,
                db_parent,
                None,
                None,
                &scope_filter,
            )
        };

        // Fetch the set of task IDs that have at least one open blocker (via dep graph).
        // These tasks belong in the Blocked column regardless of their `status` field,
        // because `dep add` does not automatically change a task's status to "blocked".
        let dep_blocked_ids: std::collections::HashSet<String> = db
            .get_blocked_tasks(&scope_filter)?
            .into_iter()
            .map(|t| t.id)
            .collect();

        // Fetch open tasks, then split: those with open blockers go to the blocked column.
        let open_raw = fetch("open", false)?;
        let (dep_blocked_open, open_raw_filtered): (Vec<Task>, Vec<Task>) = open_raw
            .into_iter()
            .partition(|t| dep_blocked_ids.contains(&t.id));

        let in_progress_raw = fetch("in_progress", false)?;
        // Combine status=blocked tasks with open tasks that have active dep blockers.
        let mut blocked_raw = fetch("blocked", false)?;
        blocked_raw.extend(dep_blocked_open);
        let done_raw = fetch("done", true)?;

        // Compute the done_since cutoff for filtering completed tasks.
        let done_cutoff = parse_done_since(&done_since_param);

        // Post-filter for multi-value epic or priority selections, plus done_since for done column.
        let post_filter = |mut tasks: Vec<Task>, apply_done_since: bool| -> Vec<Task> {
            if epic_values.len() > 1 {
                tasks.retain(|t| {
                    t.parent_id
                        .as_deref()
                        .is_some_and(|pid| epic_values.contains(&pid.to_string()))
                });
            }
            if priority_values.len() > 1 {
                tasks.retain(|t| priority_values.contains(&t.priority));
            }
            if apply_done_since && let Some(cutoff) = done_cutoff {
                tasks.retain(|t| t.updated_at >= cutoff);
            }
            tasks
        };

        let open_raw_filtered = post_filter(open_raw_filtered, false);
        let in_progress_raw = post_filter(in_progress_raw, false);
        let blocked_raw = post_filter(blocked_raw, false);
        let done_raw = post_filter(done_raw, true);

        // Batch-load all unique parent epics across all columns
        let all_tasks_iter = open_raw_filtered
            .iter()
            .chain(in_progress_raw.iter())
            .chain(blocked_raw.iter())
            .chain(done_raw.iter());
        let parent_ids: std::collections::HashSet<String> =
            all_tasks_iter.filter_map(|t| t.parent_id.clone()).collect();
        let parents = fetch_parent_map(&db, parent_ids.into_iter())?;

        // Board cards don't show child counts — pass empty map so from_task defaults to (0, 0)
        let empty_child_counts = std::collections::HashMap::new();
        // Batch-fetch blocker counts for all tasks across all columns
        let all_board_ids: Vec<String> = open_raw_filtered
            .iter()
            .chain(in_progress_raw.iter())
            .chain(blocked_raw.iter())
            .chain(done_raw.iter())
            .map(|t| t.id.clone())
            .collect();
        let blocker_counts = fetch_blocker_counts(&db, &all_board_ids)?;
        let pending = db.pending_user_comment_count_by_task(&scope_filter)?;
        let to_rows = |tasks: Vec<Task>| -> Vec<TaskRow> {
            tasks
                .into_iter()
                .map(|t| {
                    TaskRow::from_task(t, &parents, &empty_child_counts, &blocker_counts)
                        .with_workspace(&scope)
                        .with_pending(&pending)
                })
                .collect()
        };

        let open_tasks = to_rows(open_raw_filtered);
        let in_progress_tasks = to_rows(in_progress_raw);
        let blocked_tasks = to_rows(blocked_raw);
        let done_tasks = to_rows(done_raw);

        let selected_epic = epic_filter.clone().unwrap_or_default();
        let selected_priority = priority_filter.clone().unwrap_or_default();
        let done_since_label = done_since_label(&done_since_param);
        let poll_query = build_board_poll_query(&epic_filter, &priority_filter, &done_since_label);

        Ok(BoardTemplate {
            layout,
            open_tasks,
            in_progress_tasks,
            blocked_tasks,
            done_tasks,
            epics,
            selected_epic,
            selected_priority,
            done_since: done_since_label,
            poll_query,
        })
    })
    .await
    .unwrap();

    match result {
        Ok(tmpl) => render_template(tmpl),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// GET /epics — Epics overview with subtask progress, optional status/priority filters.
pub async fn epics(
    State(state): State<AppState>,
    scope: Scope,
    Query(query): Query<EpicsQuery>,
) -> Response {
    let scope_filter = scope.filter;
    let layout = scope.layout("epics");
    let status_filter = query.status.clone();
    let priority_filter = query.priority.clone();

    let status_values = parse_status_values(&status_filter);
    let priority_values = parse_priority_values(&priority_filter);

    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<EpicRow>, String> {
        let db = db.lock().unwrap();
        let epic_tasks = db.list_tasks(
            true,
            None,
            None,
            Some("epic"),
            None,
            None,
            None,
            &scope_filter,
        )?;
        let mut rows = Vec::with_capacity(epic_tasks.len());
        for task in epic_tasks {
            let children = db.get_children(&task.id)?;
            let children_total = children.len();
            let children_done = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::Done))
                .count();
            let children_in_progress = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::InProgress))
                .count();
            let children_open = children_total - children_done - children_in_progress;
            let workspace = if scope.show_workspace_badge() {
                task.workspace_id.and_then(|id| scope.ws(id)).cloned()
            } else {
                None
            };
            rows.push(EpicRow {
                workspace,
                task,
                children_total,
                children_done,
                children_in_progress,
                children_open,
            });
        }

        // Post-filter by status (multi-select OR)
        if !status_values.is_empty() {
            rows.retain(|row| {
                let s = match row.task.status {
                    crate::models::Status::Open => "open",
                    crate::models::Status::InProgress => "in_progress",
                    crate::models::Status::Done => "done",
                    crate::models::Status::Blocked => "blocked",
                };
                status_values.iter().any(|v| v == s)
            });
        }

        // Post-filter by priority (multi-select OR)
        if !priority_values.is_empty() {
            rows.retain(|row| priority_values.contains(&row.task.priority));
        }

        // Sort: in_progress first, then open, then done; within each group, priority ascending.
        rows.sort_by(|a, b| {
            let status_order = |s: &crate::models::Status| match s {
                crate::models::Status::InProgress => 0,
                crate::models::Status::Open => 1,
                crate::models::Status::Blocked => 2,
                crate::models::Status::Done => 3,
            };
            let sa = status_order(&a.task.status);
            let sb = status_order(&b.task.status);
            sa.cmp(&sb).then(a.task.priority.cmp(&b.task.priority))
        });

        Ok(rows)
    })
    .await
    .unwrap();

    // Build poll query preserving current filters.
    let poll_query = {
        let mut parts = Vec::new();
        if let Some(s) = &status_filter {
            parts.push(format!("status={s}"));
        }
        if let Some(p) = &priority_filter {
            parts.push(format!("priority={p}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("?{}", parts.join("&"))
        }
    };

    match result {
        Ok(epics) => render_template(EpicsTemplate {
            layout,
            epics,
            status_filter,
            priority_filter,
            poll_query,
        }),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// GET /epics/:id — Epic detail page with children task list (200 or 404).
pub async fn epic_detail(
    State(state): State<AppState>,
    scope: Scope,
    Query(query): Query<EpicDetailQuery>,
) -> Response {
    let Some(id) = scope.param("id").map(str::to_string) else {
        return (StatusCode::NOT_FOUND, "epic not found").into_response();
    };
    let layout = scope.layout("epics");
    // Normalise view param: accept "board", default to "list" for anything else.
    let view = match query.view.as_deref() {
        Some("board") => "board".to_string(),
        _ => "list".to_string(),
    };
    let view_clone = view.clone();
    // Epic board defaults to "all" (show all completed tasks); global board defaults to "7d".
    let done_since_param = Some(
        query
            .done_since
            .clone()
            .unwrap_or_else(|| "all".to_string()),
    );

    let db = state.db.clone();
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<EpicDetailTemplate>, String> {
            let db = db.lock().unwrap();
            let task = match db.get_task(&id)? {
                Some(t) => t,
                None => return Ok(None),
            };
            let mut children = db.get_children(&id)?;
            // Sort children by the numeric suffix of their hierarchical ID (e.g. `tk-xxxx.N`)
            // so that ordering is 1, 2, 3 … 10, 11 rather than lexicographic 1, 10, 11 … 2.
            children.sort_by_key(|c| {
                c.id.rfind('.')
                    .and_then(|pos| c.id[pos + 1..].parse::<u64>().ok())
                    .unwrap_or(0)
            });
            let children_total = children.len();
            // Stats counts reflect ALL children regardless of done_since.
            let children_done = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::Done))
                .count();
            let board_open_count = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::Open))
                .count();
            let board_in_progress_count = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::InProgress))
                .count();
            let board_blocked_count = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::Blocked))
                .count();
            let children_in_progress = board_in_progress_count;
            let children_open = children_total - children_done - children_in_progress;

            // Board done column: post-filter by done_since cutoff.
            let done_cutoff = parse_done_since(&done_since_param);
            let board_done_count = children
                .iter()
                .filter(|c| matches!(c.status, crate::models::Status::Done))
                .filter(|c| done_cutoff.is_none_or(|cutoff| c.updated_at >= cutoff))
                .count();

            // Pre-filter children for the board done column (used by template iteration).
            // The template iterates `children` for the done column and checks status; we store
            // the cutoff in the template so it can apply it, or we pass a pre-filtered list.
            // Since askama templates cannot call Rust functions directly, we apply the filter
            // here by retaining done children that pass the cutoff and rebuilding children
            // with non-done items unaffected. The template iterates `children` for all columns.
            // Strategy: replace done children that are outside the window with a sentinel
            // is complex; instead we store a separate `board_done_children` would require a
            // new template field. For simplicity, keep `children` unfiltered (list view needs
            // all), and set `board_done_count` to the filtered count. The template for the
            // board done column already iterates `children` filtered by status==Done —
            // we need it to also apply the cutoff. The cleanest approach is to filter `children`
            // so that done tasks outside the window are excluded, knowing the list view renders
            // all statuses and the board open/in-progress/blocked are unaffected.
            // We accomplish this by retaining all non-done children plus done children within
            // the window. This keeps list view correct (it shows all statuses independently)
            // while the board done column sees only the filtered set.
            if done_cutoff.is_some() {
                children.retain(|c| {
                    !matches!(c.status, crate::models::Status::Done)
                        || done_cutoff.is_none_or(|cutoff| c.updated_at >= cutoff)
                });
            }

            // Pre-render description from markdown to HTML.
            // The |safe filter in the template prevents double-escaping.
            let description_html = task.description.as_deref().map(render_markdown);
            let done_since_str = done_since_label(&done_since_param);
            let poll_query = build_epic_detail_poll_query(&view_clone, &done_since_str);
            Ok(Some(EpicDetailTemplate {
                layout,
                task,
                children,
                children_done,
                children_in_progress,
                children_open,
                children_total,
                board_open_count,
                board_in_progress_count,
                board_blocked_count,
                board_done_count,
                view: view_clone,
                done_since: done_since_str,
                poll_query,
                description_html,
            }))
        })
        .await
        .unwrap();

    match result {
        Ok(Some(tmpl)) => render_template(tmpl),
        Ok(None) => (StatusCode::NOT_FOUND, "epic not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// GET /api/tags — Unique tag names sorted by usage count descending (200).
pub async fn api_tags(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let tags: Vec<String> = tokio::task::spawn_blocking(move || {
        let db = db.lock().unwrap();
        db.task_count_by_tag(&scope)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?
    .into_iter()
    .map(|(tag, _count)| tag)
    .collect();

    Ok(Json(tags))
}

// ---------------------------------------------------------------------------
// Dependency tree partial
// ---------------------------------------------------------------------------

/// A single node in the dependency tree, with pre-computed depth for CSS indentation.
#[allow(dead_code)]
struct DepTreeNode {
    task: Task,
    /// BFS depth from the root task (1 = direct dep, 2 = transitive, etc.).
    depth: usize,
}

/// Template for the GET /tasks/:id/dep-tree partial.
#[derive(Template)]
#[template(path = "partials/dep_tree.html")]
#[allow(dead_code)]
struct DepTreeTemplate {
    /// Nodes in the "blocked by" (upstream) direction.
    up_nodes: Vec<DepTreeNode>,
    /// Nodes in the "blocks" (downstream) direction.
    down_nodes: Vec<DepTreeNode>,
    /// Direction query param value ("up", "down", or "both").
    dir: String,
}

/// Query parameters for GET /tasks/:id/dep-tree.
#[derive(Debug, Deserialize)]
pub struct DepTreeQuery {
    /// Direction: "up" (blockers), "down" (dependents), or "both" (default).
    pub dir: Option<String>,
}

/// BFS traversal: collect all transitive dependencies in one direction.
///
/// `direction` is either "up" (follow blockers) or "down" (follow dependents).
/// Returns nodes ordered by BFS level (depth 1 first), then by task priority.
fn bfs_deps(
    db: &crate::db::Database,
    root_id: &str,
    direction: &str,
) -> Result<Vec<DepTreeNode>, String> {
    use std::collections::{HashSet, VecDeque};

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(root_id.to_string());

    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut nodes: Vec<DepTreeNode> = Vec::new();

    // Seed queue with direct deps at depth 1.
    let direct: Vec<Task> = if direction == "up" {
        let deps = db.get_blockers(root_id)?;
        let mut tasks = Vec::with_capacity(deps.len());
        for dep in deps {
            if let Some(t) = db.get_task(&dep.parent_id)? {
                tasks.push(t);
            }
        }
        tasks
    } else {
        db.get_dependents(root_id)?
    };

    for task in direct {
        if visited.insert(task.id.clone()) {
            queue.push_back((task.id.clone(), 1));
            nodes.push(DepTreeNode { task, depth: 1 });
        }
    }

    // BFS: expand each node's children.
    while let Some((id, depth)) = queue.pop_front() {
        let next_depth = depth + 1;
        let nexts: Vec<Task> = if direction == "up" {
            let deps = db.get_blockers(&id)?;
            let mut tasks = Vec::with_capacity(deps.len());
            for dep in deps {
                if let Some(t) = db.get_task(&dep.parent_id)? {
                    tasks.push(t);
                }
            }
            tasks
        } else {
            db.get_dependents(&id)?
        };

        for task in nexts {
            if visited.insert(task.id.clone()) {
                queue.push_back((task.id.clone(), next_depth));
                nodes.push(DepTreeNode {
                    task,
                    depth: next_depth,
                });
            }
        }
    }

    Ok(nodes)
}

/// GET /tasks/:id/dep-tree — Dependency tree HTML partial.
///
/// Returns a nested HTML partial showing a task's dependency graph.
/// The `?dir` query parameter controls direction:
/// - `up`: show tasks that block this task (blockers)
/// - `down`: show tasks that this task blocks (dependents)
/// - `both` (default): show both directions with section headers
pub async fn task_dep_tree(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DepTreeQuery>,
) -> Response {
    let dir = query
        .dir
        .as_deref()
        .and_then(|d| match d {
            "up" | "down" | "both" => Some(d),
            _ => None,
        })
        .unwrap_or("both")
        .to_string();
    let dir_clone = dir.clone();

    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<DepTreeTemplate>, String> {
        let db = db.lock().unwrap();

        // Verify task exists.
        if db.get_task(&id)?.is_none() {
            return Ok(None);
        }

        let up_nodes = if dir_clone == "up" || dir_clone == "both" {
            bfs_deps(&db, &id, "up")?
        } else {
            vec![]
        };

        let down_nodes = if dir_clone == "down" || dir_clone == "both" {
            bfs_deps(&db, &id, "down")?
        } else {
            vec![]
        };

        Ok(Some(DepTreeTemplate {
            up_nodes,
            down_nodes,
            dir: dir_clone,
        }))
    })
    .await
    .unwrap();

    match result {
        Ok(Some(tmpl)) => render_template(tmpl),
        Ok(None) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// A task and the subset of its blockers that are also children of the same epic.
#[allow(dead_code)]
struct EpicDepItem {
    task: Task,
    blockers: Vec<Task>,
}

/// Template for the epic dependency tree partial at GET /epics/:id/dep-tree.
#[derive(Template)]
#[template(path = "partials/epic_dep_tree.html")]
#[allow(dead_code)]
struct EpicDepTreeTemplate {
    items: Vec<EpicDepItem>,
}

/// GET /epics/:id/dep-tree — Intra-epic dependency list (lazy-loaded HTML partial).
///
/// Shows which children of the epic have dependencies on other children of the same epic.
/// Only intra-epic dependencies are shown; cross-epic or external deps are excluded.
pub async fn epic_dep_tree(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<EpicDepItem>, String> {
        let db = db.lock().unwrap();
        let children = db.get_children(&id)?;
        let child_ids: std::collections::HashSet<String> =
            children.iter().map(|c| c.id.clone()).collect();

        let mut items = Vec::new();
        for child in &children {
            let all_blockers_deps = db.get_blockers(&child.id)?;
            let mut intra_blockers = Vec::new();
            for dep in all_blockers_deps {
                if child_ids.contains(&dep.parent_id)
                    && let Some(t) = db.get_task(&dep.parent_id)?
                {
                    intra_blockers.push(t);
                }
            }
            if !intra_blockers.is_empty() {
                items.push(EpicDepItem {
                    task: child.clone(),
                    blockers: intra_blockers,
                });
            }
        }
        Ok(items)
    })
    .await
    .unwrap();

    match result {
        Ok(items) => render_template(EpicDepTreeTemplate { items }),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database error: {e}"),
        )
            .into_response(),
    }
}

/// GET /api/stats — Task statistics (200).
pub async fn api_stats(
    State(state): State<AppState>,
    ApiScope(scope): ApiScope,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<StatsResponse, String> {
        let db = db.lock().unwrap();

        let by_status_vec = db.task_count_by_status(&scope)?;
        let by_priority_vec = db.task_count_by_priority(&scope)?;
        let by_tag_vec = db.task_count_by_tag(&scope)?;

        let by_status: Map<String, Value> = by_status_vec
            .into_iter()
            .map(|(k, v)| (k, Value::Number(v.into())))
            .collect();

        let by_priority: Map<String, Value> = by_priority_vec
            .into_iter()
            .map(|(k, v)| (k.to_string(), Value::Number(v.into())))
            .collect();

        let by_tag: Map<String, Value> = by_tag_vec
            .into_iter()
            .map(|(k, v)| (k, Value::Number(v.into())))
            .collect();

        Ok(StatsResponse {
            by_status,
            by_priority,
            by_tag,
        })
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(AppError::Internal)?;

    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::{CreateTaskBody, UpdateTaskBody};

    #[test]
    fn test_update_body_workspace_id_distinguishes_absent_null_and_value() {
        let absent: UpdateTaskBody = serde_json::from_str(r#"{"title":"x"}"#).unwrap();
        assert_eq!(absent.workspace_id, None);
        let null: UpdateTaskBody = serde_json::from_str(r#"{"workspace_id":null}"#).unwrap();
        assert_eq!(null.workspace_id, Some(None));
        let value: UpdateTaskBody = serde_json::from_str(r#"{"workspace_id":4}"#).unwrap();
        assert_eq!(value.workspace_id, Some(Some(4)));
        assert!(serde_json::from_str::<UpdateTaskBody>(r#"{"workspace_id":"x"}"#).is_err());
    }

    #[test]
    fn test_create_body_workspace_id_is_optional() {
        let none: CreateTaskBody = serde_json::from_str(r#"{"title":"x"}"#).unwrap();
        assert_eq!(none.workspace_id, None);
        let some: CreateTaskBody =
            serde_json::from_str(r#"{"title":"x","workspace_id":2}"#).unwrap();
        assert_eq!(some.workspace_id, Some(2));
    }
}
