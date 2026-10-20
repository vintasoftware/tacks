use std::path::Path;

use crate::db::{BacklogTaskCounts, Database};
use crate::models::{PendingComment, Task};
use crate::scope::{Scope, ScopeMode};

const READY_LIMIT: u32 = 5;

const COMMAND_REFERENCE: &[&str] = &[
    "tk create <title> [-p priority] [-d markdown] [-t tags] [--parent id] [--to-backlog]",
    "tk list [-s status] [-p pri] [-t tag] [--json]",
    "tk ready [--limit N] [--json]",
    "tk show <id> [--json]",
    "tk update <id> [fields...] [--claim] [--to-backlog]",
    "tk close <id> [-c comment] (refused while user comments are unanswered)",
    "tk dep add|remove <child> <parent>",
    "tk comment <id> <body> (reply to user feedback with it)",
    "Descriptions, notes and comments are markdown (rendered in the web UI): use checklists, code spans and multi-line text via heredoc",
    "Write descriptions for the user to review and decide; when blocked, set -s blocked and add a '## Blocked' section (cause, tried, decision needed, options, recommendation)",
    "tk stats [--oneline] [--json]",
    "tk workspaces [--json]",
    "Scope: list/ready/stats/blocked/epic/prime cover the current workspace; add --scope backlog|project|all to widen (reads only; do not use it for writes unless the user asked)",
    "Backlog: park work that does not belong to this workspace (out-of-scope findings) with tk create <title> --to-backlog or tk update <id> --to-backlog; take a backlog task only when the user asks (tk update <id> --claim --scope backlog)",
];

/// Run the `tk prime` command.
///
/// Outputs an AI-optimized context summary composed of stats, in-progress tasks,
/// and the ready queue for the current scope. Exits silently when the database does not
/// exist, the current workspace is not registered, or the scope cannot be resolved.
/// Outside a git repository it prints the unscoped bucket.
pub fn run(db_path: &Path, scope: &Scope, json: bool) -> Result<(), String> {
    // Silent exit when no tacks database is present — hooks call this on every
    // session, so it must be a no-op in projects that don't use tacks.
    if !db_path.exists() {
        return Ok(());
    }

    let db = Database::open(db_path)?;

    // Also silent when the current workspace never used tacks.
    // A scope that cannot be resolved (bad --workspace / TACKS_WORKSPACE) is also silent:
    // exit status 0, no output.
    if scope.mode == ScopeMode::Workspace {
        match scope.resolve() {
            Ok(Some(_)) => match scope.registered_workspace(&db) {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return Ok(()),
            },
            Ok(None) => {}
            Err(_) => return Ok(()),
        }
    }
    let Ok(filter) = scope.filter(&db) else {
        return Ok(());
    };

    let by_status = db.task_count_by_status(&filter)?;
    let in_progress = db.list_tasks(
        false,
        Some("in_progress"),
        None,
        None,
        None,
        None,
        None,
        &filter,
    )?;
    let ready = db.get_ready_tasks(Some(READY_LIMIT), &filter)?;
    let feedback = db.pending_user_comments(&filter)?;
    let backlog = backlog_counts(&db, scope)?;

    let result = if json {
        print_json(
            &by_status,
            &in_progress,
            &ready,
            &feedback,
            backlog.as_ref(),
        )
    } else {
        // The backlog line only appears in the default workspace scope: with
        // `--scope backlog|project|all` the backlog tasks are already listed (or it
        // would be redundant).
        let backlog_line = if scope.mode == ScopeMode::Workspace {
            backlog.as_ref().and_then(backlog_line)
        } else {
            None
        };
        print_markdown(
            &by_status,
            &in_progress,
            &ready,
            &feedback,
            backlog_line.as_deref(),
        )
    };

    // The feedback is now in the agent's context: stop the hook from repeating it.
    // A failure here must not fail prime (worst case the hook repeats it once).
    if result.is_ok() {
        let _ = db.claim_undelivered_pending_user_comments(&filter, |_| true);
    }
    result
}

