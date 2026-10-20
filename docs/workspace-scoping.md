# Workspace Scoping

Tacks (vintasoftware fork) scopes tasks per git worktree ("workspace") and groups
workspaces by repository ("project"). One global database holds all of them, and
one `tk serve` instance shows every project and workspace. Each project also has a
backlog: tasks that belong to the project but to no workspace (see the backlog sections
below).

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
  bucket (no workspace, no project).

## Scope resolution

Resolved once per CLI invocation, in this order:

1. `--workspace <path>` global flag, or `TACKS_WORKSPACE` env var.
2. `git rev-parse --show-toplevel` from the current directory.
3. None (not in a git repo): the "unscoped" bucket (no workspace, no project). With
   `--scope backlog` this is an error instead (see below).

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

Registration: write commands (`create`, `create --to-backlog`, `update --move-to`,
`update --to-backlog`, `update --claim` on a backlog task) insert the project and workspace
rows the first time they are seen. Read commands only look them up; an unregistered
workspace has no tasks, and an unregistered project has no backlog.

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

## Schema (migration v7): project backlog

```sql
ALTER TABLE tasks ADD COLUMN project_id INTEGER REFERENCES projects(id);  -- nullable
CREATE INDEX idx_tasks_project ON tasks(project_id);
UPDATE tasks SET project_id = (SELECT w.project_id FROM workspaces w WHERE w.id = tasks.workspace_id)
WHERE workspace_id IS NOT NULL;  -- backfill
```

A task has one of three placements:

| `workspace_id` | `project_id` | Meaning |
|---|---|---|
| set | set (the workspace's project) | workspace task |
| NULL | set | **project backlog** task |
| NULL | NULL | unscoped (existing tasks, tasks created outside a repo) |

The backfill only touches workspace tasks; unscoped tasks keep `project_id = NULL`.

Sync rule: every write that sets the placement also sets `project_id`.

- `insert_task`: with a workspace, `project_id` is the workspace's project (any value in the
  task is ignored). Without one, the task's `project_id` puts it in that backlog. A subtask of a
  backlog parent inherits the parent's project.
- `set_task_workspace` (`--move-to`, web move): `project_id` becomes the workspace's project,
  or NULL when the target is unscoped. So `--move-to none` on a backlog task makes it unscoped.
- `move_task_to_backlog` (`--to-backlog`, web move): `workspace_id = NULL`, `project_id = P`.
- Both moves, and reparenting (`update --parent`, which follows the new parent's workspace or
  backlog), apply to the task and its whole subtree.

Project filters match workspace tasks through the `workspaces` row, not through
`tasks.project_id`; `project_id` decides only backlog membership. A workspace task with
`project_id = NULL` (written by an older binary) still belongs to its project.

Mixed-version caveat: a `tk` binary older than this migration, run on a migrated DB, only
writes `workspace_id`. `update --move-to none` then leaves `project_id` set, so the task
turns into a backlog task of that project instead of unscoped. Other old-binary moves and
creates can leave `project_id` NULL or stale on workspace tasks (harmless for filters and the
guard, which read the workspace first, but the task JSON shows the stale value). Upgrade every
`tk` that shares the DB.

## CLI

Global flags (all optional):

- `--workspace <path>` (env `TACKS_WORKSPACE`): explicit workspace.
- `--scope workspace|backlog|project|all` (default `workspace`): widen list-type commands
  to the backlog of the current project, to every workspace in the current project plus its
  backlog, or to the whole DB.
  (`list -a/--all` already means "include closed", so the cross-workspace flag is
  `--scope`, not `--all`.)

| Scope | Covers |
|---|---|
| `workspace` | the current workspace |
| `backlog` | the current project's backlog (`workspace_id IS NULL AND project_id = P`) |
| `project` | all workspaces of the current project, plus its backlog |
| `all` | everything, including other projects' backlogs and the unscoped bucket |

Scope filter applies to: `list`, `ready`, `prime`, `blocked`, `epic`, `stats`.

The project comes from the current directory as for `--scope project`; `--workspace` /
`TACKS_WORKSPACE` select another repo. `--scope backlog` outside a git repository (and
without `--workspace`) is an error: `not in a git repository, so there is no project backlog;
use --workspace <path>` (exit 1; `tk prime` stays silent instead). An unregistered project has
an empty backlog.

### Write guard

Agents must stay inside their workspace unless told otherwise, so ID-based **write**
commands refuse (exit 1, nothing written) a task outside the allowed scope. The allowed
scope is the global `--scope` flag:

- `workspace` (default): the task's `workspace_id` must equal the current registered
  workspace. Without a resolvable workspace (non-git dir, no `--workspace`) the task must be
  unscoped. If the current workspace is not registered, no existing task is in scope.
