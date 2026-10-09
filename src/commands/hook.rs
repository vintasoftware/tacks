//! Claude Code hook entry points (`tk hook <event>`), hidden from `--help`.
//!
//! `pre-tool-use` reads the hook JSON from stdin and asks the user to confirm
//! any Bash command that runs `tk` in a way that reaches outside the current
//! workspace. It never touches the database or git, and never fails: any
//! problem results in no output and exit 0 (meaning "no opinion").

use std::io::Read;

use clap::ValueEnum;
use serde_json::{Value, json};

/// Hook events supported by `tk hook`.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum HookEvent {
    /// Claude Code PreToolUse event
    PreToolUse,
}

/// Run the hook for `event`, reading its JSON payload from stdin.
pub fn run(event: HookEvent) {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let out = match event {
        HookEvent::PreToolUse => pre_tool_use(&input),
    };
    if let Some(out) = out {
        println!("{out}");
    }
}

/// Compute the PreToolUse hook output for a stdin payload, if any.
pub fn pre_tool_use(input: &str) -> Option<String> {
    let v: Value = serde_json::from_str(input).ok()?;
    if v.get("tool_name")?.as_str()? != "Bash" {
        return None;
    }
    let command = v.get("tool_input")?.get("command")?.as_str()?;
    let triggers = cross_workspace_triggers(command);
    if triggers.is_empty() {
        return None;
    }
    let reason = format!(
        "tacks: this command reaches outside the current workspace ({}). Allow only if you asked the agent to work on another workspace's tasks.",
        triggers.join(", ")
    );
    Some(
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "ask",
            "permissionDecisionReason": reason,
        }})
        .to_string(),
    )
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Sep,
}

/// Split a shell command into words and command separators, honoring quotes.
fn tokenize(s: &str) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut i = 0;
    macro_rules! flush {
        () => {
            if in_word {
                toks.push(Tok::Word(std::mem::take(&mut cur)));
                in_word = false;
            }
        };
    }
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                in_word = true;
                i += 1;
                if i < chars.len() && chars[i] != '\n' {
                    cur.push(chars[i]);
                }
            }
            ' ' | '\t' => flush!(),
            ';' | '&' | '|' | '(' | ')' | '\n' | '`' => {
                flush!();
                toks.push(Tok::Sep);
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
        i += 1;
    }
    if in_word {
        toks.push(Tok::Word(cur));
    }
    toks
}

fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

fn is_tk(w: &str) -> bool {
    w == "tk" || w.ends_with("/tk")
}

fn env_trigger(w: &str) -> Option<&'static str> {
    if w.starts_with("TACKS_WORKSPACE=") {
        Some("TACKS_WORKSPACE")
    } else if w.starts_with("TACKS_DB=") {
        Some("TACKS_DB")
    } else {
        None
    }
}

fn add(out: &mut Vec<String>, s: String) {
    if !out.contains(&s) {
        out.push(s);
    }
}

/// Inspect the words of a `tk` invocation (after the `tk` word) for flags that
/// reach outside the current workspace.
fn tk_flag_triggers(args: &[&str], out: &mut Vec<String>) {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == "--scope" {
            if let Some(&v) = args.get(i + 1)
                && (v == "project" || v == "all")
            {
                add(out, format!("--scope {v}"));
            }
        } else if let Some(v) = a.strip_prefix("--scope=") {
            if v == "project" || v == "all" {
                add(out, format!("--scope {v}"));
            }
        } else if a == "--workspace" || a.starts_with("--workspace=") {
            add(out, "--workspace".to_string());
        } else if a == "--move-to" || a.starts_with("--move-to=") {
            add(out, "--move-to".to_string());
        } else if a == "--db" || a.starts_with("--db=") {
            add(out, "--db".to_string());
        }
        i += 1;
    }
}