/// Backlog counts of the current project (zeros when its backlog is empty).
///
/// `None` when there is no registered current project (outside git, or never registered).
fn backlog_counts(db: &Database, scope: &Scope) -> Result<Option<BacklogTaskCounts>, String> {
    let Some(project) = scope.current_project(db)? else {
        return Ok(None);
    };
    let counts = db
        .backlog_task_counts()?
        .into_iter()
        .find(|c| c.project_id == project.id);
    Ok(Some(counts.unwrap_or(BacklogTaskCounts {
        project_id: project.id,
        open: 0,
        in_progress: 0,
        blocked: 0,
        done: 0,
    })))
}

/// The one-line backlog hint, or `None` when the backlog has no tasks that are not done.
fn backlog_line(counts: &BacklogTaskCounts) -> Option<String> {
    let n = counts.open + counts.in_progress + counts.blocked;
    if n == 0 {
        return None;
    }
    let noun = if n == 1 { "task" } else { "tasks" };
    Some(format!(
        "Project backlog: {n} open {noun} (tk list --scope backlog; take one only when the user asks: tk update <id> --claim --scope backlog)"
    ))
}

fn print_markdown(
    by_status: &[(String, i64)],
    in_progress: &[Task],
    ready: &[Task],
    feedback: &[PendingComment],
    backlog_line: Option<&str>,
) -> Result<(), String> {
    println!("# Tacks: Project Status");

    if !feedback.is_empty() {
        println!();
        println!("## User feedback awaiting reply");
        println!(
            "The user commented on these tasks. Treat each comment as an instruction for that task: address it, then reply with `tk comment <id> \"...\"`."
        );
        for p in feedback {
            println!(
                "- {}: {} ({})",
                p.task_id,
                p.task_title,
                p.comment.created_at.format("%Y-%m-%d %H:%M")
            );
            println!(
                "    \"{}\"",
                super::truncate_chars(&p.comment.body.replace('\n', " "), 300)
            );
        }
    }

    // Stats section
    println!();
    println!("## Stats");
    if by_status.is_empty() {
        println!("no tasks");
    } else {
        let parts: Vec<String> = by_status.iter().map(|(s, c)| format!("{c} {s}")).collect();
        println!("{}", parts.join(", "));
    }
    if let Some(line) = backlog_line {
        println!("{line}");
    }

    // In Progress section
    println!();
    println!("## In Progress");
    if in_progress.is_empty() {
        println!("none");
    } else {
        for task in in_progress {
            let mut line = format!("- {}: {} [P{}]", task.id, task.title, task.priority);
            if let Some(assignee) = &task.assignee {
                line.push_str(&format!(" (assigned: {assignee})"));
            }
            println!("{line}");
        }
    }

    // Ready section
    println!();
    println!("## Ready (next {READY_LIMIT})");
    if ready.is_empty() {
        println!("none");
    } else {
        for task in ready {
            println!("- {}: {} [P{}]", task.id, task.title, task.priority);
        }
    }

    // Command Reference section
    println!();
    println!("## Command Reference");
    for cmd in COMMAND_REFERENCE {
        println!("{cmd}");
    }

    Ok(())
}

