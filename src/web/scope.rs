//! Web scope handling: the `Scope` extractor for HTML routes, the `ApiScope`
//! extractor for `?project=`/`?workspace=` on JSON endpoints, and the sidebar data.
//!
//! Scope lives in the URL path for HTML views (`/p/{project}/...`,
//! `/p/{project}/w/{workspace}/...`) and in query parameters for the JSON API.
//! IDs are strictly numeric. Workspace and project paths from the database are only
//! displayed and checked with `exists()`; they are never read or served.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;

use axum::extract::{FromRequestParts, MatchedPath, Query, RawPathParams};
use axum::http::request::Parts;
use serde::Deserialize;

use crate::db::{Database, ScopeFilter};
use crate::web::AppState;
use crate::web::errors::AppError;

/// A workspace together with its project name and on-disk status.
#[derive(Debug, Clone)]
pub struct WsInfo {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub project_id: i64,
    pub project_name: String,
    /// True when `path` no longer exists on disk.
    pub missing: bool,
    /// True when the workspace was removed (archived) in the web UI.
    pub archived: bool,
}

/// One `<option>` of a workspace `<select>`.
#[derive(Debug, Clone)]
pub struct WsOption {
    pub id: i64,
    pub name: String,
    pub missing: bool,
    /// Archived workspaces are only listed when they are the task's current workspace.
    pub archived: bool,
    pub selected: bool,
    /// Missing workspaces cannot be chosen as a target unless already selected.
    pub disabled: bool,
}

/// A project id with its display name.
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub id: i64,
    pub name: String,
}

/// One project in a grouped `<select>`: a "Backlog" option (value `backlog:{project_id}`)
/// followed by the project's workspaces.
#[derive(Debug, Clone)]
pub struct WsGroup {
    pub project_id: i64,
    pub project_name: String,
    /// True when the task (or the default) is in this project's backlog.
    pub backlog_selected: bool,
    pub options: Vec<WsOption>,
}

/// One workspace line in the sidebar.
#[derive(Debug, Clone)]
pub struct SidebarWorkspace {
    pub id: i64,
    pub name: String,
    pub path: String,
    /// Tasks that are not done.
    pub open: i64,
    pub missing: bool,
    pub active: bool,
}

/// One project in the sidebar, with its workspaces nested.
#[derive(Debug, Clone)]
pub struct SidebarProject {
    pub id: i64,
    pub name: String,
    /// Tasks not done in the whole project: its workspaces plus its backlog.
    pub open: i64,
    pub active: bool,
    /// Tasks not done in the project's backlog.
    pub backlog_open: i64,
    /// The project's backlog is the active scope.
    pub backlog_active: bool,
    pub workspaces: Vec<SidebarWorkspace>,
}

/// Sidebar contents: the "All" entry, every project with its visible workspaces and the
/// number of archived workspaces (listed on the `/archived` page).
#[derive(Debug, Clone)]
pub struct Sidebar {
    pub all_active: bool,
    pub all_open: i64,
    pub projects: Vec<SidebarProject>,
    /// Archived workspaces of every project; the sidebar links to `/archived` when > 0.
    pub archived_count: usize,
}

/// The scope header shown above the content: `project / workspace` or "All projects".
#[derive(Debug, Clone)]
pub struct ScopeHeader {
    /// Project name; `None` in the All scope.
    pub project_name: Option<String>,
    /// URL prefix of the project scope (`/p/1`), when a project is selected.
    pub project_prefix: Option<String>,
    /// The selected workspace (workspace scope only).
    pub workspace: Option<WsInfo>,
    /// The project's backlog is selected (backlog scope only).
    pub backlog: bool,
}

/// What base.html needs: URL prefix for the current scope, the current view
/// (`tasks`, `board`, `epics`), the sidebar and the scope header (both absent for HTMX
/// requests).
#[derive(Debug, Clone)]
pub struct Layout {
    /// `""`, `"/p/1"`, `"/p/1/backlog"` or `"/p/1/w/2"`. Prepend to `/board`, `/tasks`, `/epics`.
    pub prefix: String,
    pub view: &'static str,
    pub sidebar: Option<Arc<Sidebar>>,
    pub header: Option<Arc<ScopeHeader>>,
}

