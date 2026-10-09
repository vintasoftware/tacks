---
description: Initialize the global tacks database and set the task ID prefix
---

Run `tk init` to initialize the tacks database. Tacks uses one global SQLite database (`~/.tacks/tacks.db`) shared by every project, and scopes tasks automatically per git worktree ("workspace"), so there is no per-directory setup.

## Usage

```bash
tk init [--prefix <prefix>]
```

## Instructions

1. Run `tk init` using the Bash tool (only needed once, or to change the ID prefix).
2. Confirm success by checking the output message.
3. The database is ready. Tasks you create from inside a git repository are registered to that repository's workspace automatically; see `/tacks:workspaces`.

## Notes

- The database lives at `~/.tacks/tacks.db` and is created on first use, so `tk init` is optional unless you want a custom prefix.
- Workspace scoping is automatic: the current git worktree root is the workspace, and its repository is the project. Use `--workspace <path>` (or `TACKS_WORKSPACE`) to target another one.
- The ID prefix is stored in the database config, so it is shared by all projects and workspaces.
- A `.tacks/tacks.db` in the current directory is no longer used.
- To use a different database path, pass `--db <path>` or set the `TACKS_DB` environment variable.
- Running `tk init` again is safe.
