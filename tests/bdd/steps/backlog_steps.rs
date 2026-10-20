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