/// The scope of an HTML request, resolved from the URL path.
#[derive(Debug, Clone)]
pub struct Scope {
    pub filter: ScopeFilter,
    /// `""`, `"/p/{project}"`, `"/p/{project}/backlog"` or `"/p/{project}/w/{workspace}"`.
    pub prefix: String,
    pub project_id: Option<i64>,
    pub workspace_id: Option<i64>,
    /// The request is in a project's backlog scope (`/p/{project}/backlog/...`).
    pub backlog: bool,
    /// The request came from HTMX (`HX-Request`), so no sidebar is rendered.
    pub is_htmx: bool,
    /// Every workspace (archived ones included), ordered by project name then workspace
    /// name. Use [`Scope::groups`] / [`Scope::default_workspace_id`] for selectable ones.
    pub workspaces: Arc<Vec<WsInfo>>,
    /// Every project, ordered by name then id.
    pub projects: Arc<Vec<ProjectInfo>>,
    by_id: Arc<HashMap<i64, WsInfo>>,
    sidebar: Option<Arc<Sidebar>>,
    header: Option<Arc<ScopeHeader>>,
    /// Raw path parameters other than the scope ones (e.g. `id` of `/epics/{id}`).
    params: Arc<HashMap<String, String>>,
}

impl Scope {
    /// Layout data for a full-page render of `view`.
    pub fn layout(&self, view: &'static str) -> Layout {
        Layout {
            prefix: self.prefix.clone(),
            view,
            sidebar: self.sidebar.clone(),
            header: self.header.clone(),
        }
    }

    /// The workspace of a scoped request (workspace scope only).
    pub fn workspace(&self) -> Option<&WsInfo> {
        self.workspace_id.and_then(|id| self.by_id.get(&id))
    }

    /// Look up a workspace by id.
    pub fn ws(&self, id: i64) -> Option<&WsInfo> {
        self.by_id.get(&id)
    }

    /// True when views should show a workspace (or "backlog") badge on each task: project
    /// and All scopes. Workspace and backlog scopes would only repeat the scope.
    pub fn show_workspace_badge(&self) -> bool {
        self.workspace_id.is_none() && !self.backlog
    }

    /// Name of a project by id.
    pub fn project_name(&self, id: i64) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }

    /// The project whose backlog holds `task`: set when the task has no workspace but a
    /// project.
    pub fn backlog_project_of(&self, task: &crate::models::Task) -> Option<(i64, String)> {
        if task.workspace_id.is_some() {
            return None;
        }
        let id = task.project_id?;
        Some((id, self.project_name(id).unwrap_or_default().to_string()))
    }

    /// A non-scope path parameter (e.g. `id`).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(String::as_str)
    }

    /// Every project as `<optgroup>` data: a "Backlog" option plus the project's
    /// workspaces, with `selected` / `backlog_selected` marked. Missing workspaces are left
    /// out when `include_missing` is false; when included they are disabled unless
    /// currently selected. Archived workspaces only appear when selected.
    pub fn groups(
        &self,
        selected: Option<i64>,
        backlog_project: Option<i64>,
        include_missing: bool,
    ) -> Vec<WsGroup> {
        self.projects
            .iter()
            .map(|p| {
                let options = self
                    .workspaces
                    .iter()
                    .filter(|w| w.project_id == p.id)
                    .filter_map(|w| {
                        let is_selected = selected == Some(w.id);
                        if (w.missing && !include_missing || w.archived) && !is_selected {
                            return None;
                        }
                        Some(WsOption {
                            id: w.id,
                            name: w.name.clone(),
                            missing: w.missing,
                            archived: w.archived,
                            selected: is_selected,
                            disabled: (w.missing || w.archived) && !is_selected,
                        })
                    })
                    .collect();
                WsGroup {
                    project_id: p.id,
                    project_name: p.name.clone(),
                    backlog_selected: backlog_project == Some(p.id),
                    options,
                }
            })
            .collect()
    }

    /// Default placement for new tasks as `(workspace, backlog project)`: the workspace in
    /// workspace scope, the project's backlog in project and backlog scopes, unscoped
    /// (both `None`) in All scope.
    pub fn default_placement(&self) -> (Option<i64>, Option<i64>) {
        if let Some(w) = self.workspace_id {
            return (Some(w), None);
        }
        (None, self.project_id)
    }
}

