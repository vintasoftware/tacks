/// Rules content embedded at compile time.
const RULES_CONTENT: &str = r#"# Tacks Task Manager

Tacks (`tk`) is the task tracker for this project. Use it to manage tasks, track progress, and coordinate work.

## Session Start

Run `tk prime` at the start of each session to get context: backlog stats, in-progress tasks, and the ready queue.

## Command Reference

```
tk create <title> [-p priority] [-d markdown] [-t tags] [--parent id]
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
5. Blocked? Follow "When you are blocked" under Writing tasks
6. Close when done: `tk close <id> -c "summary"`

## Writing tasks

Descriptions (`-d`), notes (`--notes`) and comments are markdown, rendered in the web UI.

Write each description for the user, who reviews tasks in the web UI and makes decisions from them without your context. A good description lets them decide without opening the code.

- Title: one short line, plain text, imperative ("Add OAuth login").
- **Context**: why the task exists, what you found.
- **Approach**: the plan, and rejected alternatives with a one-line reason, when relevant.
- **Done when**: checklist (`- [ ] ...`).
- **Open questions**: if any.
- Use `code` for paths, commands and identifiers, fenced blocks for snippets.
- Pass multi-line markdown with a quoted heredoc, never one long line:

  ```bash
  tk create "Add OAuth login" -p 1 -d "$(cat <<'EOF'
  ## Context
  Users can only sign in with a password; `src/auth.rs` has no provider hook.

  ## Approach
  Use the `oauth2` crate. Rejected: hand-rolled flow (more code, same result).

  ## Done when
  - [ ] Google OAuth button on `/login`
  - [ ] Tests in `tests/auth.rs`

  ## Open questions
  - Keep password login for existing users?
  EOF
  )"
  ```

Keep the description current: when you learn something that changes the plan or scope, run `tk update <id> -d ...`. It replaces the whole description, so pass the full text (read it first with `tk show <id> --json`). Notes (`--notes`) are for short-lived working state; decisions and findings go in the description.

### When you are blocked

This is the most important case. The user cannot help unless the task tells them what you need.

1. `tk update <id> -s blocked`
2. Add a `## Blocked` section at the top of the description (full text, see above) with: what blocks you (be specific: error, missing access, unclear requirement), what you already tried, the decision you need from the user, the options with their trade-offs, and your recommendation.
3. Tell the user in the session too, then stop working on that task.

The user answers by commenting on the task. Act on the comment, remove or update the `## Blocked` section, and set the status back to `in_progress`.

```markdown
## Blocked
**Blocker:** the `tasks.due` migration fails on prod-sized data: `ALTER TABLE` locks the table for ~40s.
**Tried:** a local run on a 2M-row copy; adding the column as nullable (same lock).

**Decision needed:** which migration strategy?
- A. In-place `ALTER TABLE` in a maintenance window. Simple, but ~40s of downtime.
- B. Shadow table plus backfill, then swap. No downtime, about a day more work.

**Recommendation:** B. The table is written all day, so 40s of downtime fails requests.
```

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

#[cfg(test)]
mod tests {
    use super::RULES_CONTENT;

    #[test]
    fn test_rules_describe_markdown_writing() {
        assert!(RULES_CONTENT.contains("## Writing tasks"));
        assert!(RULES_CONTENT.contains("markdown"));
        assert!(RULES_CONTENT.contains("## Blocked"));
        assert!(RULES_CONTENT.contains("recommendation"));
    }
}
