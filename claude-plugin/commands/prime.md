---
description: Get AI context output — stats, in-progress tasks, and the ready queue
---

Run `tk prime` to get a compact, AI-optimized snapshot of the current backlog state. This is automatically run at session start and before context compaction via hooks.

## Usage

```bash
tk prime [--scope project|all] [--workspace <path>] [--json]
```

## Instructions

1. Run `tk prime --json` using the Bash tool.
2. Parse the output to orient yourself:
   - **stats**: How many tasks are open, in-progress, done?
   - **in_progress**: What is currently being worked on?
   - **ready**: What unblocked tasks are available to pick up next?
   - **user_feedback**: Comments the user left on tasks that nobody has answered yet. Treat them as instructions for that task: address them, then reply with `tk comment <id> "..."`.
3. Use this information to make decisions about what to work on and report status to the user.

## Examples

```bash
# Get AI context (human-readable)
tk prime

# Get AI context (JSON for parsing)
tk prime --json
```

## Notes

- `tk prime` is the recommended first command when starting a session — it gives you full situational awareness in one call.
- The output combines: backlog stats, currently in-progress tasks, and the top ready (unblocked) tasks.
- This command is automatically invoked by the plugin hooks at `SessionStart` and `PreCompact` so context is never lost during compaction.
- `tk prime` prints nothing (exit 0) when the current workspace has never used tacks, so hooks stay silent in unrelated repositories.
- When the user has unanswered comments, `tk prime` prints a "User feedback awaiting reply" section first. Each comment is shown once by prime or the PostToolUse hook; `tk show <id>` always lists it until you reply.
- For more detail on any specific task, follow up with `tk show <id>`.