/// Parse a strictly numeric id (ASCII digits only, fits in i64).
pub fn parse_id(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

/// Load every workspace with its project name and missing flag.
///
/// `check_missing` gates the per-workspace `Path::exists()` filesystem check: when false,
/// `missing` is left `false` (the caller renders nothing that depends on it).
fn load_workspaces(
    db: &Database,
    check_missing: bool,
) -> Result<(Vec<ProjectInfo>, Vec<WsInfo>), String> {
    let projects: Vec<ProjectInfo> = db
        .list_projects()?
        .into_iter()
        .map(|p| ProjectInfo {
            id: p.id,
            name: p.name,
        })
        .collect();
    let names: HashMap<i64, &str> = projects.iter().map(|p| (p.id, p.name.as_str())).collect();
    let workspaces = db
        .list_workspaces()?
        .into_iter()
        .map(|w| WsInfo {
            id: w.id,
            project_name: names
                .get(&w.project_id)
                .copied()
                .unwrap_or_default()
                .to_string(),
            project_id: w.project_id,
            missing: check_missing && !FsPath::new(&w.path).exists(),
            archived: w.archived_at.is_some(),
            name: w.name,
            path: w.path,
        })
        .collect();
    Ok((projects, workspaces))
}

/// Build the sidebar tree with non-done task counts and the active scope highlighted.
/// Archived workspaces are left out of the tree and of every count; only their number is
/// kept (the `/archived` page lists them). Projects whose workspaces are all archived are
/// hidden unless active or they have backlog tasks. Every shown project gets a "Backlog" entry, also with a
/// count of 0: the entry is the way to reach an empty backlog and create tasks in it,
/// and a stable layout beats hiding it.
fn build_sidebar(
    db: &Database,
    projects: &[ProjectInfo],
    workspaces: &[WsInfo],
    project_id: Option<i64>,
    workspace_id: Option<i64>,
    backlog: bool,
) -> Result<Sidebar, String> {
    let counts: HashMap<Option<i64>, i64> = db
        .workspace_task_counts()?
        .into_iter()
        .map(|c| (c.workspace_id, c.open + c.in_progress + c.blocked))
        .collect();
    let backlog_counts: HashMap<i64, i64> = db
        .backlog_task_counts()?
        .into_iter()
        .map(|c| (c.project_id, c.open + c.in_progress + c.blocked))
        .collect();
    let open_of = |id: i64| counts.get(&Some(id)).copied().unwrap_or(0);
    let archived_ids: std::collections::HashSet<i64> = workspaces
        .iter()
        .filter(|w| w.archived)
        .map(|w| w.id)
        .collect();
    let all_open: i64 = counts
        .iter()
        .filter(|(k, _)| k.is_none_or(|id| !archived_ids.contains(&id)))
        .map(|(_, v)| v)
        .sum::<i64>()
        + backlog_counts.values().sum::<i64>();
    let mut sidebar_projects: Vec<SidebarProject> = Vec::new();
    for p in projects {
        let backlog_open = backlog_counts.get(&p.id).copied().unwrap_or(0);
        let in_project = project_id == Some(p.id);
        let mut sp = SidebarProject {
            id: p.id,
            name: p.name.clone(),
            open: backlog_open,
            active: in_project && workspace_id.is_none() && !backlog,
            backlog_open,
            backlog_active: in_project && backlog,
            workspaces: Vec::new(),
        };
        let mut has_archived = false;
        for w in workspaces.iter().filter(|w| w.project_id == p.id) {
            if w.archived {
                has_archived = true;
                continue;
            }
            let open = open_of(w.id);
            sp.open += open;
            sp.workspaces.push(SidebarWorkspace {
                id: w.id,
                name: w.name.clone(),
                path: w.path.clone(),
                open,
                missing: w.missing,
                active: workspace_id == Some(w.id),
            });
        }
        if sp.workspaces.is_empty()
            && has_archived
            && !in_project
            && !backlog_counts.contains_key(&p.id)
        {
            continue;
        }
        sidebar_projects.push(sp);
    }
    Ok(Sidebar {
        all_active: project_id.is_none() && workspace_id.is_none(),
        all_open,
        projects: sidebar_projects,
        archived_count: archived_ids.len(),
    })
}

impl FromRequestParts<AppState> for Scope {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let is_htmx = parts.headers.contains_key("HX-Request");
        let mut params: HashMap<String, String> = HashMap::new();
        if let Ok(raw) = RawPathParams::from_request_parts(parts, state).await {
            for (k, v) in raw.iter() {
                params.insert(k.to_string(), v.to_string());
            }
        }
        // Backlog routes (`/p/{project_id}/backlog/...`) share the handlers of the other
        // prefixes; the matched route pattern tells them apart.
        let backlog_req = parts
            .extensions
            .get::<MatchedPath>()
            .is_some_and(|m| m.as_str().starts_with("/p/{project_id}/backlog/"));
        let project_raw = params.remove("project_id");
        let workspace_raw = params.remove("workspace_id");
        let not_found = |what: &str| AppError::NotFound(format!("{what} not found"));
        let project_req = match project_raw {
            Some(s) => Some(parse_id(&s).ok_or_else(|| not_found("project"))?),
            None => None,
        };
        let workspace_req = match workspace_raw {
            Some(s) => Some(parse_id(&s).ok_or_else(|| not_found("workspace"))?),
            None => None,
        };

        let db = state.db.clone();
        let (projects, workspaces, sidebar, project_name) =
            tokio::task::spawn_blocking(move || {
                let db = db.lock().unwrap();
                // `missing` is only rendered by the sidebar (full pages) and by workspace
                // badges (project/All scopes). HTMX polls in workspace scope render neither,
                // so skip the filesystem checks there.
                let check_missing = !is_htmx || workspace_req.is_none();
                if check_missing {
                    db.auto_archive_removed_workspaces(|p| !FsPath::new(p).exists())?;
                }
                let (projects, workspaces) = load_workspaces(&db, check_missing)?;
                let project_name = match project_req {
                    Some(id) => db.get_project(id)?.map(|p| Some(p.name)),
                    None => Some(None),
                };
                let project_exists = project_name.is_some();
                let sidebar = if is_htmx || !project_exists {
                    None
                } else {
                    Some(Arc::new(build_sidebar(
                        &db,
                        &projects,
                        &workspaces,
                        project_req,
                        workspace_req,
                        backlog_req,
                    )?))
                };
                Ok::<_, String>((projects, workspaces, sidebar, project_name))
            })
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .map_err(AppError::Internal)?;

        let by_id: HashMap<i64, WsInfo> = workspaces.iter().map(|w| (w.id, w.clone())).collect();
        let Some(project_name) = project_name else {
            return Err(not_found("project"));
        };
        // Archived workspaces are not addressable as a scope (task detail by URL still works).
        let (filter, prefix) = match (project_req, workspace_req) {
            (Some(p), None) if backlog_req => (ScopeFilter::Backlog(p), format!("/p/{p}/backlog")),
            (Some(p), Some(w)) => {
                match by_id.get(&w) {
                    Some(info) if info.project_id == p && !info.archived => {}
                    _ => return Err(not_found("workspace")),
                }
                (ScopeFilter::Workspace(w), format!("/p/{p}/w/{w}"))
            }
            (Some(p), None) => (ScopeFilter::ProjectVisible(p), format!("/p/{p}")),
            _ => (ScopeFilter::AllVisible, String::new()),
        };
        let header = (!is_htmx).then(|| {
            Arc::new(ScopeHeader {
                project_name,
                project_prefix: project_req.map(|p| format!("/p/{p}")),
                workspace: workspace_req.and_then(|w| by_id.get(&w).cloned()),
                backlog: backlog_req,
            })
        });
        Ok(Scope {
            filter,
            prefix,
            project_id: project_req,
            workspace_id: workspace_req,
            backlog: backlog_req,
            is_htmx,
            workspaces: Arc::new(workspaces),
            projects: Arc::new(projects),
            by_id: Arc::new(by_id),
            sidebar,
            header,
            params: Arc::new(params),
        })
    }
}

