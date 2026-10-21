#![allow(deprecated)]
//! Steps asserting on the structure of the `/board` HTML: which column a task
//! card sits in, card order inside a column, column counts and the
//! "blocked by" marker. Columns are delimited by
//! `<div class="board-column" data-status="...">` and cards by
//! `<article class="board-card" ... data-task-id="...">` ... `</article>`.
//! Task aliases resolve through `world.task_ids`.

use cucumber::{given, then, when};
use serde_json::json;

use crate::TacksWorld;
use crate::steps::web_api_steps::http_patch;
use crate::steps::web_steps::http_get;

const COLUMN_MARKER: &str = "<div class=\"board-column\" data-status=\"";

/// Resolve a task alias to its ID.
fn task_id(world: &TacksWorld, alias: &str) -> String {
    world
        .task_ids
        .get(alias)
        .unwrap_or_else(|| panic!("no task with alias '{alias}'"))
        .clone()
}

/// The last response body.
fn body(world: &TacksWorld) -> &str {
    world
        .last_response_body
        .as_deref()
        .expect("no HTTP response body recorded — did you GET the board?")
}

/// HTML of one board column (from its opening tag up to the next column or the end).
fn column_html<'a>(body: &'a str, status: &str) -> &'a str {
    let start_tag = format!("{COLUMN_MARKER}{status}\"");
    let start = body
        .find(&start_tag)
        .unwrap_or_else(|| panic!("board column '{status}' not found in body:\n{body}"));
    let rest = &body[start + start_tag.len()..];
    let end = rest.find(COLUMN_MARKER).unwrap_or(rest.len());
    &rest[..end]
}

/// Task IDs of the cards of a column, in document order.
fn card_ids(column: &str) -> Vec<String> {
    let needle = "data-task-id=\"";
    let mut ids = Vec::new();
    let mut rest = column;
    while let Some(pos) = rest.find(needle) {
        let after = &rest[pos + needle.len()..];
        let end = after.find('"').expect("unterminated data-task-id");
        ids.push(after[..end].to_string());
        rest = &after[end..];
    }
    ids
}

/// HTML of the card of `id` anywhere on the board.
fn card_html<'a>(body: &'a str, id: &str) -> &'a str {
    let needle = format!("data-task-id=\"{id}\"");
    let start = body
        .find(&needle)
        .unwrap_or_else(|| panic!("no board card for task {id} in body:\n{body}"));
    let rest = &body[start..];
    let end = rest.find("</article>").expect("unterminated board card");
    &rest[..end]
}

/// Set a task's status through the API (setup step; the web server must be running).
#[given(expr = "I set the API task {string} to status {string}")]
async fn set_api_task_status(world: &mut TacksWorld, alias: String, status: String) {
    let id = task_id(world, &alias);
    let (code, resp) = http_patch(
        world,
        &format!("/api/tasks/{id}"),
        json!({"status": status}),
    )
    .await;
    assert_eq!(code, 200, "expected 200 from PATCH but got {code}: {resp}");
}

/// GET the main board filtered by an epic identified by alias.
#[when(expr = "I GET the board filtered by epic {string}")]
async fn get_board_filtered_by_epic(world: &mut TacksWorld, alias: String) {
    let id = task_id(world, &alias);
    http_get(world, &format!("/board?epic={id}")).await;
}

#[then(expr = "the board column {string} contains the task {string}")]
async fn column_contains_task(world: &mut TacksWorld, status: String, alias: String) {
    let id = task_id(world, &alias);
    let ids = card_ids(column_html(body(world), &status));
    assert!(
        ids.contains(&id),
        "expected task {alias} ({id}) in column '{status}' but it holds {ids:?}"
    );
}

#[then(expr = "the board column {string} does not contain the task {string}")]
async fn column_lacks_task(world: &mut TacksWorld, status: String, alias: String) {
    let id = task_id(world, &alias);
    let ids = card_ids(column_html(body(world), &status));
    assert!(
        !ids.contains(&id),
        "expected task {alias} ({id}) NOT in column '{status}' but it holds {ids:?}"
    );
}

