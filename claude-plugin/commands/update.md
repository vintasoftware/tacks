---
description: Update task fields, claim a task, set working notes, or reparent under an epic
---

Run `tk update <id>` to modify task properties. Use `--claim` to take ownership and move the task to in_progress. Use `--parent` to reparent a task under an epic or promote it to top-level.

## Usage

```bash
tk update <id> [--title <title>] [--priority <n>] [--status <status>]
               [--tags <tags>] [--assignee <name>] [--claim] [--notes <text>]
               [--parent <id>|none] [--move-to <path>|none]
```

## Flags

| Flag | Description |
|------|-------------|
| `--title <title>` | New title for the task |
| `--priority <n>` | New priority level (0 = critical, 1 = high, 2 = medium, 3 = low, 4 = trivial) |
| `--status <status>` | New status: `open`, `in_progress`, `done`, `blocked` |
| `--tags <tags>` | Replace tags with comma-separated list |
| `--assignee <name>` | Assign to a person or agent |
| `--claim` | Set status to `in_progress` and assignee to current user |
| `--notes <text>` | Set mutable working notes (overwrites previous notes) |
| `--parent <id>` | Move task under a parent/epic |
| `--parent none` | Promote subtask to top-level task |
| `--move-to <path>` | Move the task and its subtasks to another workspace (worktree path); `none` makes it unscoped |

## Instructions

1. Identify the task ID and which fields need updating.
2. Run `tk update <id> <flags> --json` using the Bash tool.
3. Confirm the update to the user.

## Examples

```bash
# Claim a task before starting work
tk update tk-a1b2 --claim

# Update priority and tags
tk update tk-a1b2 --priority 1 --tags backend,urgent

# Set working notes (overwrites previous notes)
tk update tk-a1b2 --notes "Investigated root cause — issue is in auth middleware"

# Change status to blocked
tk update tk-a1b2 --status blocked

# Reparent a task under an epic
tk update tk-a1b2 --parent tk-c3d4

# Move a task (and its subtasks) to another workspace
tk update tk-a1b2 --move-to /path/to/other/worktree

# Promote a subtask to top-level
tk update tk-c3d4.1 --parent none
```

## Notes

- `--claim` is the standard way to start working on a task: it sets `status=in_progress` and records the assignee.
- `--notes` **overwrites** previous notes — it is mutable working context, not a log. Use `tk comment` to append to the activity log instead.
- `--move-to` registers the target workspace if it is new; see `/tacks:workspaces` for listing workspaces.
- `update` only works on tasks of the current workspace (for `--parent`, both tasks; for `--move-to`, the task being moved). A task elsewhere fails with an error naming its workspace. Do not rerun with `--scope project|all`, and do not use `--move-to`, `--workspace` or `TACKS_WORKSPACE`, unless the user explicitly asked for that task or workspace; otherwise tell the user.
- Multiple flags can be combined in a single `tk update` call.
- Use `--json` to parse the updated task back.