- `backlog`: the task must be a backlog task of the current project. Workspace tasks, other
  projects' backlogs and unscoped tasks are out of scope. Outside a repo: the error above.
- `project`: the task's workspace must belong to the current project, or the task must be in
  that project's backlog (both unscoped when not in a repo).
- `all`: any task.

In the default `workspace` scope backlog tasks are out of scope, even those of the current
project. "Unscoped" means neither workspace nor project.

`--workspace` / `TACKS_WORKSPACE` change what "current" means, as for list commands.

Guarded commands: `update` (all fields, `--claim`, `--notes`), `close`, `comment`,
`dep add` / `dep remove` (both tasks), `create --parent` (the parent). Specifics:

- `update --parent E`: both the task and E must be in scope.
- `update --move-to`: the task being moved must be in scope. You can push your own task
  out of the workspace; pulling a foreign task in needs `--scope`.
- The check is read-only and runs before any write (`ensure_in_scope` in `src/scope.rs`).
  An unknown id is not a scope error; the command's usual "task not found" applies.

Error example (stderr, exit 1):

```
error: tk-a1b2 belongs to workspace feat-login (project app), not the current workspace. do not act on it unless the user explicitly asked you to; if they did, rerun with --scope project
```

The hint says `--scope all` when the task is in another project or unscoped. Other texts:

```
error: tk-a1b2 is in the backlog of project app, not the current workspace. do not act on it unless the user explicitly asked you to; if they did, rerun with --scope backlog
error: tk-a1b2 belongs to workspace feat-login (project app), not the current project's backlog. do not act on it unless the user explicitly asked you to; if they did, rerun with --scope project
```

The hint is `--scope backlog` for a backlog task of the current project under `workspace`
scope, and `--scope project` for a workspace task of the current project under `workspace`
or `backlog` scope. The "not the current ..." part is `workspace`, `project` or
`project's backlog`, after the active `--scope`.

Exceptions:

- **Reads by ID stay unrestricted**: `show` and `children`. Reading is harmless and lets
  an agent see what another workspace is doing (coordination).
- **Web UI and JSON API are unrestricted**: they are the human operator's tools.

Other commands:

- `create`: sets `workspace_id` to the current workspace. A subtask
  (`--parent`) inherits the parent's workspace (or backlog).
- `create --to-backlog`: creates the task in the current project's backlog (project
  registered if new). Outside a git repo: the `--scope backlog` error. With `--parent`: error
  (`--to-backlog cannot be used with --parent; subtasks follow their parent`). Allowed in the
  default scope.
- `update <id> --move-to <path>`: moves the task and all its descendants to
  that workspace (registered if new; the path resolves like `--workspace`).
  `--move-to none` moves them to the unscoped bucket. (Not `--workspace`: that
  is the global flag selecting the current workspace.) Tasks already in the target
  workspace are untouched.
- `update <id> --to-backlog`: moves the task and all its descendants to the backlog of
  the task's own project. An unscoped task goes to the current project's backlog (error
  outside a repo). The default scope allows it (the task must be in the current workspace);
  the hook does not ask. Conflicts, all exit 1 with nothing written: `--move-to`
  (`--to-backlog cannot be used with --move-to`) and `--claim`
  (`--to-backlog cannot be used with --claim; claiming moves the task into the current workspace`).
  Tasks already in that backlog are untouched.
- Claim pull-in: `update <id> --claim` on a backlog task also moves it and its subtasks into
  the current workspace (registered if new), so a task is taken with
  `tk update <id> --claim --scope backlog` (`--scope project|all` also work). Without a
  resolvable workspace it fails: `cannot claim a backlog task without a current workspace to
  move it into; use --workspace <path>`. Order inside the one transaction: fields and reparent,
  claim pull-in, then `--move-to` / `--to-backlog`.
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
  The human table also has a `(backlog)` line (same counts, `(project backlog, no workspace)`
  in the path column) before the workspaces of each project that has at least one backlog
  task of any status, projects without workspaces included. `--json` is unchanged: a backlog
  row would look like the unscoped bucket (`workspace_id: null`), so backlog counts are not
  rows of that array (web: `GET /api/backlogs`).
- `tk show <id>`: human output says `Workspace:   none (backlog of project <name>)` for a
  backlog task (`Workspace:   none` for an unscoped one).
- `tk prime`: in the default (`workspace`) scope, one line under `## Stats` when the current
  project's backlog has not-done tasks:
  `Project backlog: N open task(s) (tk list --scope backlog; take one only when the user asks: tk update <id> --claim --scope backlog)`.
  Hidden when N is 0 (done tasks do not count) and with `--scope backlog|project|all`.
  The command reference and the rules file (`tk init-rules`) say to park out-of-scope
  findings with `--to-backlog` and not to take backlog tasks unless the user asks.