#[then(expr = "in the board column {string} the task {string} comes before the task {string}")]
async fn column_order(world: &mut TacksWorld, status: String, first: String, second: String) {
    let (a, b) = (task_id(world, &first), task_id(world, &second));
    let ids = card_ids(column_html(body(world), &status));
    let pos = |id: &String| {
        ids.iter()
            .position(|x| x == id)
            .unwrap_or_else(|| panic!("task {id} not in column '{status}': {ids:?}"))
    };
    assert!(
        pos(&a) < pos(&b),
        "expected {first} ({a}) before {second} ({b}) in column '{status}' but order is {ids:?}"
    );
}

#[then(expr = "the board column {string} has {int} cards")]
async fn column_card_count(world: &mut TacksWorld, status: String, expected: usize) {
    let ids = card_ids(column_html(body(world), &status));
    assert_eq!(
        ids.len(),
        expected,
        "expected {expected} cards in column '{status}' but found {ids:?}"
    );
}

#[then(expr = "the board column {string} header count is {int}")]
async fn column_header_count(world: &mut TacksWorld, status: String, expected: usize) {
    let column = column_html(body(world), &status);
    let needle = "<span class=\"column-count\">(";
    let start = column
        .find(needle)
        .unwrap_or_else(|| panic!("no column count in column '{status}':\n{column}"))
        + needle.len();
    let end = column[start..]
        .find(')')
        .expect("unterminated column count");
    let actual: usize = column[start..start + end]
        .parse()
        .expect("column count is not a number");
    assert_eq!(
        actual, expected,
        "expected header count {expected} for column '{status}' but got {actual}"
    );
}

/// Assert the card of a task carries the marker listing exactly the given
/// blocker aliases (comma separated), in that order.
#[then(expr = "the board card of {string} shows it is blocked by {string}")]
async fn card_shows_blocked_by(world: &mut TacksWorld, alias: String, blockers: String) {
    let id = task_id(world, &alias);
    let ids: Vec<String> = blockers
        .split(',')
        .map(|a| task_id(world, a.trim()))
        .collect();
    let card = card_html(body(world), &id);
    let text = format!("\u{2298} blocked by {}</span>", ids.join(", "));
    assert!(
        card.contains(&text),
        "expected card of {alias} ({id}) to contain {text:?}, card:\n{card}"
    );
    let aria = format!("aria-label=\"Blocked by {}\"", ids.join(", "));
    assert!(
        card.contains(&aria),
        "expected card of {alias} ({id}) to contain {aria:?}, card:\n{card}"
    );
    assert!(
        card.contains("class=\"badge dep-blocked\""),
        "expected dep-blocked badge class in card:\n{card}"
    );
}

#[then(expr = "the board card of {string} shows no blocked by marker")]
async fn card_shows_no_marker(world: &mut TacksWorld, alias: String) {
    let id = task_id(world, &alias);
    let card = card_html(body(world), &id);
    assert!(
        !card.contains("blocked by") && !card.contains("dep-blocked"),
        "expected no blocked-by marker on card of {alias} ({id}), card:\n{card}"
    );
}

/// The marker tooltip names each open blocker with its title.
#[then(expr = "the board card of {string} marker tooltip names {string} as {string}")]
async fn card_tooltip_names(world: &mut TacksWorld, alias: String, blocker: String, title: String) {
    let id = task_id(world, &alias);
    let blocker_id = task_id(world, &blocker);
    let card = card_html(body(world), &id);
    let expected = format!("{blocker_id}: {title}");
    let title_attr = card
        .find("title=\"Blocked by ")
        .map(|p| &card[p..])
        .unwrap_or_else(|| panic!("no 'Blocked by' tooltip in card:\n{card}"));
    let end = title_attr[7..].find('"').expect("unterminated title");
    assert!(
        title_attr[..7 + end].contains(&expected),
        "expected tooltip to contain {expected:?}, got {:?}",
        &title_attr[..7 + end]
    );
}