fn print_json(
    by_status: &[(String, i64)],
    in_progress: &[Task],
    ready: &[Task],
    feedback: &[PendingComment],
    backlog: Option<&BacklogTaskCounts>,
) -> Result<(), String> {
    // Build a stats object with the four canonical statuses always present.
    let mut stats = serde_json::Map::new();
    let canonical = ["open", "in_progress", "blocked", "done"];
    for key in canonical {
        stats.insert(key.to_string(), serde_json::Value::Number(0.into()));
    }
    for (status, count) in by_status {
        stats.insert(status.clone(), serde_json::Value::Number((*count).into()));
    }

    let cmd_ref: Vec<serde_json::Value> = COMMAND_REFERENCE
        .iter()
        .map(|s| serde_json::Value::String(s.to_string()))
        .collect();

    // Additive key: user comments awaiting an agent reply (empty array when none).
    let user_feedback: Vec<serde_json::Value> = feedback
        .iter()
        .map(|p| {
            serde_json::json!({
                "task_id": p.task_id,
                "task_title": p.task_title,
                "comment_id": p.comment.id,
                "body": p.comment.body,
                "created_at": p.comment.created_at,
            })
        })
        .collect();

    // Additive key: backlog counts of the current project, null without a registered project.
    let backlog = backlog.map(|c| {
        serde_json::json!({
            "project_id": c.project_id,
            "open": c.open,
            "in_progress": c.in_progress,
            "blocked": c.blocked,
        })
    });

    let out = serde_json::json!({
        "backlog": backlog,
        "user_feedback": user_feedback,
        "stats": stats,
        "in_progress": in_progress,
        "ready": ready,
        "command_reference": cmd_ref,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| format!("json error: {e}"))?
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BacklogTaskCounts, backlog_line};
    use assert_cmd::Command;
    use serde_json::Value;
    use tempfile::TempDir;

    fn tk(tmp: &TempDir) -> Command {
        let db_path = tmp.path().join("tacks.db");
        let mut cmd = Command::cargo_bin("tk").expect("tk binary not found");
        cmd.env("TACKS_DB", &db_path);
        cmd
    }

    fn init_db(tmp: &TempDir) {
        tk(tmp).args(["init"]).assert().success();
    }

    #[test]
    fn test_prime_silent_exit_when_no_db() {
        // prime should exit 0 silently when the database does not exist
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("no_such.db");
        Command::cargo_bin("tk")
            .unwrap()
            .env("TACKS_DB", &db_path)
            .args(["prime"])
            .assert()
            .success()
            .stdout("");
    }

    #[test]
    fn test_prime_silent_when_workspace_not_registered() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);
        tk(&tmp).args(["prime"]).assert().success().stdout("");
        tk(&tmp)
            .args(["--scope", "all", "prime"])
            .assert()
            .success()
            .stdout(predicates::str::contains("# Tacks: Project Status"));
    }

    #[test]
    fn test_prime_markdown_empty_db() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);

        let output = tk(&tmp).args(["--scope", "all", "prime"]).output().unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("# Tacks: Project Status"));
        assert!(stdout.contains("## Stats"));
        assert!(stdout.contains("## In Progress"));
        assert!(stdout.contains("## Ready"));
        assert!(stdout.contains("## Command Reference"));
        assert!(stdout.contains("tk create"));
    }

    #[test]
    fn test_prime_json_empty_db() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);

        let output = tk(&tmp)
            .args(["--json", "--scope", "all", "prime"])
            .output()
            .unwrap();

        assert!(output.status.success());
        let json: Value =
            serde_json::from_slice(&output.stdout).expect("prime --json output is not valid JSON");

        // All four canonical status keys are present
        assert!(json["stats"]["open"].is_number());
        assert!(json["stats"]["in_progress"].is_number());
        assert!(json["stats"]["blocked"].is_number());
        assert!(json["stats"]["done"].is_number());

        assert!(json["in_progress"].is_array());
        assert!(json["ready"].is_array());
        assert!(json["command_reference"].is_array());
        assert!(!json["command_reference"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_prime_shows_in_progress_tasks() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);

        // Create a task and claim it (sets to in_progress)
        let create_out = tk(&tmp)
            .args(["--json", "create", "My in-progress task", "-p", "1"])
            .output()
            .unwrap();
        let create_json: Value = serde_json::from_slice(&create_out.stdout).unwrap();
        let task_id = create_json["id"].as_str().unwrap().to_string();

        tk(&tmp)
            .args(["update", &task_id, "--claim"])
            .assert()
            .success();

        let output = tk(&tmp).args(["prime"]).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("My in-progress task"),
            "expected in-progress task in prime output; got:\n{stdout}"
        );
    }

    #[test]
    fn test_prime_shows_ready_tasks() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);

        tk(&tmp)
            .args(["create", "Ready task", "-p", "2"])
            .assert()
            .success();

        let output = tk(&tmp).args(["prime"]).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Ready task"),
            "expected ready task in prime output; got:\n{stdout}"
        );
    }

    #[test]
    fn test_prime_json_in_progress_and_ready() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);

        // Create a ready task (open, no blockers)
        tk(&tmp).args(["create", "Ready item"]).assert().success();

        // Create and claim a task so it becomes in_progress
        let create_out = tk(&tmp)
            .args(["--json", "create", "Working item"])
            .output()
            .unwrap();
        let id: Value = serde_json::from_slice(&create_out.stdout).unwrap();
        let task_id = id["id"].as_str().unwrap().to_string();
        tk(&tmp)
            .args(["update", &task_id, "--claim"])
            .assert()
            .success();

        let out = tk(&tmp).args(["--json", "prime"]).output().unwrap();
        let json: Value = serde_json::from_slice(&out.stdout).unwrap();

        // in_progress array should contain "Working item"
        let ip = json["in_progress"].as_array().unwrap();
        assert!(
            ip.iter()
                .any(|t| t["title"].as_str() == Some("Working item")),
            "expected 'Working item' in in_progress; got {ip:?}"
        );

        // stats should show 1 in_progress
        assert_eq!(json["stats"]["in_progress"], 1);
    }

    /// A git repository in a temp dir, with `tk` calls run from inside it.
    fn repo(tmp: &TempDir) -> std::path::PathBuf {
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git init failed");
        dir
    }

    fn tk_in(tmp: &TempDir, dir: &std::path::Path, args: &[&str]) -> std::process::Output {
        tk(tmp).current_dir(dir).args(args).output().unwrap()
    }

    #[test]
    fn test_backlog_line_counts_and_plural() {
        let mut c = BacklogTaskCounts {
            project_id: 1,
            open: 0,
            in_progress: 0,
            blocked: 0,
            done: 4,
        };
        assert_eq!(backlog_line(&c), None, "done tasks do not count");
        c.open = 1;
        let one = backlog_line(&c).unwrap();
        assert!(one.starts_with("Project backlog: 1 open task ("), "{one}");
        c.blocked = 2;
        let three = backlog_line(&c).unwrap();
        assert!(
            three.starts_with("Project backlog: 3 open tasks (tk list --scope backlog;"),
            "{three}"
        );
        assert!(three.contains("tk update <id> --claim --scope backlog"));
    }

    #[test]
    fn test_prime_backlog_line_and_json() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);
        let dir = repo(&tmp);

        // register the workspace; empty backlog: no line, zero counts
        assert!(tk_in(&tmp, &dir, &["create", "Own task"]).status.success());
        let out = tk_in(&tmp, &dir, &["prime"]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains("Project backlog"), "{stdout}");
        let out = tk_in(&tmp, &dir, &["--json", "prime"]);
        let json: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["backlog"]["open"], 0);
        assert!(json["backlog"]["project_id"].is_number());

        assert!(
            tk_in(&tmp, &dir, &["create", "Parked", "--to-backlog"])
                .status
                .success()
        );
        let out = tk_in(&tmp, &dir, &["prime"]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Project backlog: 1 open task ("),
            "{stdout}"
        );
        // the line sits in the stats part, before the in-progress section
        let line = stdout.find("Project backlog").unwrap();
        assert!(line > stdout.find("## Stats").unwrap());
        assert!(line < stdout.find("## In Progress").unwrap());

        // the line only shows in workspace scope
        for scope in ["backlog", "project", "all"] {
            let out = tk_in(&tmp, &dir, &["--scope", scope, "prime"]);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(!stdout.contains("Project backlog"), "{scope}: {stdout}");
        }

        let out = tk_in(&tmp, &dir, &["--json", "prime"]);
        let json: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["backlog"]["open"], 1);
        assert_eq!(json["backlog"]["in_progress"], 0);
        assert_eq!(json["backlog"]["blocked"], 0);
        // stats still cover the workspace only
        assert_eq!(json["stats"]["open"], 1);
    }

    #[test]
    fn test_prime_json_backlog_null_without_project() {
        let tmp = TempDir::new().unwrap();
        init_db(&tmp);
        // a directory that is not a git repository
        let dir = tmp.path().join("plain");
        std::fs::create_dir(&dir).unwrap();
        let out = tk_in(&tmp, &dir, &["--json", "--scope", "all", "prime"]);
        let json: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(json["backlog"].is_null(), "{json}");
    }

    #[test]
    fn test_prime_command_reference_mentions_backlog() {
        assert!(
            super::COMMAND_REFERENCE
                .iter()
                .any(|l| l.contains("--to-backlog") && l.contains("--scope backlog"))
        );
    }
}