JSON: Task gets two new nullable fields, `workspace_id` and `project_id`. `show --json`
gets a `workspace` object (`id`, `name`, `path`, `project_id`, `project_name`,
`missing`) or `null`, and a `project` object (`id`, `name`, `path`) or `null` (set for
workspace tasks and backlog tasks, null for unscoped). `prime --json` gets a `backlog`
object `{project_id, open, in_progress, blocked}` for the current project (zeros for an
empty backlog, `null` without a registered project, outside a repo included; present in
every scope). No existing field changes.

### Agent hook

The Claude Code plugin registers a `PreToolUse` hook (matcher `Bash`, command
`tk hook pre-tool-use 2>/dev/null || true`, in `claude-plugin/.claude-plugin/plugin.json`).
The hidden `tk hook pre-tool-use` command reads the hook JSON from stdin and, when a Bash
command runs `tk` in a way that reaches outside the current workspace, answers with
`permissionDecision: "ask"` so the user must confirm. Otherwise it prints nothing (no
opinion). It does no database or git access and never fails (errors mean no output, exit 0).

It asks when a `tk` invocation (at command position: start of command, after `;`, `&&`,
`||`, `|`, `(`, `$(`, backtick, newline, or after `VAR=value`, `env`, `command`, `exec`) has:

- `--scope backlog`, `--scope project`, `--scope all` (or `--scope=...`); `--scope workspace`
  is fine. `tk update <id> --claim --scope backlog` (taking a backlog task) therefore asks.
- `--workspace` / `--workspace=`
- `--move-to`
- `--db` / `--db=`
- a `TACKS_WORKSPACE=` / `TACKS_DB=` assignment prefixing it, or `export TACKS_WORKSPACE` /
  `export TACKS_DB` anywhere in the command

Wrappers are skipped together with their arguments before the command word:
`sudo` (flags, `-u user` etc.), `time`, `nohup`, `nice` (`-n N`), `timeout` (flags, then
DURATION), `xargs` (flags, then the command), `env` (flags such as `-i`, `-u NAME`, `--`,
and `NAME=value` pairs; `TACKS_*` assignments still trigger), plus `command` and `exec`.
`bash|sh|zsh|dash|ksh -c '<cmd>'` (also `-lc`, or `-c` after other flags) is scanned
recursively, to a depth of 5.

`--to-backlog` (on `create` and `update`) does not ask: it only parks a task in the project's
backlog, which the user reviews later.

Quoted text is not scanned, so `tk comment X "use --scope all"` does not trigger.

Caveats: the hook is a best-effort layer. It is a simple tokenizer, not a shell parser (it
does not see through scripts, aliases, variable expansion, `eval`, `find -exec`, `ssh`,
here-documents, or wrapper flags it does not know). In `auto` mode "ask" still
prompts; whether it is honored in `bypassPermissions` mode is unverified, so do not rely on it
there. Without `tk` on PATH the hook silently does nothing. The write guard above (CLI
refusing out-of-scope tasks) is the hard layer; the hook only adds a confirmation prompt.

## Deleted workspaces

No data changes. A workspace is "missing" when its path no longer exists
(checked at read time; in the web UI only for requests that render it). Its tasks stay in the project. The CLI (`show`,
`workspaces`) and the web UI mark them, and the user can move them to another
workspace, or to the project backlog, or an agent elsewhere can take them over with
`--scope project|all` when the user asks.
A missing workspace whose tasks are all done (or that has none) is archived automatically on a web page load or `tk workspaces`, unless the user restored it before (`workspaces.restored_at`). Backlog tasks never block
auto-archive (they have no workspace).

Move-open-to-backlog (web only): on the task detail of a task whose workspace is missing, a
"Move open tasks to backlog" button asks for confirmation (it shows the count) and then moves
every not-done task of that workspace, with its subtasks, to the backlog of the workspace's
project (`POST /api/workspaces/{id}/move-open-to-backlog`, below). Nothing is deleted; done
tasks stay in the workspace unless they are subtasks of a moved task. Repeating it moves nothing.

## Web UI

- Sidebar (full-page renders only, not HTMX partials): "All" at the top, then each
  project with its workspaces nested, with counts of tasks that are not done. Each shown
  project has a "Backlog" entry first among its nested items (count 0 when empty, so an empty
  backlog stays reachable); the project count includes its backlog, and "All" counts every
  backlog. A project whose workspaces are all archived stays hidden unless active or it has
  backlog tasks. The active scope is highlighted. Workspaces whose path no longer exists are shown muted
  with a "missing" label. Switching scope keeps the current view (board/tasks/epics).
  On narrow screens the sidebar moves above the content.
