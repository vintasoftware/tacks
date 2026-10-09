# Workspace Scoping

Tacks (vintasoftware fork) scopes tasks per git worktree ("workspace") and groups
workspaces by repository ("project"). One global database holds all of them, and
one `tk serve` instance shows every project and workspace.

## Database location

- Default path: `~/.tacks/tacks.db`. The directory and schema are created on first use.
- `--db` / `TACKS_DB` still override the path.
- A `.tacks/tacks.db` in the current directory is **not** used anymore.
- `tk init` still works: it initializes the resolved DB and sets the prefix.
- The ID prefix (`tk init --prefix`) is stored in the DB config, so it is global to
  the single DB: every project and workspace shares one prefix.
- `tk prime` exits silently (status 0, no output) when the DB does not exist, when the
  current workspace is not registered (repos that never used tacks stay silent), or when
  the scope cannot be resolved (bad `--workspace` / `TACKS_WORKSPACE`), since hooks
  call it. Outside a git repository (and without `--workspace`) it prints the unscoped
  bucket (`workspace_id IS NULL`).

## Scope resolution

Resolved once per CLI invocation, in this order:

1. `--workspace <path>` global flag, or `TACKS_WORKSPACE` env var.
2. `git rev-parse --show-toplevel` from the current directory.
3. None (not in a git repo): the "unscoped" bucket (`workspace_id IS NULL`).

For a resolved workspace path:

- Workspace path = canonical absolute path of the worktree root.
- Project path = parent of `git rev-parse --path-format=absolute --git-common-dir`
  when it ends in `.git`, else the common dir itself (bare repo). When git is not
  available for the path (explicit `TACKS_WORKSPACE` outside git), project path =
  workspace path.
