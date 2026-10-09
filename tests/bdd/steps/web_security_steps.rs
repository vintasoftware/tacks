//! Step definitions for requests with custom headers (cross-origin and Host guards).
#![allow(deprecated)]
use cucumber::{then, when};
use serde_json::Value;

use crate::TacksWorld;
use crate::steps::workspace_steps::resolve;

/// Replace `@port` with the test server port.
fn with_port(world: &TacksWorld, text: &str) -> String {
    let port = world.server_port.expect("server not started");
    text.replace("@port", &port.to_string())
}

async fn send(world: &mut TacksWorld, req: reqwest::RequestBuilder) {
    let resp = req.send().await.expect("request failed");
    world.last_response_status = Some(resp.status().as_u16());
    world.last_response_headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    world.last_response_body = Some(resp.text().await.expect("read body"));
}

#[when(expr = "I POST {string} with body {string} and header {string} set to {string}")]
async fn post_with_header(
    world: &mut TacksWorld,
    path: String,
    body: String,
    name: String,
    value: String,
) {
    let path = resolve(world, &path);
    let body: Value = serde_json::from_str(&resolve(world, &body)).expect("body is JSON");
    let value = with_port(world, &value);
    let port = world.server_port.expect("server not started");
    let req = world
        .http_client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header(name, value)
        .json(&body);
    send(world, req).await;
}

#[when(expr = "I GET {string} with header {string} set to {string}")]
async fn get_with_header(world: &mut TacksWorld, path: String, name: String, value: String) {
    let path = resolve(world, &path);
    let value = with_port(world, &value);
    let port = world.server_port.expect("server not started");
    let req = world
        .http_client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .header(name, value);
    send(world, req).await;
}

#[when(expr = "I GET {string} recording the response headers")]
async fn get_recording_headers(world: &mut TacksWorld, path: String) {
    let path = resolve(world, &path);
    let port = world.server_port.expect("server not started");
    let req = world
        .http_client
        .get(format!("http://127.0.0.1:{port}{path}"));
    send(world, req).await;
}

#[then(expr = "the response header {string} contains {string}")]
async fn response_header_contains(world: &mut TacksWorld, name: String, expected: String) {
    let name = name.to_lowercase();
    let value = world
        .last_response_headers
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("no {name} header in {:?}", world.last_response_headers));
    assert!(
        value.contains(&expected),
        "header {name} is {value:?}, expected to contain {expected:?}"
    );
}
