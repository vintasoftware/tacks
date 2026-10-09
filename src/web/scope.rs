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

use axum::extract::{FromRequestParts, Query, RawPathParams};
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
}

/// One `<option>` of a workspace `<select>`.
#[derive(Debug, Clone)]
pub struct WsOption {
    pub id: i64,
    pub name: String,
    pub missing: bool,
    pub selected: bool,
    /// Missing workspaces cannot be chosen as a target unless already selected.
    pub disabled: bool,
}

/// Workspaces of one project, for grouped `<select>` options.
#[derive(Debug, Clone)]
pub struct WsGroup {
    pub project_name: String,
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
    pub open: i64,
    pub active: bool,
    pub workspaces: Vec<SidebarWorkspace>,
}

/// Sidebar contents: the "All" entry and every project with its workspaces.
#[derive(Debug, Clone)]
pub struct Sidebar {
    pub all_active: bool,
    pub all_open: i64,
    pub projects: Vec<SidebarProject>,
}

/// What base.html needs: URL prefix for the current scope, the current view
/// (`tasks`, `board`, `epics`) and the sidebar (absent for HTMX requests).
#[derive(Debug, Clone)]
pub struct Layout {
    /// `""`, `"/p/1"` or `"/p/1/w/2"`. Prepend to `/board`, `/tasks`, `/epics`.
    pub prefix: String,
    pub view: &'static str,
    pub sidebar: Option<Arc<Sidebar>>,
}

/// The scope of an HTML request, resolved from the URL path.
#[derive(Debug, Clone)]
pub struct Scope {
    pub filter: ScopeFilter,
    /// `""`, `"/p/{project}"` or `"/p/{project}/w/{workspace}"`.
    pub prefix: String,
    pub project_id: Option<i64>,
    pub workspace_id: Option<i64>,
    /// The request came from HTMX (`HX-Request`), so no sidebar is rendered.
    pub is_htmx: bool,
    /// Every workspace, ordered by project name then workspace name.
    pub workspaces: Arc<Vec<WsInfo>>,
    by_id: Arc<HashMap<i64, WsInfo>>,
    sidebar: Option<Arc<Sidebar>>,
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

    /// True when views should show a workspace badge on each task (project and All scopes).
    pub fn show_workspace_badge(&self) -> bool {
        self.workspace_id.is_none()
    }

