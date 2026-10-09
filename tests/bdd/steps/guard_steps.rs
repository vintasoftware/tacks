//! Step definitions for the write guard and the `tk hook pre-tool-use` agent hook.
#![allow(deprecated)]
use cucumber::{then, when};
use serde_json::{Value, json};

use crate::TacksWorld;

#[then(expr = "the tk error contains {string}")]
async fn the_tk_error_contains(world: &mut TacksWorld, expected: String) {
    assert!(
        world.last_stderr.contains(&expected),
        "expected stderr to contain {expected:?}; stderr:\n{}",
        world.last_stderr
    );
}

#[then(expr = "the tk error does not contain {string}")]
async fn the_tk_error_does_not_contain(world: &mut TacksWorld, unexpected: String) {
    assert!(
        !world.last_stderr.contains(&unexpected),
        "expected stderr NOT to contain {unexpected:?}; stderr:\n{}",
        world.last_stderr
    );
}

#[then(expr = "the tk output does not contain {string}")]
async fn the_tk_output_does_not_contain(world: &mut TacksWorld, unexpected: String) {
    assert!(
        !world.last_stdout.contains(&unexpected),
        "expected stdout NOT to contain {unexpected:?}; stdout:\n{}",
        world.last_stdout
    );
}

/// Feed raw text to `tk hook pre-tool-use` on stdin and record the result.
fn run_hook(world: &mut TacksWorld, stdin: String) {
    let output = assert_cmd::Command::cargo_bin("tk")
        .expect("tk binary not found")
        .args(["hook", "pre-tool-use"])
        .write_stdin(stdin)
        .output()
        .expect("failed to run tk hook");
    world.last_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.last_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    world.last_exit_code = output.status.code().unwrap_or(-1);
}

/// The whole rest of the line is the shell command, so quotes need no escaping.
#[when(regex = r"^I run the pre-tool-use hook for the Bash command (.+)$")]
async fn hook_for_bash_command(world: &mut TacksWorld, command: String) {
    let payload = json!({"tool_name": "Bash", "tool_input": {"command": command.trim()}});
    run_hook(world, payload.to_string());
}

#[when("I run the pre-tool-use hook with this stdin:")]
async fn hook_with_stdin(world: &mut TacksWorld, step: &cucumber::gherkin::Step) {
    let doc = step.docstring.clone().expect("step needs a docstring");
    run_hook(world, doc);
}

#[then("the hook prints nothing and exits 0")]
async fn hook_prints_nothing(world: &mut TacksWorld) {
    assert_eq!(world.last_exit_code, 0, "stderr: {}", world.last_stderr);
    assert!(
        world.last_stdout.trim().is_empty(),
        "expected no output but got:\n{}",
        world.last_stdout
    );
}

#[then(expr = "the hook asks for confirmation naming {string}")]
async fn hook_asks(world: &mut TacksWorld, trigger: String) {
    assert_eq!(world.last_exit_code, 0, "stderr: {}", world.last_stderr);
    let json: Value = serde_json::from_str(&world.last_stdout)
        .unwrap_or_else(|e| panic!("hook output is not JSON ({e}): {}", world.last_stdout));
    let out = &json["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreToolUse", "json: {json}");
    assert_eq!(out["permissionDecision"], "ask", "json: {json}");
    let reason = out["permissionDecisionReason"].as_str().unwrap_or_default();
    assert!(
        reason.contains(&trigger),
        "reason {reason:?} does not name {trigger:?}"
    );
}