/// Raw `?project=&workspace=` query parameters of the JSON API.
#[derive(Debug, Deserialize)]
struct ScopeQuery {
    project: Option<String>,
    workspace: Option<String>,
    backlog: Option<String>,
}

/// Parse the `backlog` query flag: `1`/`true` set it, `0`/`false` clear it, anything else
/// (including an empty value) is a 400.
fn parse_backlog_flag(v: Option<&str>) -> Result<bool, AppError> {
    match v {
        None | Some("0") | Some("false") => Ok(false),
        Some("1") | Some("true") => Ok(true),
        Some(_) => Err(AppError::BadRequest(
            "invalid backlog: expected 1, true, 0 or false".to_string(),
        )),
    }
}

/// Scope filter of a JSON API request, from the optional `project` and `workspace`
/// integer query params and the `backlog` flag. `workspace` wins when both `workspace`
/// and `project` are given; absent means all tasks. `backlog=1` (or `true`) together with
/// `project` selects that project's backlog (tasks without a workspace); `backlog` without
/// `project`, or together with `workspace`, is a 400.
/// A non-integer id (including an empty one) is a 400; an unknown id is a 404.
#[derive(Debug, Clone, Copy)]
pub struct ApiScope(pub ScopeFilter);

impl FromRequestParts<AppState> for ApiScope {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Query(q) = Query::<ScopeQuery>::from_request_parts(parts, state)
            .await
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        let parse = |name: &str, v: Option<String>| -> Result<Option<i64>, AppError> {
            match v.as_deref() {
                None => Ok(None),
                Some(s) => parse_id(s).map(Some).ok_or_else(|| {
                    AppError::BadRequest(format!("invalid {name}: expected an integer id"))
                }),
            }
        };
        let project = parse("project", q.project)?;
        let workspace = parse("workspace", q.workspace)?;
        let backlog = parse_backlog_flag(q.backlog.as_deref())?;
        if backlog && workspace.is_some() {
            return Err(AppError::BadRequest(
                "backlog cannot be combined with workspace".to_string(),
            ));
        }
        if backlog && project.is_none() {
            return Err(AppError::BadRequest(
                "backlog requires a project".to_string(),
            ));
        }
        if project.is_none() && workspace.is_none() {
            return Ok(ApiScope(ScopeFilter::All));
        }
        let db = state.db.clone();
        tokio::task::spawn_blocking(move || -> Result<ApiScope, AppError> {
            let db = db.lock().unwrap();
            if let Some(w) = workspace {
                db.get_workspace(w)
                    .map_err(AppError::Internal)?
                    .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;
                return Ok(ApiScope(ScopeFilter::Workspace(w)));
            }
            let p = project.unwrap_or_default();
            db.get_project(p)
                .map_err(AppError::Internal)?
                .ok_or_else(|| AppError::NotFound("project not found".to_string()))?;
            Ok(ApiScope(if backlog {
                ScopeFilter::Backlog(p)
            } else {
                ScopeFilter::Project(p)
            }))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::{build_sidebar, load_workspaces, parse_backlog_flag, parse_id};
    use crate::db::Database;
    use crate::models::{Status, Task};

    fn backlog_task(id: &str, status: Status) -> Task {
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
    fn test_sidebar_backlog_entries_counts_and_visibility() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        let _p_empty = db.upsert_project("/tmp/a", "a-empty").unwrap();
        let p_only = db.upsert_project("/tmp/b", "b-backlog-only").unwrap();
        let p_arch = db.upsert_project("/tmp/c", "c-archived").unwrap();
        let p_arch_empty = db.upsert_project("/tmp/d", "d-archived-empty").unwrap();
        let w = db.upsert_workspace(p_arch.id, "/tmp/c", "c").unwrap();
        db.archive_workspace(w.id).unwrap();
        let w2 = db.upsert_workspace(p_arch_empty.id, "/tmp/d", "d").unwrap();
        db.archive_workspace(w2.id).unwrap();
        db.insert_backlog_task(&backlog_task("tk-1", Status::Open), p_only.id)
            .unwrap();
        db.insert_backlog_task(&backlog_task("tk-2", Status::Done), p_only.id)
            .unwrap();
        db.insert_backlog_task(&backlog_task("tk-3", Status::InProgress), p_arch.id)
            .unwrap();

        let (projects, workspaces) = load_workspaces(&db, false).unwrap();
        let sb = build_sidebar(&db, &projects, &workspaces, None, None, false).unwrap();
        let names: Vec<&str> = sb.projects.iter().map(|p| p.name.as_str()).collect();
        // The archived project without backlog tasks is hidden; the others are listed,
        // the backlog-only project (no workspaces at all) included.
        assert_eq!(names, ["a-empty", "b-backlog-only", "c-archived"]);
        let by = |n: &str| sb.projects.iter().find(|p| p.name == n).unwrap();
        // Done tasks are not counted; an empty backlog shows 0.
        assert_eq!(by("a-empty").backlog_open, 0);
        assert_eq!(by("b-backlog-only").backlog_open, 1);
        assert_eq!(by("c-archived").backlog_open, 1);
        // The project count covers its backlog; All covers every backlog.
        assert_eq!(by("b-backlog-only").open, 1);
        assert_eq!(sb.all_open, 2);
        assert!(sb.all_active && sb.projects.iter().all(|p| !p.backlog_active));
        // Only the number of archived workspaces is kept, across every project.
        assert_eq!(sb.archived_count, 2);

        // Backlog scope highlights only the backlog entry; project scope only the project.
        let sb = build_sidebar(&db, &projects, &workspaces, Some(p_only.id), None, true).unwrap();
        assert!(!sb.all_active);
        assert!(by_name(&sb, "b-backlog-only").backlog_active);
        assert!(!by_name(&sb, "b-backlog-only").active);
        let sb = build_sidebar(&db, &projects, &workspaces, Some(p_only.id), None, false).unwrap();
        assert!(by_name(&sb, "b-backlog-only").active);
        assert!(!by_name(&sb, "b-backlog-only").backlog_active);
        // The active archived-empty project is shown.
        let sb = build_sidebar(
            &db,
            &projects,
            &workspaces,
            Some(p_arch_empty.id),
            None,
            true,
        )
        .unwrap();
        assert!(sb.projects.iter().any(|p| p.name == "d-archived-empty"));
    }

    fn by_name<'a>(sb: &'a super::Sidebar, name: &str) -> &'a super::SidebarProject {
        sb.projects.iter().find(|p| p.name == name).unwrap()
    }

    #[test]
    fn test_parse_backlog_flag() {
        assert!(!parse_backlog_flag(None).unwrap());
        assert!(!parse_backlog_flag(Some("0")).unwrap());
        assert!(!parse_backlog_flag(Some("false")).unwrap());
        assert!(parse_backlog_flag(Some("1")).unwrap());
        assert!(parse_backlog_flag(Some("true")).unwrap());
        assert!(parse_backlog_flag(Some("")).is_err());
        assert!(parse_backlog_flag(Some("yes")).is_err());
        assert!(parse_backlog_flag(Some("TRUE")).is_err());
    }

    #[test]
    fn test_parse_id_accepts_only_digits() {
        assert_eq!(parse_id("12"), Some(12));
        assert_eq!(parse_id("0"), Some(0));
        assert_eq!(parse_id(""), None);
        assert_eq!(parse_id("-1"), None);
        assert_eq!(parse_id("+1"), None);
        assert_eq!(parse_id("1a"), None);
        assert_eq!(parse_id("99999999999999999999"), None);
    }
}