- Scope in the URL path, by numeric ID. One set of handlers serves all four prefixes
  (`Scope` extractor in `src/web/scope.rs`; backlog routes are recognised by the matched
  route pattern):
  - `/board`, `/tasks`, `/epics`, `/epics/{id}`: all projects.
  - `/p/{project_id}/board|tasks|epics|epics/{id}`: every workspace in one project plus its
    backlog (missing-workspace tasks included, marked on cards).
  - `/p/{project_id}/backlog/board|tasks|epics|epics/{id}`: the backlog of one project.
  - `/p/{project_id}/w/{workspace_id}/board|tasks|epics|epics/{id}`: one workspace.
  - `tasks/new` and `tasks/new/modal` exist under each prefix too (the create form
    needs the scope); `POST {prefix}/tasks` redirects back to `{prefix}/tasks`.
  - Task detail (`/tasks/{id}`), `/tasks/{id}/dep-tree` and `/epics/{id}/dep-tree`
    stay global.
- Unknown project or workspace id, non-numeric id, or a workspace that does not belong
  to the project in the URL: 404 (also for the backlog prefix).
- Links, forms, nav tabs, HTMX poll URLs and partial URLs keep the scope prefix.
- Task cards and rows (and epic rows) show the workspace name as a badge in project and All
  scopes, marked "missing" when the path is gone; backlog tasks show a "backlog" badge
  instead (title: `Backlog of project <name>`). Both are hidden in workspace and backlog
  scope: redundant. The breadcrumb of a backlog page reads `<project> / Backlog`.
- Task detail (page and modal): workspace name, path, project, "missing" marker, or
  `none (backlog of project <name>)` for a backlog task (plain "none" when unscoped), and a
  selector to move the task. Each project is an `<optgroup>` with a "Backlog" option (value
  `backlog:<project id>`) followed by its workspaces; the empty option stays "none, unscoped".
  The selector sends `PATCH /api/tasks/{id}` with `workspace_id` (`workspace_id: null` plus
  `project_id` for a backlog); descendants move too (the UI says so). Missing workspaces are
  listed but disabled as move targets. For a missing workspace the note also has the
  "Move open tasks to backlog" button (see Deleted workspaces).
- Create form: in workspace scope new tasks go to that workspace (hidden field). In
  project scope a select defaults to the project's backlog; in backlog scope there is no
  select, new tasks go to the project's backlog (hidden `backlog:<project id>`); in All
  scope it defaults to "none". The select offers "Backlog" per project plus non-missing
  workspaces. Subtasks (with a parent) always inherit the parent's workspace or backlog.
  The HTML form field `workspace_id` accepts `""` (unscoped), a workspace id or
  `backlog:<project id>`; anything else: 422 (`invalid workspace_id`).
- JSON API (additive): optional integer `project` and `workspace` query params on
  `/api/tasks`, `/api/tasks/ready`, `/api/tasks/blocked`, `/api/epics`, `/api/stats`,
  `/api/tags`, `/api/prime`. `workspace` wins when both are given; absent means all.
  `project` covers its workspaces and its backlog. `backlog=1` (or `true`; `0`/`false`
  are off) with `project` selects only that project's backlog; `backlog` without `project`,
  or together with `workspace`, is 400, as is any other `backlog` value.
  Non-integer: 400; well-formed but unknown id: 404.
  `GET /api/workspaces` returns the same rows as `tk workspaces --json` (`current` is
  always false; no backlog rows). `GET /api/backlogs` returns the backlog counts:
  `[{project_id, project_name, open, in_progress, blocked, done}]`, one row per project with
  at least one backlog task, ordered by project name then id.
  `PATCH /api/tasks/{id}` accepts `workspace_id` (integer, or `null` for
  unscoped; absent leaves it unchanged; unknown workspace: 404) and optional `project_id`:
  with `workspace_id: null` it moves the task and its subtasks to that project's backlog
  (unknown project: 404); with a workspace id it must be that workspace's project (400);
  without `workspace_id` it is 400 and nothing is changed. `POST /api/tasks`
  accepts optional `workspace_id` (ignored for subtasks; unknown workspace: 404) and
  `project_id`: without `workspace_id` it creates the task in that project's backlog (unknown
  project: 404); with one it must match the workspace's project (400). The response carries
  the stored `project_id`.
  `POST /api/workspaces/{id}/move-open-to-backlog` moves every not-done task of the workspace,
  with its subtasks, to the backlog of the workspace's project, in one transaction. Answers 200
  `{"moved": N, "project_id": P}` (`moved` counts every task whose placement changed, subtasks
  included; repeating the call gives `moved: 0`), 404 for an unknown or non-numeric id. The body
  is ignored. Like the other workspace actions it is behind the host/origin guard
  (`docs/web-security.md`): a cross-site request is rejected and moves nothing.
- Paths from the DB are only displayed and checked with `exists()`, never read or served.
