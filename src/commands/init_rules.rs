/// Rules content embedded at compile time.
const RULES_CONTENT: &str = r#"# Tacks Task Manager

Tacks (`tk`) is the task tracker for this project. Use it to manage tasks, track progress, and coordinate work.

## Session Start

Run `tk prime` at the start of each session to get context: backlog stats, in-progress tasks, and the ready queue.

## Command Reference

```
tk create <title> [-p priority] [-d desc] [-t tags] [--parent id]
tk list [-s status] [-p pri] [-t tag] [--parent id]
tk ready [--limit N]
tk show <id>
tk update <id> [fields...] [--claim] [--notes text] [--parent id|none]
tk close <id> [-c comment] [-r reason] [--force]
tk dep add|remove <child> <parent>
tk comment <id> <body> [--author name]
tk children <id>
tk epic
tk blocked
tk stats [--oneline]
tk prime
tk workspaces
```

All commands support `--json` for machine-readable output.

Tasks are scoped automatically to the current git worktree (workspace) in one global database (`~/.tacks/tacks.db`). 
## Workspace boundaries

- Work only on tasks of your current workspace. Write commands (`update`, `close`, `comment`, `dep`, `create --parent`) refuse tasks of other workspaces.
- Never use `--scope project|all`, `--workspace`, `TACKS_WORKSPACE`, `--db`/`TACKS_DB` or `--move-to` unless the user explicitly asked for that task or workspace.
- If a task you need belongs elsewhere, tell the user instead of acting on it.
- Reading other workspaces' tasks with `tk show <id>` is fine.

## Workflow

1. Run `tk prime` to orient
2. Pick a task: `tk ready --limit 1`
3. Claim it: `tk update <id> --claim`
4. Add working notes: `tk update <id> --notes "context"`
5. Close when done: `tk close <id> -c "summary"`

## User feedback

- The user can comment on tasks in the web UI. Those comments appear in `tk prime` ("User feedback awaiting reply"), in `tk show` (marked "awaiting reply"), and as hook notices during the session.
- Treat them as instructions from the user for that task: address them, then reply with `tk comment <id> "..."`.
- `tk close` refuses while a task has unanswered user comments. Use `--force` only if the user agreed.
- Never use `--author user` (or impersonate the user in any way): that author is reserved for the human user, and your replies must use the default author.

## Conventions

- Task IDs use `tk-XXXX` format, subtasks use `tk-XXXX.N`
- Priority: P0 (critical) through P3 (low), P4 (trivial), default P2
- Status: open → in_progress → done (or blocked)
- Close reasons: done, duplicate, absorbed, stale, superseded
- Tags are comma-separated: `-t "backend,api"`
- The `epic` tag is auto-added when a task gets children
- Use `--json` when parsing output programmatically
- Use `tk stats --oneline` for compact status checks
"#;

/// Install a Claude Code rules file that teaches Claude how to use `tk`.
///
/// Writes `tacks.md` to either `.claude/rules/` (project, default) or
/// `~/.claude/rules/` (global, when `--global` is passed).
pub fn run(global: bool) -> Result<(), String> {
    let rules_dir = if global {
        let home =
            std::env::var("HOME").map_err(|_| "HOME environment variable not set".to_string())?;
        std::path::PathBuf::from(home).join(".claude").join("rules")
    } else {
        std::env::current_dir()
            .map_err(|e| format!("cannot determine current directory: {e}"))?
            .join(".claude")
            .join("rules")
    };

    std::fs::create_dir_all(&rules_dir)
        .map_err(|e| format!("cannot create directory {}: {e}", rules_dir.display()))?;

    let output_path = rules_dir.join("tacks.md");

    std::fs::write(&output_path, RULES_CONTENT)
        .map_err(|e| format!("cannot write {}: {e}", output_path.display()))?;

    println!("Wrote Claude Code rules to {}", output_path.display());
    println!();
    println!("Contents:");
    println!("  - Session bootstrapping (tk prime)");
    println!("  - Command reference (15 commands)");
    println!("  - Workflow guide (claim → work → close)");
    println!("  - Conventions (IDs, priorities, statuses, tags)");

    Ok(())
}
