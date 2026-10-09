//! Workspace scope resolution: maps the current directory (or an explicit path)
//! to a registered workspace/project and to a [`ScopeFilter`] for queries.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::ValueEnum;

use crate::db::{Database, ScopeFilter};
use crate::models::Workspace;

/// How wide list-type commands look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ScopeMode {
    /// Only the current workspace (default).
    Workspace,
    /// Every workspace of the current project.
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
    pub fn filter(&self, db: &Database) -> Result<ScopeFilter, String> {
        if self.mode == ScopeMode::All {
            return Ok(ScopeFilter::All);
        }
        let Some(resolved) = self.resolve()? else {
            return Ok(ScopeFilter::Unscoped);
        };
        match self.mode {
            ScopeMode::Workspace => {
                Ok(match db.find_workspace_by_path(&resolved.workspace_path)? {
                    Some(w) => ScopeFilter::Workspace(w.id),
                    None => ScopeFilter::Nothing,
                })
            }
            ScopeMode::Project => {
                let project = db
                    .list_projects()?
                    .into_iter()
                    .find(|p| p.path == resolved.project_path);
                Ok(match project {
                    Some(p) => ScopeFilter::Project(p.id),
                    None => ScopeFilter::Nothing,
                })
            }
            ScopeMode::All => Ok(ScopeFilter::All),
        }
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
}