    /// A non-scope path parameter (e.g. `id`).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(String::as_str)
    }

    /// Workspaces grouped by project as `<option>` data, with `selected` marked.
    /// Missing workspaces are left out when `include_missing` is false; when included they
    /// are disabled unless currently selected.
    pub fn groups(&self, selected: Option<i64>, include_missing: bool) -> Vec<WsGroup> {
        let mut groups: Vec<(i64, WsGroup)> = Vec::new();
        for w in self.workspaces.iter() {
            let is_selected = selected == Some(w.id);
            if w.missing && !include_missing && !is_selected {
                continue;
            }
            let opt = WsOption {
                id: w.id,
                name: w.name.clone(),
                missing: w.missing,
                selected: is_selected,
                disabled: w.missing && !is_selected,
            };
            match groups.last_mut() {
                Some((pid, g)) if *pid == w.project_id => g.options.push(opt),
                _ => groups.push((
                    w.project_id,
                    WsGroup {
                        project_name: w.project_name.clone(),
                        options: vec![opt],
                    },
                )),
            }
        }
        groups.into_iter().map(|(_, g)| g).collect()
    }

    /// Default workspace for new tasks: the workspace in workspace scope, the first
    /// non-missing workspace of the project in project scope, none in All scope.
    pub fn default_workspace_id(&self) -> Option<i64> {
        if let Some(w) = self.workspace_id {
            return Some(w);
        }
        let project = self.project_id?;
        self.workspaces
            .iter()
            .find(|w| w.project_id == project && !w.missing)
            .map(|w| w.id)
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
fn load_workspaces(db: &Database, check_missing: bool) -> Result<Vec<WsInfo>, String> {
    let names: HashMap<i64, String> = db
        .list_projects()?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    Ok(db
        .list_workspaces()?
        .into_iter()
        .map(|w| WsInfo {
            id: w.id,
            project_name: names.get(&w.project_id).cloned().unwrap_or_default(),
            project_id: w.project_id,
            missing: check_missing && !FsPath::new(&w.path).exists(),
            name: w.name,
            path: w.path,
        })
        .collect())
}

/// Build the sidebar tree with non-done task counts and the active scope highlighted.
fn build_sidebar(
    db: &Database,
    workspaces: &[WsInfo],
    project_id: Option<i64>,
    workspace_id: Option<i64>,
) -> Result<Sidebar, String> {
    let counts: HashMap<Option<i64>, i64> = db
        .workspace_task_counts()?
        .into_iter()
        .map(|c| (c.workspace_id, c.open + c.in_progress + c.blocked))
        .collect();
    let all_open: i64 = counts.values().sum();
    let mut projects: Vec<SidebarProject> = Vec::new();
    for p in db.list_projects()? {
        let mut sp = SidebarProject {
            id: p.id,
            name: p.name,
            open: 0,
            active: project_id == Some(p.id) && workspace_id.is_none(),
            workspaces: Vec::new(),
        };
        for w in workspaces.iter().filter(|w| w.project_id == p.id) {
            let open = counts.get(&Some(w.id)).copied().unwrap_or(0);
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
        projects.push(sp);
    }
    Ok(Sidebar {
        all_active: project_id.is_none() && workspace_id.is_none(),
        all_open,
        projects,
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
        let (workspaces, sidebar, project_exists) = tokio::task::spawn_blocking(move || {
            let db = db.lock().unwrap();
            // `missing` is only rendered by the sidebar (full pages) and by workspace
            // badges (project/All scopes). HTMX polls in workspace scope render neither,
            // so skip the filesystem checks there.
            let check_missing = !is_htmx || workspace_req.is_none();
            let workspaces = load_workspaces(&db, check_missing)?;
            let project_exists = match project_req {
                Some(id) => db.get_project(id)?.is_some(),
                None => true,
            };
            let sidebar = if is_htmx || !project_exists {
                None
            } else {
                Some(Arc::new(build_sidebar(
                    &db,
                    &workspaces,
                    project_req,
                    workspace_req,
                )?))
            };
            Ok::<_, String>((workspaces, sidebar, project_exists))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Internal)?;

        let by_id: HashMap<i64, WsInfo> = workspaces.iter().map(|w| (w.id, w.clone())).collect();
        if !project_exists {
            return Err(not_found("project"));
        }
        let (filter, prefix) = match (project_req, workspace_req) {
            (Some(p), Some(w)) => {
                match by_id.get(&w) {
                    Some(info) if info.project_id == p => {}
                    _ => return Err(not_found("workspace")),
                }
                (ScopeFilter::Workspace(w), format!("/p/{p}/w/{w}"))
            }
            (Some(p), None) => (ScopeFilter::Project(p), format!("/p/{p}")),
            _ => (ScopeFilter::All, String::new()),
        };
        Ok(Scope {
            filter,
            prefix,
            project_id: project_req,
            workspace_id: workspace_req,
            is_htmx,
            workspaces: Arc::new(workspaces),
            by_id: Arc::new(by_id),
            sidebar,
            params: Arc::new(params),
        })
    }
}

/// Raw `?project=&workspace=` query parameters of the JSON API.
#[derive(Debug, Deserialize)]
struct ScopeQuery {
    project: Option<String>,
    workspace: Option<String>,
}

/// Scope filter of a JSON API request, from the optional `project` and `workspace`
/// integer query params. `workspace` wins when both are given; absent means all tasks.
/// A non-integer value (including an empty one) is a 400; an unknown id is a 404.
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
            Ok(ApiScope(ScopeFilter::Project(p)))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::parse_id;

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
