# tacks

Lightweight task manager for AI coding agents. Local-only, single-binary, SQLite-backed.

This is the [vintasoftware](https://github.com/vintasoftware/tacks) fork of [srmccray/tacks](https://github.com/srmccray/tacks). It adds **workspace scoping**: one global database, tasks scoped per git worktree, and one web UI for all projects and workspaces. See [Workspace scoping](#workspace-scoping).

## Install

The upstream installers, release binaries and the `tacks` crate on crates.io do **not** include the fork changes. Install from this repository with cargo.

### 1. Rust toolchain

You need `cargo` on your `PATH`. With [rustup](https://rustup.rs) it is in `~/.cargo/bin`. If you installed rustup with Homebrew, the toolchain proxies are in `/opt/homebrew/opt/rustup/bin`.

Add both to your shell config. Use `~/.zshenv` (not only `~/.zshrc`) so that non-interactive shells, such as Claude Code hooks, also find `tk`:

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
```

### 2. The `tk` binary

```bash
cargo install --git https://github.com/vintasoftware/tacks --locked
tk --version
tk workspaces --help   # if this command exists, you have the fork
```

To update, run the same `cargo install` command again with `--force`.

From a local clone:

```bash
git clone https://github.com/vintasoftware/tacks.git
cd tacks
cargo install --path . --locked
```

## Set up your agents (Claude Code)

Do these steps once per machine. You do not need to do anything per repository or per worktree.

1. Install the `tk` binary (see [Install](#install)).
2. Install the Claude Code plugin. In Claude Code:

   ```
   /plugin marketplace add vintasoftware/tacks
   /plugin install tacks@tacks-marketplace
   ```

   The plugin runs `tk prime` at session start and before compaction, and adds `/tacks:*` slash commands.
3. Install the global rules file:

   ```bash
   tk init-rules --global   # writes ~/.claude/rules/tacks.md
   ```

   This file teaches every agent, in every repository, the `tk` commands, the workflow and the scoping rules. You need it because `tk prime` is silent in a workspace that has no tasks yet. Without the rules file, an agent in a new worktree gets no tacks context.

   To use tacks only in some repositories, run `tk init-rules` (without `--global`) in each of them instead. This writes `.claude/rules/tacks.md` in the repository.
4. Start a new Claude Code session (or run `/reload-plugins`). Sessions that are already open do not load the new rules.

Each agent now sees only the tasks of its own worktree. To see the full board, run `tk serve`.

## Quick start

You do not need `tk init`. The database `~/.tacks/tacks.db` is created on first use, and the current git worktree is detected automatically.

```bash
tk create "Implement auth" -p 1      # create a P1 task in this worktree
tk create "Write tests" -d "Unit and integration tests for auth module"
tk list                              # show open tasks
tk ready                             # tasks with no blockers
tk update <id> --claim               # claim a task (sets in_progress + assignee)
tk update <id> --parent <epic-id>    # reparent task under an epic
tk update <id> --parent none         # promote subtask to top-level
tk close <id> -c "Done"              # close with comment

tk list --scope project              # tasks of every worktree of this repository
tk list --scope all                  # every task in the database
tk workspaces                        # projects and workspaces with task counts
tk update <id> --move-to ../other-worktree   # move a task (and its subtasks)
```

Use `tk init --prefix <prefix>` only to change the task ID prefix. The prefix applies to the whole global database.

## Commands

| Command | Description |
|---------|-------------|
| `tk init` | Initialize the tacks database (global by default, `~/.tacks/tacks.db`) |
| `tk create <title>` | Create a task (`-p` priority, `-d` description, `-t` tags, `--parent` subtask; sets the current workspace) |
| `tk list` | List open tasks (`-a` all, `-s` status, `-p` priority, `-t` tag, `--parent` filter) |
| `tk ready` | Show tasks with no open blockers (`--limit N`) |
| `tk show <id>` | Task details with blockers, dependents, comments, notes |
| `tk update <id>` | Update fields (`--claim`, `--notes`, `--parent`, `--move-to <path|none>`, `-d`, `-p`, `-t`, `-s`) |
| `tk close <id>` | Close a task (`-c` comment, `-r` reason, `--force` to bypass subtask guard) |
| `tk dep add <child> <parent>` | Add a dependency (cycle-checked) |
| `tk dep remove <child> <parent>` | Remove a dependency |
| `tk comment <id> <body>` | Add a comment |
| `tk children <id>` | List subtasks of a task |
| `tk epic` | Show epic progress (completion stats) |
| `tk blocked` | List tasks blocked by open dependencies |
| `tk stats` | Backlog overview (`--oneline` for compact output) |
| `tk workspaces` | List projects and workspaces with task counts and a `missing` marker |
| `tk prime` | AI context output: stats + in-progress + ready queue |
| `tk init-rules` | Install Claude Code rules file (`--global` for all projects) |
| `tk serve` | Start web UI server (`--port` to set port, default 3000) |

All commands support `--json` for machine-readable output.

Global flags: `--db <path>` (`TACKS_DB`), `--workspace <path>` (`TACKS_WORKSPACE`), `--scope workspace|project|all` (default `workspace`).

## Web UI

Tacks includes a built-in web interface. One instance shows every project and workspace in the global database. Start it from any directory:

```bash
tk serve              # http://localhost:3000
tk serve --port 8080  # custom port
```

The sidebar shows **All**, then each project with its workspaces. The scope is in the URL:

| URL | Shows |
|-----|-------|
| `/board`, `/tasks`, `/epics` | All projects |
| `/p/<project-id>/board` | Every workspace of one project |
| `/p/<project-id>/w/<workspace-id>/board` | One workspace |

A workspace whose directory was deleted shows a "missing" marker. Its tasks stay in the project, and you can move them to another workspace from the task detail.

### Board view

Kanban board with drag-and-drop status changes, column counts, and multi-select filters.

![Board view](docs/images/board.png)

### Task list

Sortable table with inline editing — click any field to edit in place.

![Task list](docs/images/list.png)

### Features

- **Kanban board** with drag-and-drop between status columns
- **Inline editing** on both board and list views
- **Multi-select filters** for status, priority, epic, and tags
- **Dark mode** by default
- **Keyboard shortcuts** — press `?` for the full list
- **Live polling** — changes from CLI or other sessions appear automatically
- **Epic detail pages** with subtask progress and board view
- **Responsive layout** for different screen sizes

## Claude Code Plugin

Tacks ships a Claude Code plugin that wires `tk` commands into slash commands and session hooks.

### Install from marketplace

```bash
/plugin marketplace add vintasoftware/tacks
/plugin install tacks@tacks-marketplace
```

### Manual install from local clone

```bash
git clone https://github.com/vintasoftware/tacks.git
cd tacks

# Option 1: Session-scoped (for testing)
claude --plugin-dir ./claude-plugin

# Option 2: Persistent install
/plugin marketplace add ./claude-plugin
/plugin install tacks@tacks-marketplace
```

**Prerequisite**: the `tk` binary must be installed and on your `PATH` (see Install section above).

### What the plugin provides

- **Slash commands** for all `tk` operations: `/tacks:create`, `/tacks:list`, `/tacks:ready`, `/tacks:show`, `/tacks:update`, `/tacks:close`, `/tacks:dep`, `/tacks:comment`, `/tacks:children`, `/tacks:epic`, `/tacks:blocked`, `/tacks:stats`, `/tacks:prime`, `/tacks:init`, `/tacks:workspaces`
- **SessionStart hook** that auto-loads backlog context via `tk prime` for the current workspace (silent in workspaces that have no tasks; install the rules file too, see [Set up your agents](#set-up-your-agents-claude-code))
- **PreCompact hook** that re-runs `tk prime` before context compaction to preserve backlog state
- **PreToolUse hook** that asks you to confirm any Bash `tk` command reaching outside the current workspace (`--scope project|all`, `--workspace`, `--move-to`, `--db`, `TACKS_WORKSPACE`/`TACKS_DB`); see [workspace scoping](docs/workspace-scoping.md#agent-hook)
- **Task agent** (`@task-agent`) for autonomous work discovery: finds ready tasks, claims them, executes, files discoveries, and closes on completion

## Designed for agents

Tacks is built to be consumed by AI coding agents like Claude Code:

- **`tk init-rules`** installs a Claude Code rules file that teaches Claude how to use `tk` — command reference, workflow, and conventions. Use `--global` to install for all projects.
- **`tk prime --json`** gives agents a snapshot of project state: what's in progress, what's ready, backlog stats
- **`tk ready --limit 1`** picks the next task for an agent to work on
- **`--json` on every command** means agents can parse output reliably
- **Hash-based IDs** (`tk-a1b2`) are short and unambiguous
- **Dependency tracking** with cycle detection prevents agents from picking up blocked work
- **Subtask hierarchies** with auto-tagging: creating a subtask automatically tags the parent as an epic

## Key concepts

- **Priority**: 0-4 (0 = critical, 4 = backlog)
- **Close reasons**: `done`, `duplicate`, `absorbed`, `stale`, `superseded`
- **Notes vs comments**: Notes are mutable working context (overwritten). Comments are append-only history.
- **Close guard**: Can't close a task with open subtasks unless you use `--force`
- **Tags over types**: Epic, bug, etc. are tags, not a type system. The `epic` tag is auto-added when you create a subtask.

## Stability contract

Tacks is consumed by downstream tools (e.g., [Tackline](https://github.com/steveyegge/tackline)) that call `tk` commands in hooks, skills, and agent definitions. The CLI interface is stable and all changes are backwards-compatible:

- **Commands and flags are permanent.** No existing command, subcommand, or flag will be removed or renamed. New flags are always optional.
- **JSON output is frozen.** Fields in `--json` output will not be removed or have their types changed. New fields may be added.
- **Enums are append-only.** Status values (`open`, `in_progress`, `done`, `blocked`) and close reasons (`done`, `duplicate`, `absorbed`, `stale`, `superseded`) will not be removed. New values may be added.
- **DB schema is additive.** Existing columns and tables are never removed or renamed. New columns are nullable or defaulted.
- **Exit codes are stable.** 0 for success, 1 for error.
- **ID format is stable.** `tk-XXXX` for tasks, `tk-XXXX.N` for subtasks.

If a breaking change is ever necessary, it will be flagged with a `BREAKING:` commit prefix and include a migration path.

## Storage

Tacks uses SQLite (bundled, no system dependency) in one global database at `~/.tacks/tacks.db`. Override with `--db` or the `TACKS_DB` environment variable. A `.tacks/tacks.db` in the current directory is no longer used.

### Workspace scoping

Tasks are scoped per git worktree ("workspace"); worktrees of the same repository form a "project". The workspace comes from `--workspace` / `TACKS_WORKSPACE`, else `git rev-parse --show-toplevel` from the current directory, else the "unscoped" bucket (existing tasks and tasks created outside a repo).

- `list`, `ready`, `blocked`, `epic`, `stats`, `prime` show the current workspace only. Use `--scope project` for every workspace of the repository, `--scope all` for everything. (`list -a` still means "include closed".)
- `create` records the current workspace; subtasks inherit the parent's workspace.
- Write guard: `update`, `close`, `comment`, `dep add|remove` and `create --parent` refuse (exit 1) tasks outside the current workspace, so agents stay in their lane. `--scope project|all` lifts the guard; use it only when asked. `show` and `children` read any task, and the web UI/API are unrestricted.
- `tk update <id> --move-to <path|none>` moves a task and its subtasks to another workspace (or the unscoped bucket).
- `tk prime` is silent when the current workspace has never used tacks.

See `docs/workspace-scoping.md` for details.

No sync, no network calls. Everything stays local (git is only invoked to detect the worktree).

## License

MIT
