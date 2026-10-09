//! Step definitions for the workspace action and pass-2 web UI scenarios.
#![allow(deprecated)]
use cucumber::{then, when};
use serde_json::Value;

use crate::TacksWorld;
use crate::steps::workspace_steps::resolve;

/// GET a path without following redirects and assert status and `Location`.
#[when(expr = "I GET {string} without following redirects it answers {int} with location {string}")]
async fn get_without_redirect(world: &mut TacksWorld, path: String, status: u16, location: String) {
    let port = world.server_port.expect("server not started");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");
    let resp = client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), status);
    let actual = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(actual, location);
}

/// Assert a dot-separated path in the last response JSON is present and not null.
#[then(expr = "the response JSON path {string} is set")]
async fn response_json_path_is_set(world: &mut TacksWorld, path: String) {
    let body = world.last_response_body.clone().expect("no response");
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    let value = path.split('.').fold(&json, |cur, key| &cur[key]);
    assert!(!value.is_null(), "path {path} is not set in {json}");
}

/// POST with a foreign Origin header (placeholders resolved).
#[when(expr = "I POST resolved {string} with body {string} from a foreign origin")]
async fn post_foreign_origin(world: &mut TacksWorld, path: String, body: String) {
    let path = resolve(world, &path);
    let body: Value = serde_json::from_str(&resolve(world, &body)).expect("body is JSON");
    let port = world.server_port.expect("server not started");
    let resp = world
        .http_client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header("Origin", "http://evil.example")
        .json(&body)
        .send()
        .await
        .expect("request failed");
    world.last_response_status = Some(resp.status().as_u16());
    world.last_response_body = Some(resp.text().await.expect("read body"));
}
