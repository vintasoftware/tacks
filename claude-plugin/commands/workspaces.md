---
description: List projects and workspaces with task counts
---

Run `tk workspaces` to see every project and workspace (git worktree) in the global database, with open, in-progress, blocked, and done counts. The current workspace is marked with `*`. Workspaces whose path no longer exists are flagged as missing.

## Usage

```bash
tk workspaces [--json]
```

## Instructions

1. Run `tk workspaces --json` using the Bash tool.
2. Present the projects and workspaces with their task counts, noting the current one and any missing paths.
3. To act on another workspace, pass the global `--workspace <path>` flag; to widen a list-type command, pass `--scope project` or `--scope all`.

## Examples

```bash
# Overview of all projects and workspaces
tk workspaces

# Machine-readable
tk workspaces --json

# List tasks across every workspace of the current project
tk list --scope project

# List tasks in the whole database
tk list --scope all

# Work against another workspace
tk --workspace /path/to/worktree ready
```

## Notes

- Scope is resolved from `--workspace <path>` / `TACKS_WORKSPACE`, otherwise from the git worktree of the current directory; outside git, tasks fall in the "unscoped" bucket.
- `--scope workspace|project|all` (default `workspace`) applies to `list`, `ready`, `prime`, `blocked`, `epic`, and `stats`.
- ID-based commands (`show`, `update`, `close`, `comment`, `dep`, `children`) work on any task regardless of scope.
- Move a task and its subtasks to another workspace with `tk update <id> --move-to <path|none>`.
