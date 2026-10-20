//! Workspace scope resolution: maps the current directory (or an explicit path)
//! to a registered workspace/project and to a [`ScopeFilter`] for queries.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::ValueEnum;

use crate::db::{Database, ScopeFilter};
use crate::models::{Project, Task, Workspace};

/// How wide list-type commands look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ScopeMode {
    /// Only the current workspace (default).
    Workspace,
    /// The current project's backlog: tasks in the project but in no workspace.
    Backlog,
    /// Every workspace of the current project, plus its backlog.
    Project,
    /// The whole database.
    All,
}

/// A workspace (worktree) and its project, as resolved from the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspace {
    /// Canonical absolute path of the worktree root.
    pub workspace_path: String,
    /// Display name (last path component).
    pub workspace_name: String,
    /// Canonical absolute path of the project (repository) root.
    pub project_path: String,
    /// Display name (last path component).
    pub project_name: String,
}

/// Scope request for one CLI invocation: explicit workspace path plus mode.
#[derive(Debug, Clone)]
pub struct Scope {
    /// `--workspace` / `TACKS_WORKSPACE`.
    pub explicit: Option<PathBuf>,
    /// `--scope`.
    pub mode: ScopeMode,
}

impl Scope {
    /// Resolve the current workspace from the explicit path or the current directory.
    pub fn resolve(&self) -> Result<Option<ResolvedWorkspace>, String> {
        let cwd = std::env::current_dir()
            .map_err(|e| format!("cannot determine current directory: {e}"))?;
        resolve(self.explicit.as_deref(), &cwd)
    }

    /// Read-only lookup of the current workspace row, if it is registered.
    pub fn registered_workspace(&self, db: &Database) -> Result<Option<Workspace>, String> {
        match self.resolve()? {
            Some(r) => db.find_workspace_by_path(&r.workspace_path),
            None => Ok(None),
        }
    }

    /// Translate the scope into a query filter without registering anything.
    ///
    /// `--scope backlog` needs a resolvable workspace (to find the project) and fails with
    /// [`NO_BACKLOG_ERROR`] otherwise; an unregistered project has no tasks (`Nothing`).
    pub fn filter(&self, db: &Database) -> Result<ScopeFilter, String> {
        if self.mode == ScopeMode::All {
            return Ok(ScopeFilter::All);
        }
        let Some(resolved) = self.resolve()? else {
            if self.mode == ScopeMode::Backlog {
                return Err(NO_BACKLOG_ERROR.to_string());
            }
            return Ok(ScopeFilter::Unscoped);
        };
        match self.mode {
            ScopeMode::Workspace => {
                Ok(match db.find_workspace_by_path(&resolved.workspace_path)? {
                    Some(w) => ScopeFilter::Workspace(w.id),
                    None => ScopeFilter::Nothing,
                })
            }
            ScopeMode::Backlog => Ok(match find_project(db, &resolved.project_path)? {
                Some(p) => ScopeFilter::Backlog(p.id),
                None => ScopeFilter::Nothing,
            }),
            ScopeMode::Project => Ok(match find_project(db, &resolved.project_path)? {
                Some(p) => ScopeFilter::Project(p.id),
                None => ScopeFilter::Nothing,
            }),
            ScopeMode::All => Ok(ScopeFilter::All),
        }
    }
}

/// Error for `--scope backlog` / `create --to-backlog` when no project can be resolved.
pub const NO_BACKLOG_ERROR: &str =
    "not in a git repository, so there is no project backlog; use --workspace <path>";

/// Read-only lookup of a registered project by its path.
fn find_project(db: &Database, path: &str) -> Result<Option<Project>, String> {
    Ok(db.list_projects()?.into_iter().find(|p| p.path == path))
}

