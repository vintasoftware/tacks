//! Step definitions for the project backlog in the web UI and API (`web_backlog.feature`).
//!
//! Sidebar entries are addressed by their `data-scope-prefix` (`""` is "All",
//! `/p/<id>` a project, `/p/<id>/backlog` its backlog). Arguments accept the
//! placeholders of `workspace_steps::resolve`.
#![allow(deprecated)]
use cucumber::{then, when};
use serde_json::Value;

use crate::TacksWorld;
use crate::steps::workspace_steps::resolve;

/// Body of the last HTTP response.
fn body_of(world: &TacksWorld) -> String {
    world
        .last_response_body
        .clone()
        .expect("no HTTP response recorded")
}

/// The sidebar anchor (from its opening `<a` to `</a>`) for a scope prefix.
fn sidebar_anchor(world: &mut TacksWorld, prefix: &str) -> String {
    let prefix = resolve(world, prefix);
    let marker = format!("data-scope-prefix=\"{prefix}\"");
    let body = body_of(world);
    let start = body
        .find(&marker)
        .unwrap_or_else(|| panic!("no sidebar link {marker} in:\n{body}"));
    let start = body[..start].rfind("<a ").unwrap_or(start);
    let end = body[start..]
        .find("</a>")
        .unwrap_or_else(|| panic!("unterminated sidebar link {marker}"))
        + start;
    body[start..end].to_string()
}

/// Byte offset of a sidebar link in the last response.
fn sidebar_position(world: &mut TacksWorld, prefix: &str) -> usize {
    let prefix = resolve(world, prefix);
    let marker = format!("data-scope-prefix=\"{prefix}\"");
    body_of(world)
        .find(&marker)
        .unwrap_or_else(|| panic!("no sidebar link {marker}"))
}

/// Submit a form (`application/x-www-form-urlencoded`) without following redirects and
/// assert the status and the `Location` header.
#[when(
    expr = "I POST the form {string} to resolved {string} without following redirects it answers {int} with location {string}"
)]
async fn post_form_without_redirect(
    world: &mut TacksWorld,
    form: String,
    path: String,
    status: u16,
    location: String,
) {
    let form = resolve(world, &form);
    let path = resolve(world, &path);
    let location = resolve(world, &location);
    let port = world.server_port.expect("server not started");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");
    let resp = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form)
        .send()
        .await
        .expect("request failed");
    let actual_status = resp.status().as_u16();
    let actual_location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    world.last_response_status = Some(actual_status);
    world.last_response_body = Some(resp.text().await.unwrap_or_default());
    assert_eq!(
        actual_status, status,
        "body: {:?}",
        world.last_response_body
    );
    assert_eq!(actual_location, location);
}

/// Submit a form and only record the response (no redirect following, no assertions).
#[when(expr = "I POST the form {string} to resolved {string}")]
async fn post_form(world: &mut TacksWorld, form: String, path: String) {
    let form = resolve(world, &form);
    let path = resolve(world, &path);
    let port = world.server_port.expect("server not started");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");
    let resp = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form)
        .send()
        .await
        .expect("request failed");
    world.last_response_status = Some(resp.status().as_u16());
    world.last_response_body = Some(resp.text().await.unwrap_or_default());
}

#[then(expr = "the sidebar has the link {string}")]
async fn sidebar_has_link(world: &mut TacksWorld, prefix: String) {
    sidebar_anchor(world, &prefix);
}

/// The number shown in the `scope-count` span of a sidebar link.
#[then(expr = "the sidebar link {string} has count {int}")]
async fn sidebar_link_count(world: &mut TacksWorld, prefix: String, expected: i64) {
    let anchor = sidebar_anchor(world, &prefix);
    let after = anchor
        .split("scope-count")
        .nth(1)
        .unwrap_or_else(|| panic!("no scope-count in {anchor}"));
    let digits: String = after
        .split_once('>')
        .map(|(_, rest)| rest)
        .unwrap_or_default()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let actual: i64 = digits
        .parse()
        .unwrap_or_else(|_| panic!("no count in {anchor}"));
    assert_eq!(actual, expected, "sidebar link {prefix}: {anchor}");
}

#[then(expr = "the sidebar link {string} is active")]
async fn sidebar_link_is_active(world: &mut TacksWorld, prefix: String) {
    let anchor = sidebar_anchor(world, &prefix);
    assert!(
        anchor.contains("scope-active") && anchor.contains("aria-current=\"page\""),
        "sidebar link {prefix} is not active: {anchor}"
    );
}

#[then(expr = "the sidebar link {string} is not active")]
async fn sidebar_link_is_not_active(world: &mut TacksWorld, prefix: String) {
    let anchor = sidebar_anchor(world, &prefix);
    assert!(
        !anchor.contains("scope-active") && !anchor.contains("aria-current"),
        "sidebar link {prefix} is unexpectedly active: {anchor}"
    );
}

#[then(expr = "the sidebar link {string} comes before the sidebar link {string}")]
async fn sidebar_link_order(world: &mut TacksWorld, first: String, second: String) {
    let a = sidebar_position(world, &first);
    let b = sidebar_position(world, &second);
    assert!(a < b, "expected {first} (at {a}) before {second} (at {b})");
}

/// Assert a field of the `/api/backlogs` row of a project (matched by `project_id`).
#[then(expr = "the backlogs response row for project {string} has {string} equal to {string}")]
async fn backlogs_row_has(
    world: &mut TacksWorld,
    project: String,
    field: String,
    expected: String,
) {
    let project = resolve(world, &project);
    let expected = resolve(world, &expected);
    let body = body_of(world);
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    let row = json
        .as_array()
        .unwrap_or_else(|| panic!("not an array: {json}"))
        .iter()
        .find(|r| r["project_id"].as_i64() == project.parse::<i64>().ok())
        .unwrap_or_else(|| panic!("no backlog row for project {project} in {json}"));
    let actual = match &row[field.as_str()] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    assert_eq!(actual, expected, "row: {row}");
}

/// Assert that the response JSON object has exactly the given set of keys.
#[then(expr = "the backlogs response rows have exactly the keys {string}")]
async fn backlogs_rows_keys(world: &mut TacksWorld, csv: String) {
    let body = body_of(world);
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    let mut expected: Vec<&str> = csv.split(',').map(str::trim).collect();
    expected.sort_unstable();
    for row in json.as_array().expect("array") {
        let mut keys: Vec<&str> = row
            .as_object()
            .expect("row is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, expected, "row: {row}");
    }
}

#[then(expr = "the tk output has {int} lines containing {string}")]
async fn tk_output_line_count(world: &mut TacksWorld, expected: usize, needle: String) {
    let needle = resolve(world, &needle);
    let actual = world
        .last_stdout
        .lines()
        .filter(|l| l.contains(&needle))
        .count();
    assert_eq!(
        actual, expected,
        "lines containing {needle:?}; stdout:\n{}",
        world.last_stdout
    );
}

#[then(expr = "the tk output line containing {string} comes before the line containing {string}")]
async fn tk_output_line_order(world: &mut TacksWorld, first: String, second: String) {
    let first = resolve(world, &first);
    let second = resolve(world, &second);
    let lines: Vec<&str> = world.last_stdout.lines().collect();
    let pos = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no line containing {needle:?} in:\n{}", world.last_stdout))
    };
    let (a, b) = (pos(&first), pos(&second));
    assert!(
        a < b,
        "expected {first:?} (line {a}) before {second:?} (line {b}); stdout:\n{}",
        world.last_stdout
    );
}