- Display name = last path component.
- Git runs with `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and `GIT_COMMON_DIR`
  removed from the environment, so the path is authoritative even inside git hooks.
- Submodules (common dir under `.git/modules/`): the project is the submodule's own
  worktree root, not the superproject.

Registration: write commands (`create`, `update --move-to`) insert the project
and workspace rows the first time they are seen. Read commands only look them up;
an unregistered workspace has no tasks.

## Schema (migration v3)

```sql
CREATE TABLE projects (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    path       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE workspaces (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    path       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
ALTER TABLE tasks ADD COLUMN workspace_id INTEGER REFERENCES workspaces(id);  -- nullable
CREATE INDEX idx_tasks_workspace ON tasks(workspace_id);
```

Existing tasks keep `workspace_id = NULL` (the unscoped bucket).

## CLI

Global flags (all optional):

- `--workspace <path>` (env `TACKS_WORKSPACE`): explicit workspace.
- `--scope workspace|project|all` (default `workspace`): widen list-type commands
  to every workspace in the current project, or to the whole DB.
  (`list -a/--all` already means "include closed", so the cross-workspace flag is
  `--scope`, not `--all`.)

Scope filter applies to: `list`, `ready`, `prime`, `blocked`, `epic`, `stats`.

ID-based commands (`show`, `update`, `close`, `comment`, `dep`, `children`) work
on any task in the DB, whatever the scope. An agent in workspace B can claim a
task from workspace A by ID.

- `create`: sets `workspace_id` to the current workspace. A subtask
  (`--parent`) inherits the parent's workspace.
- `update <id> --move-to <path>`: moves the task and all its descendants to
  that workspace (registered if new; the path resolves like `--workspace`).
  `--move-to none` moves them to the unscoped bucket. (Not `--workspace`: that
  is the global flag selecting the current workspace.) Tasks already in the target
  workspace are untouched.
- Reparent: `update <id> --parent <epic>` moves the task and its whole subtree to the
  epic's workspace (subtasks always live in their parent's workspace).
  `--parent none` (promote) keeps the task's current workspace. If `--parent` and
  `--move-to` are both given, the reparent is applied first and then the explicit
  `--move-to`, so the explicit move wins. Web `PATCH /api/tasks/{id}` behaves the
  same with `parent_id` and `workspace_id`.
- Moves bump `updated_at` on non-done tasks only; done tasks keep it, because the board
  uses `updated_at` of done tasks as their completion time.
- `update` is atomic: the `--move-to` target is resolved first, then all changes (fields,
  tags, reparent, move) run in one transaction; any failure leaves the task unchanged.
  Web `PATCH /api/tasks/{id}` validates status and workspace id up front and also applies
  everything in one transaction.
- `tk workspaces`: lists projects and their workspaces, with task counts (open, wip,
  blocked, done) and a `missing` flag when the workspace path no longer exists on disk.

JSON: Task gets one new nullable field, `workspace_id`. `show --json` gets a
`workspace` object (`id`, `name`, `path`, `project_id`, `project_name`,
`missing`) or `null`. No existing field changes.

## Deleted workspaces

No data changes. A workspace is "missing" when its path no longer exists
(checked at read time; in the web UI only for requests that render it). Its tasks stay in the project. The CLI (`show`,
`workspaces`) and the web UI mark them, and the user can move them to another
workspace, or an agent elsewhere can claim them by ID.

## Web UI

- Sidebar (full-page renders only, not HTMX partials): "All" at the top, then each
  project with its workspaces nested, with counts of tasks that are not done. The
  active scope is highlighted. Workspaces whose path no longer exists are shown muted
  with a "missing" label. Switching scope keeps the current view (board/tasks/epics).
  On narrow screens the sidebar moves above the content.
- Scope in the URL path, by numeric ID. One set of handlers serves all three prefixes
  (`Scope` extractor in `src/web/scope.rs`):
  - `/board`, `/tasks`, `/epics`, `/epics/{id}`: all projects.
  - `/p/{project_id}/board|tasks|epics|epics/{id}`: every workspace in one project
    (missing-workspace tasks included, marked on cards).
  - `/p/{project_id}/w/{workspace_id}/board|tasks|epics|epics/{id}`: one workspace.
  - `tasks/new` and `tasks/new/modal` exist under each prefix too (the create form
    needs the scope); `POST {prefix}/tasks` redirects back to `{prefix}/tasks`.
  - Task detail (`/tasks/{id}`), `/tasks/{id}/dep-tree` and `/epics/{id}/dep-tree`
    stay global.
- Unknown project or workspace id, non-numeric id, or a workspace that does not belong
  to the project in the URL: 404.
- Links, forms, nav tabs, HTMX poll URLs and partial URLs keep the scope prefix.
- Task cards and rows show the workspace name as a badge in project and All scopes,
  marked "missing" when the path is gone (hidden in workspace scope: redundant).
- Task detail (page and modal): workspace name, path, project, "missing" marker (or
  "none"), and a selector to move the task. It sends `PATCH /api/tasks/{id}` with
  `workspace_id`; descendants move too (the UI says so). Missing workspaces are listed
  but disabled as move targets.
- Create form: in workspace scope new tasks go to that workspace (hidden field). In
  project scope a workspace select defaults to the project's first non-missing
  workspace; in All scope it defaults to "none". Missing workspaces are not offered.
  Subtasks (with a parent) always inherit the parent's workspace.
- JSON API (additive): optional integer `project` and `workspace` query params on
  `/api/tasks`, `/api/tasks/ready`, `/api/tasks/blocked`, `/api/epics`, `/api/stats`,
  `/api/tags`, `/api/prime`. `workspace` wins when both are given; absent means all.
  Non-integer: 400; well-formed but unknown id: 404.
  `GET /api/workspaces` returns the same rows as `tk workspaces --json` (`current` is
  always false). `PATCH /api/tasks/{id}` accepts `workspace_id` (integer, or `null` for
  unscoped; absent leaves it unchanged; unknown workspace: 404). `POST /api/tasks`
  accepts optional `workspace_id` (ignored for subtasks; unknown workspace: 404).
- Paths from the DB are only displayed and checked with `exists()`, never read or served.