/// Guard for ID-based write commands: refuse a task outside the allowed scope.
///
/// Read-only (lookups only), so callers run it before any write. A missing task
/// passes here; the command's own "task not found" error then applies.
///
/// A task is in a workspace (`workspace_id` set), in a project backlog (`workspace_id`
/// NULL, `project_id` set) or unscoped (both NULL).
///
/// - `--scope all`: always allowed.
/// - `workspace`: the task's workspace must be the current registered workspace
///   (no resolvable workspace: the task must be unscoped). Backlog tasks are out of scope.
/// - `backlog`: only backlog tasks of the current project (no resolvable workspace is an
///   error, there is no project).
/// - `project`: the task's workspace, or its backlog project, must be the current
///   project (no resolvable workspace: the task must be unscoped).
pub fn ensure_in_scope(db: &Database, task: &Task, scope: &Scope) -> Result<(), String> {
    if scope.mode == ScopeMode::All {
        return Ok(());
    }
    let current = scope.resolve()?;
    if current.is_none() && scope.mode == ScopeMode::Backlog {
        return Err(NO_BACKLOG_ERROR.to_string());
    }
    let task_ws = match task.workspace_id {
        Some(id) => db.get_workspace(id)?,
        None => None,
    };
    // Workspace tasks: the workspace's project. Backlog tasks: `project_id`.
    let task_project = match (&task_ws, task.project_id) {
        (Some(w), _) => db.get_project(w.project_id)?,
        (None, Some(pid)) => db.get_project(pid)?,
        (None, None) => None,
    };
    let is_backlog = task.workspace_id.is_none() && task.project_id.is_some();
    let in_current_project = match (&current, &task_project) {
        (Some(r), Some(p)) => p.path == r.project_path,
        _ => false,
    };
    let allowed = if is_backlog {
        match scope.mode {
            ScopeMode::Backlog | ScopeMode::Project => in_current_project,
            _ => false,
        }
    } else {
        match (&current, task.workspace_id) {
            (None, None) => true,
            (None, Some(_)) | (Some(_), None) => false,
            (Some(r), Some(wid)) => match scope.mode {
                ScopeMode::Workspace => db
                    .find_workspace_by_path(&r.workspace_path)?
                    .is_some_and(|w| w.id == wid),
                ScopeMode::Project => in_current_project,
                // backlog mode only covers backlog tasks; `all` returned above
                ScopeMode::Backlog | ScopeMode::All => false,
            },
        }
    };
    if allowed {
        return Ok(());
    }
    let hint = if in_current_project && scope.mode == ScopeMode::Workspace && is_backlog {
        "--scope backlog"
    } else if in_current_project && scope.mode != ScopeMode::Project && !is_backlog {
        "--scope project"
    } else {
        "--scope all"
    };
    let project_name = task_project
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let place = match (task.workspace_id, task.project_id) {
        (None, None) => "is unscoped (no workspace, no project)".to_string(),
        (None, Some(_)) => format!("is in the backlog of project {project_name}"),
        (Some(wid), _) => {
            let ws = task_ws
                .as_ref()
                .map(|w| w.name.clone())
                .unwrap_or_else(|| format!("#{wid}"));
            format!("belongs to workspace {ws} (project {project_name})")
        }
    };
    let what = match scope.mode {
        ScopeMode::Project => "current project",
        ScopeMode::Backlog => "current project's backlog",
        _ => "current workspace",
    };
    Err(format!(
        "{} {place}, not the {what}. do not act on it unless the user explicitly asked you to; if they did, rerun with {hint}",
        task.id
    ))
}

/// [`ensure_in_scope`] by task id; an unknown id passes (the caller reports it).
pub fn ensure_id_in_scope(db: &Database, id: &str, scope: &Scope) -> Result<(), String> {
    match db.get_task(id)? {
        Some(t) => ensure_in_scope(db, &t, scope),
        None => Ok(()),
    }
}

/// Register (insert if new) the project and workspace; used by write commands only.
pub fn register(db: &Database, resolved: &ResolvedWorkspace) -> Result<Workspace, String> {
    let project = db.upsert_project(&resolved.project_path, &resolved.project_name)?;
    db.upsert_workspace(
        project.id,
        &resolved.workspace_path,
        &resolved.workspace_name,
    )
}

/// Resolve a workspace from an optional explicit path and the current directory.
///
/// An explicit path must exist; inside a git repo it maps to the worktree root.
/// Without an explicit path, `dir` must be inside a git repo, else `None`.
pub fn resolve(explicit: Option<&Path>, dir: &Path) -> Result<Option<ResolvedWorkspace>, String> {
    match explicit {
        Some(p) => {
            let canon = std::fs::canonicalize(p)
                .map_err(|e| format!("workspace path not usable: {}: {e}", p.display()))?;
            Ok(Some(
                resolve_in_git(&canon).unwrap_or_else(|| plain_workspace(&canon)),
            ))
        }
        None => Ok(resolve_in_git(dir)),
    }
}

fn plain_workspace(path: &Path) -> ResolvedWorkspace {
    let p = path.to_string_lossy().into_owned();
    let name = dir_name(path);
    ResolvedWorkspace {
        workspace_path: p.clone(),
        workspace_name: name.clone(),
        project_path: p,
        project_name: name,
    }
}

