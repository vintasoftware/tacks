//! Step definitions for the project backlog feature.
//!
//! Most steps are reused from `workspace_steps.rs` and `guard_steps.rs`; this
//! file only adds assertions that need `@id(..)` placeholders resolved.
#![allow(deprecated)]
use cucumber::then;

use crate::TacksWorld;
use crate::steps::workspace_steps::resolve;

/// Assert stderr contains the text after resolving `@id(..)`, `@path(..)` placeholders.
#[then(expr = "the tk error contains resolved {string}")]
async fn the_tk_error_contains_resolved(world: &mut TacksWorld, expected: String) {
    let expected = resolve(world, &expected);
    assert!(
        world.last_stderr.contains(&expected),
        "expected stderr to contain {expected:?}; stderr:\n{}",
        world.last_stderr
    );
}

/// Assert the line two below the `## Stats` heading (directly under the status counts)
/// starts with the expected text.
#[then(expr = "the tk output has the line {string} directly under the stats line")]
async fn the_tk_output_has_line_under_stats(world: &mut TacksWorld, expected: String) {
    let lines: Vec<&str> = world.last_stdout.lines().collect();
    let heading = lines
        .iter()
        .position(|l| l.trim() == "## Stats")
        .unwrap_or_else(|| panic!("no '## Stats' heading in:\n{}", world.last_stdout));
    let line = lines.get(heading + 2).copied().unwrap_or_default();
    assert!(
        line.starts_with(&expected),
        "expected the line under the stats line to start with {expected:?}; got {line:?}; stdout:\n{}",
        world.last_stdout
    );
}
