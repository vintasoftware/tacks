//! Step definitions for the user feedback loop: `tk hook post-tool-use` and database-file checks.
#![allow(deprecated)]
use cucumber::{given, then, when};
use serde_json::{Value, json};

use crate::TacksWorld;
use crate::steps::workspace_steps::{dir_of, record, resolve, tk_command};

/// Run `tk hook post-tool-use` from `dir` with the given stdin text.
fn run_post_hook(world: &mut TacksWorld, dir: &str, stdin: String) {
    let cwd = dir_of(world, dir);
    let mut cmd = tk_command(world, &cwd);
    let output = cmd
        .args(["hook", "post-tool-use"])
        .write_stdin(stdin)
        .output()
        .expect("failed to run tk hook");
    record(world, &output);
}

#[given(expr = "I run the post-tool-use hook in {string}")]
#[when(expr = "I run the post-tool-use hook in {string}")]
async fn post_hook_in(world: &mut TacksWorld, dir: String) {
    let cwd = dir_of(world, &dir).to_string_lossy().into_owned();
    let payload = json!({"cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read"});
    run_post_hook(world, &dir, payload.to_string());
}

#[given(expr = "I run the post-tool-use hook in {string} with raw stdin {string}")]
#[when(expr = "I run the post-tool-use hook in {string} with raw stdin {string}")]
async fn post_hook_raw(world: &mut TacksWorld, dir: String, stdin: String) {
    run_post_hook(world, &dir, stdin);
}

#[then(expr = "the hook delivers context containing {string}")]
async fn hook_delivers(world: &mut TacksWorld, expected: String) {
    let expected = resolve(world, &expected);
    assert_eq!(world.last_exit_code, 0, "stderr: {}", world.last_stderr);
    let json: Value = serde_json::from_str(&world.last_stdout)
        .unwrap_or_else(|e| panic!("hook output is not JSON ({e}): {}", world.last_stdout));
    let out = &json["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PostToolUse", "json: {json}");
    let ctx = out["additionalContext"].as_str().unwrap_or_default();
    assert!(
        ctx.contains(&expected),
        "additionalContext {ctx:?} does not contain {expected:?}"
    );
}

#[then(expr = "the hook context does not contain {string}")]
async fn hook_context_lacks(world: &mut TacksWorld, unexpected: String) {
    let json: Value = serde_json::from_str(&world.last_stdout)
        .unwrap_or_else(|e| panic!("hook output is not JSON ({e}): {}", world.last_stdout));
    let ctx = json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !ctx.contains(&unexpected),
        "context {ctx:?} has {unexpected:?}"
    );
}

#[given("I delete the database file")]
async fn delete_database_file(world: &mut TacksWorld) {
    let path = world.db_path.clone().expect("db_path not set");
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.clone().into_os_string();
        p.push(suffix);
        let _ = std::fs::remove_file(p);
    }
}

#[then("the database file does not exist")]
async fn database_file_absent(world: &mut TacksWorld) {
    let path = world.db_path.clone().expect("db_path not set");
    assert!(!path.exists(), "database file {path:?} was created");
}

#[then(expr = "the JSON output path {string} is an empty array")]
async fn json_path_empty_array(world: &mut TacksWorld, path: String) {
    let json: Value = serde_json::from_str(&world.last_stdout).expect("stdout is JSON");
    let v = path.split('.').fold(&json, |c, k| &c[k]);
    assert_eq!(v.as_array().map(Vec::len), Some(0), "json: {json}");
}

#[then(expr = "the tk output contains resolved {string}")]
async fn tk_output_contains_resolved(world: &mut TacksWorld, expected: String) {
    let expected = resolve(world, &expected);
    assert!(
        world.last_stdout.contains(&expected),
        "expected stdout to contain {expected:?}; stdout:\n{}",
        world.last_stdout
    );
}