/// Return the list of cross-workspace triggers found in a shell command
/// (empty when the command stays within the current workspace).
pub fn cross_workspace_triggers(command: &str) -> Vec<String> {
    let toks = tokenize(command);
    let mut out = Vec::new();
    for seg in toks.split(|t| *t == Tok::Sep) {
        let words: Vec<&str> = seg
            .iter()
            .filter_map(|t| match t {
                Tok::Word(w) => Some(w.as_str()),
                Tok::Sep => None,
            })
            .collect();
        if words.is_empty() {
            continue;
        }
        let mut i = 0;
        let mut env_hits = Vec::new();
        // leading assignments and wrapper commands
        while let Some(&w) = words.get(i) {
            if is_assignment(w) {
                if let Some(t) = env_trigger(w) {
                    env_hits.push(t);
                }
            } else if matches!(w, "env" | "command" | "exec") {
                // wrapper: keep skipping (env may take assignments next)
            } else {
                break;
            }
            i += 1;
        }
        if i == words.len() {
            // assignment-only segment: persists in the shell, so flag it too
            for t in env_hits {
                add(&mut out, t.to_string());
            }
            continue;
        }
        let w = words[i];
        if w == "export" {
            for a in &words[i + 1..] {
                if let Some(t) = env_trigger(a).or(match *a {
                    "TACKS_WORKSPACE" => Some("TACKS_WORKSPACE"),
                    "TACKS_DB" => Some("TACKS_DB"),
                    _ => None,
                }) {
                    add(&mut out, t.to_string());
                }
            }
        } else if is_tk(w) {
            for t in env_hits {
                add(&mut out, t.to_string());
            }
            tk_flag_triggers(&words[i + 1..], &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(c: &str) -> Vec<String> {
        cross_workspace_triggers(c)
    }

    #[test]
    fn plain_commands_are_fine() {
        assert!(t("tk list").is_empty());
        assert!(t("git status").is_empty());
        assert!(t("tk list --scope workspace").is_empty());
        assert!(t("tk list --scope=workspace").is_empty());
        assert!(t("").is_empty());
    }

    #[test]
    fn scope_flags() {
        assert_eq!(t("tk list --scope all"), ["--scope all"]);
        assert_eq!(t("tk list --scope project"), ["--scope project"]);
        assert_eq!(t("tk --scope=all ready"), ["--scope all"]);
        assert_eq!(t("/usr/local/bin/tk list --scope all"), ["--scope all"]);
    }

    #[test]
    fn other_flags() {
        assert_eq!(t("tk list --workspace /x"), ["--workspace"]);
        assert_eq!(t("tk list --workspace=/x"), ["--workspace"]);
        assert_eq!(t("tk update a --move-to ../b"), ["--move-to"]);
        assert_eq!(t("tk --db /tmp/x.db list"), ["--db"]);
        assert_eq!(t("tk --db=/tmp/x.db list"), ["--db"]);
    }

    #[test]
    fn env_assignments() {
        assert_eq!(t("TACKS_WORKSPACE=/x tk ready"), ["TACKS_WORKSPACE"]);
        assert_eq!(t("FOO=1 TACKS_DB=/y tk ready"), ["TACKS_DB"]);
        assert_eq!(t("env TACKS_WORKSPACE=/x tk ready"), ["TACKS_WORKSPACE"]);
        assert_eq!(
            t("export TACKS_WORKSPACE=/x; tk ready"),
            ["TACKS_WORKSPACE"]
        );
        assert_eq!(t("export TACKS_DB && tk ready"), ["TACKS_DB"]);
        assert!(t("TACKS_WORKSPACE=/x git status").is_empty());
        assert!(t("FOO=1 tk ready").is_empty());
    }

    #[test]
    fn command_positions() {
        assert_eq!(t("cd a && tk update X --move-to ../b"), ["--move-to"]);
        assert_eq!(t("echo hi; tk list --scope all"), ["--scope all"]);
        assert_eq!(t("true || tk list --scope all"), ["--scope all"]);
        assert_eq!(t("cat f | tk list --scope all"), ["--scope all"]);
        assert_eq!(t("(tk list --scope all)"), ["--scope all"]);
        assert_eq!(t("x=$(tk list --scope all)"), ["--scope all"]);
        assert_eq!(t("echo `tk list --scope all`"), ["--scope all"]);
        assert_eq!(t("echo a\ntk list --scope all"), ["--scope all"]);
        assert_eq!(t("command tk list --scope all"), ["--scope all"]);
        assert_eq!(t("exec tk list --scope all"), ["--scope all"]);
    }

    #[test]
    fn quoted_text_does_not_trigger() {
        assert!(t(r#"tk comment X "use --scope all""#).is_empty());
        assert!(t("tk comment X 'run tk list --scope all'").is_empty());
        assert!(t(r#"echo "a; tk list --scope all""#).is_empty());
        assert!(t(r#"git commit -m "tk --workspace x""#).is_empty());
        assert_eq!(t(r#"tk list "--scope" all"#), ["--scope all"]);
    }

    #[test]
    fn non_command_position_tk_ignored() {
        assert!(t("echo tk list --scope all").is_empty());
        assert!(t("which tk --scope all").is_empty());
    }

    #[test]
    fn multiple_triggers_listed_once() {
        assert_eq!(
            t("tk list --scope all --workspace /x; tk ready --scope all"),
            ["--scope all", "--workspace"]
        );
    }

    #[test]
    fn pre_tool_use_output() {
        let inp = r#"{"tool_name":"Bash","tool_input":{"command":"tk list --scope all"}}"#;
        let out: Value = serde_json::from_str(&pre_tool_use(inp).unwrap()).unwrap();
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "ask");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(
            out["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("--scope all")
        );
    }

    #[test]
    fn pre_tool_use_ignores_other_input() {
        assert!(pre_tool_use("garbage").is_none());
        assert!(pre_tool_use("{}").is_none());
        assert!(pre_tool_use(r#"{"tool_name":"Bash"}"#).is_none());
        assert!(
            pre_tool_use(r#"{"tool_name":"Read","tool_input":{"command":"tk list --scope all"}}"#)
                .is_none()
        );
        assert!(
            pre_tool_use(r#"{"tool_name":"Bash","tool_input":{"command":"tk list"}}"#).is_none()
        );
    }
}