fn resolve_in_git(dir: &Path) -> Option<ResolvedWorkspace> {
    let top = std::fs::canonicalize(git_output(dir, &["rev-parse", "--show-toplevel"])?).ok()?;
    let common = git_output(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .and_then(|c| std::fs::canonicalize(c).ok());
    let project = match common {
        Some(c) => project_path_from_common_dir(&c, &top),
        None => top.clone(),
    };
    Some(ResolvedWorkspace {
        workspace_path: top.to_string_lossy().into_owned(),
        workspace_name: dir_name(&top),
        project_path: project.to_string_lossy().into_owned(),
        project_name: dir_name(&project),
    })
}

/// Parent of the common git dir when it ends in `.git`, else the dir itself (bare repo).
/// A submodule's common dir lives under `<super>/.git/modules/...`; there the project is
/// the submodule's own worktree toplevel (`top`).
fn project_path_from_common_dir(common: &Path, top: &Path) -> PathBuf {
    let comps: Vec<_> = common.components().collect();
    if comps
        .windows(2)
        .any(|w| w[0].as_os_str() == ".git" && w[1].as_os_str() == "modules")
    {
        return top.to_path_buf();
    }
    if common.file_name().is_some_and(|n| n == ".git")
        && let Some(parent) = common.parent()
    {
        return parent.to_path_buf();
    }
    common.to_path_buf()
}

/// Last path component, falling back to the full path (e.g. for `/`).
fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Run git in `dir` (no shell, stderr silenced); trimmed stdout on success.
fn git_output(dir: &Path, args: &[&str]) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Inside git hooks these point at the hook's repo and would override `-C`.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim_end_matches(['\n', '\r']);
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_project_path_from_common_dir_regular_repo() {
        assert_eq!(
            project_path_from_common_dir(Path::new("/a/b/repo/.git"), Path::new("/a/b/repo")),
            PathBuf::from("/a/b/repo")
        );
    }

    #[test]
    fn test_project_path_from_common_dir_submodule() {
        assert_eq!(
            project_path_from_common_dir(
                Path::new("/a/super/.git/modules/sub"),
                Path::new("/a/super/sub")
            ),
            PathBuf::from("/a/super/sub")
        );
    }

    #[test]
    fn test_project_path_from_common_dir_bare_repo() {
        assert_eq!(
            project_path_from_common_dir(Path::new("/a/b/repo.git"), Path::new("/x/wt")),
            PathBuf::from("/a/b/repo.git")
        );
    }

    #[test]
    fn test_dir_name() {
        assert_eq!(dir_name(Path::new("/a/b/repo")), "repo");
        assert_eq!(dir_name(Path::new("/")), "/");
    }

    #[test]
    fn test_plain_workspace_uses_path_for_project() {
        let r = plain_workspace(Path::new("/x/y"));
        assert_eq!(r.workspace_path, "/x/y");
        assert_eq!(r.project_path, "/x/y");
        assert_eq!(r.project_name, "y");
    }

    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn test_resolve_repo_worktree_and_non_repo() {
        let tmp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let repo = root.join("main-repo");
        std::fs::create_dir(&repo).unwrap();
        if !git(&repo, &["init", "-q"]) {
            return; // git unavailable
        }
        let ok = git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        let wt = root.join("wt-two");
        if !ok || !git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]) {
            return;
        }
        std::fs::create_dir(repo.join("sub")).unwrap();

        let a = resolve(None, &repo.join("sub")).unwrap().unwrap();
        assert_eq!(a.workspace_path, repo.to_string_lossy());
        assert_eq!(a.workspace_name, "main-repo");
        assert_eq!(a.project_path, repo.to_string_lossy());

        let b = resolve(None, &wt).unwrap().unwrap();
        assert_eq!(b.workspace_path, wt.to_string_lossy());
        assert_eq!(b.workspace_name, "wt-two");
        assert_eq!(b.project_path, repo.to_string_lossy());
        assert_eq!(b.project_name, "main-repo");

        // explicit subdir maps to worktree root
        let c = resolve(Some(&wt.join(".")), &repo).unwrap().unwrap();
        assert_eq!(c.workspace_path, wt.to_string_lossy());

        // non-repo: unscoped without explicit, plain with explicit
        let plain = root.join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(resolve(None, &plain).unwrap(), None);
        let d = resolve(Some(&plain), &repo).unwrap().unwrap();
        assert_eq!(d.workspace_path, plain.to_string_lossy());
        assert_eq!(d.project_path, d.workspace_path);

        assert!(resolve(Some(&root.join("missing")), &repo).is_err());
    }

    /// A DB with two plain (non-git) projects: tasks in A's workspace, in A's backlog,
    /// in B's backlog, in B's workspace, and unscoped.
    struct Fixture {
        db: Database,
        _tmp: TempDir,
        dir_a: PathBuf,
        ws_task: Task,
        backlog_a: Task,
        backlog_b: Task,
        ws_b: Task,
        unscoped: Task,
    }

    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let dir_a = root.join("proj-a");
        let dir_b = root.join("proj-b");
        std::fs::create_dir(&dir_a).unwrap();
        std::fs::create_dir(&dir_b).unwrap();
        let db = Database::open(&root.join("t.db")).unwrap();
        let ra = resolve(Some(&dir_a), &root).unwrap().unwrap();
        let rb = resolve(Some(&dir_b), &root).unwrap().unwrap();
        let wa = register(&db, &ra).unwrap();
        let wb = register(&db, &rb).unwrap();
        let mk = |ws: Option<i64>, project: Option<i64>| {
            let now = chrono::Utc::now();
            let t = Task {
                id: db.generate_id().unwrap(),
                title: "t".to_string(),
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
                workspace_id: ws,
                project_id: project,
            };
            match (ws, project) {
                (None, Some(p)) => db.insert_backlog_task(&t, p).unwrap(),
                _ => db.insert_task(&t).unwrap(),
            }
            db.get_task(&t.id).unwrap().unwrap()
        };
        let ws_task = mk(Some(wa.id), None);
        let ws_b = mk(Some(wb.id), None);
        let backlog_a = mk(None, Some(wa.project_id));
        let backlog_b = mk(None, Some(wb.project_id));
        let unscoped = mk(None, None);
        Fixture {
            db,
            _tmp: tmp,
            dir_a,
            ws_task,
            backlog_a,
            backlog_b,
            ws_b,
            unscoped,
        }
    }

    fn scope_in(dir: &Path, mode: ScopeMode) -> Scope {
        Scope {
            explicit: Some(dir.to_path_buf()),
            mode,
        }
    }

    #[test]
    fn test_filter_backlog_registered_and_unregistered() {
        let f = fixture();
        let filter = scope_in(&f.dir_a, ScopeMode::Backlog)
            .filter(&f.db)
            .unwrap();
        assert_eq!(
            filter,
            ScopeFilter::Backlog(f.backlog_a.project_id.unwrap())
        );
        let other = f.dir_a.parent().unwrap().join("fresh");
        std::fs::create_dir(&other).unwrap();
        let filter = scope_in(&other, ScopeMode::Backlog).filter(&f.db).unwrap();
        assert_eq!(filter, ScopeFilter::Nothing);
    }

    #[test]
    fn test_guard_workspace_scope_refuses_backlog_with_hint() {
        let f = fixture();
        let sc = scope_in(&f.dir_a, ScopeMode::Workspace);
        assert!(ensure_in_scope(&f.db, &f.ws_task, &sc).is_ok());
        let err = ensure_in_scope(&f.db, &f.backlog_a, &sc).unwrap_err();
        assert!(err.contains("is in the backlog of project proj-a"), "{err}");
        assert!(err.contains("not the current workspace"), "{err}");
        assert!(err.ends_with("rerun with --scope backlog"), "{err}");
        let err = ensure_in_scope(&f.db, &f.backlog_b, &sc).unwrap_err();
        assert!(err.contains("backlog of project proj-b"), "{err}");
        assert!(err.ends_with("rerun with --scope all"), "{err}");
        let err = ensure_in_scope(&f.db, &f.unscoped, &sc).unwrap_err();
        assert!(err.contains("is unscoped"), "{err}");
    }

    #[test]
    fn test_guard_backlog_scope_allows_only_current_project_backlog() {
        let f = fixture();
        let sc = scope_in(&f.dir_a, ScopeMode::Backlog);
        assert!(ensure_in_scope(&f.db, &f.backlog_a, &sc).is_ok());
        let err = ensure_in_scope(&f.db, &f.ws_task, &sc).unwrap_err();
        assert!(err.contains("belongs to workspace proj-a"), "{err}");
        assert!(err.contains("not the current project's backlog"), "{err}");
        assert!(err.ends_with("rerun with --scope project"), "{err}");
        let err = ensure_in_scope(&f.db, &f.backlog_b, &sc).unwrap_err();
        assert!(err.ends_with("rerun with --scope all"), "{err}");
        assert!(ensure_in_scope(&f.db, &f.unscoped, &sc).is_err());
        assert!(ensure_in_scope(&f.db, &f.ws_b, &sc).is_err());
    }

    #[test]
    fn test_guard_project_scope_covers_workspaces_and_backlog() {
        let f = fixture();
        let sc = scope_in(&f.dir_a, ScopeMode::Project);
        assert!(ensure_in_scope(&f.db, &f.ws_task, &sc).is_ok());
        assert!(ensure_in_scope(&f.db, &f.backlog_a, &sc).is_ok());
        assert!(ensure_in_scope(&f.db, &f.backlog_b, &sc).is_err());
        assert!(ensure_in_scope(&f.db, &f.ws_b, &sc).is_err());
        assert!(ensure_in_scope(&f.db, &f.unscoped, &sc).is_err());
    }

    #[test]
    fn test_guard_all_scope_allows_everything() {
        let f = fixture();
        let sc = scope_in(&f.dir_a, ScopeMode::All);
        for t in [&f.ws_task, &f.backlog_a, &f.backlog_b, &f.ws_b, &f.unscoped] {
            assert!(ensure_in_scope(&f.db, t, &sc).is_ok());
        }
    }
}
