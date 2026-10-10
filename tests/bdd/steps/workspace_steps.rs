//! Step definitions for workspace scoping (CLI and web).
//!
//! Every scenario builds its own throwaway git repositories and worktrees
//! under a temp directory owned by the World. `HOME` is pointed at a temp
//! directory for every `tk` run, so nothing here can touch `~/.tacks` or the
//! repository under test.
//!
//! Placeholders usable in step arguments, request paths and bodies (resolved
//! at run time from `tk workspaces --json`, never hardcoded):
//!
//! - `@path(A)`      canonical path of the directory registered as "A"
//! - `@workspace(A)` workspace id of the workspace whose path is "A"
//! - `@project(A)`   project id of the workspace whose path is "A"
//! - `@id(alias)`    task id saved under `alias`
#![allow(deprecated)]
use std::path::{Path, PathBuf};
use std::process::Output;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::TacksWorld;
use crate::steps::web_api_steps::{http_patch, http_post};
use crate::steps::web_steps::http_get;

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Return the scenario's scratch directory, creating it (and `home/`) on demand.
pub fn fs_root(world: &mut TacksWorld) -> PathBuf {
    if world.fs_dir.is_none() {
        let dir = tempfile::TempDir::new().expect("create fs temp dir");
        std::fs::create_dir_all(dir.path().join("home")).expect("create fake home");
        world.fs_dir = Some(dir);
    }
    world
        .fs_dir
        .as_ref()
        .expect("fs_dir")
        .path()
        .canonicalize()
        .expect("canonicalize fs root")
}

/// Run git with a hermetic configuration.
fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn register_dir(world: &mut TacksWorld, alias: &str, path: &Path) {
    let canon = path.canonicalize().expect("canonicalize directory");
    world.fs_paths.insert(alias.to_string(), canon);
}

pub fn dir_of(world: &TacksWorld, alias: &str) -> PathBuf {
    world
        .fs_paths
        .get(alias)
        .unwrap_or_else(|| panic!("no directory registered as '{alias}'"))
        .clone()
}

// ---------------------------------------------------------------------------
// Running tk and resolving placeholders
// ---------------------------------------------------------------------------

/// Split a command line on whitespace, honouring single and double quotes.
fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                has_token = true;
            }
            None if c.is_whitespace() => {
                if has_token || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            None => cur.push(c),
        }
    }
    if has_token || !cur.is_empty() {
        args.push(cur);
    }
    args
}

/// Build a `tk` command that is isolated from the real HOME and environment.
pub fn tk_command(world: &mut TacksWorld, cwd: &Path) -> assert_cmd::Command {
    let root = fs_root(world);
    let db_path = world
        .db_path
        .clone()
        .expect("db_path not set — did you forget 'Given a tacks database is initialized'?");
    let mut cmd = assert_cmd::Command::cargo_bin("tk").expect("tk binary not found");
    cmd.current_dir(cwd)
        .env("TACKS_DB", db_path)
        .env("HOME", root.join("home"))
        .env_remove("TACKS_WORKSPACE");
    for (k, v) in &world.extra_env {
        cmd.env(k, v);
    }
    cmd
}

/// Fetch the rows of `tk workspaces --json` (run from the scratch root, unscoped).
fn workspace_rows(world: &mut TacksWorld) -> Vec<Value> {
    let root = fs_root(world);
    let mut cmd = tk_command(world, &root);
    let out = cmd
        .args(["--json", "workspaces"])
        .output()
        .expect("failed to run tk workspaces");
    assert!(
        out.status.success(),
        "tk workspaces failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).expect("workspaces output is JSON");
    json.as_array()
        .expect("workspaces JSON is an array")
        .clone()
}

/// Find the workspace row registered for a directory alias.
fn row_for(world: &mut TacksWorld, alias: &str) -> Value {
    let path = dir_of(world, alias).to_string_lossy().into_owned();
    workspace_rows(world)
        .into_iter()
        .find(|r| r["path"].as_str() == Some(path.as_str()))
        .unwrap_or_else(|| panic!("no registered workspace for '{alias}' ({path})"))
}

/// Replace `@path(..)`, `@workspace(..)`, `@project(..)` and `@id(..)` tokens.
pub fn resolve(world: &mut TacksWorld, input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(i) = rest.find('@') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let parsed = after.find('(').and_then(|open| {
            let kind = &after[..open];
            let close = after[open..].find(')')? + open;
            matches!(kind, "path" | "workspace" | "project" | "id")
                .then(|| (kind, &after[open + 1..close], close))
        });
        match parsed {
            Some((kind, arg, close)) => {
                let value = match kind {
                    "path" => dir_of(world, arg).to_string_lossy().into_owned(),
                    "id" => world
                        .task_ids
                        .get(arg)
                        .unwrap_or_else(|| panic!("no task saved as '{arg}'"))
                        .clone(),
                    "workspace" => row_for(world, arg)["workspace_id"].to_string(),
                    _ => row_for(world, arg)["project_id"].to_string(),
                };
                out.push_str(&value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('@');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn record(world: &mut TacksWorld, output: &Output) {
    world.last_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.last_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    world.last_exit_code = output.status.code().unwrap_or(-1);
}

fn run_in(world: &mut TacksWorld, dir: &str, line: &str) -> Output {
    let cwd = dir_of(world, dir);
    let line = resolve(world, line);
    let mut cmd = tk_command(world, &cwd);
    let output = cmd
        .args(split_args(&line))
        .output()
        .expect("failed to run tk");
    record(world, &output);
    output
}

// ---------------------------------------------------------------------------
// Given steps
// ---------------------------------------------------------------------------

#[given(expr = "a git repository {string}")]
async fn a_git_repository(world: &mut TacksWorld, alias: String) {
    let path = fs_root(world).join(&alias);
    std::fs::create_dir_all(&path).expect("create repo dir");
    git(&path, &["init", "-q", "-b", "main"]);
    git(&path, &["commit", "-q", "--allow-empty", "-m", "init"]);
    register_dir(world, &alias, &path);
}

#[given(expr = "a worktree {string} of repository {string}")]
async fn a_worktree_of_repository(world: &mut TacksWorld, alias: String, repo: String) {
    let repo_path = dir_of(world, &repo);
    let path = fs_root(world).join(&alias);
    let path_str = path.to_string_lossy().into_owned();
    git(
        &repo_path,
        &["worktree", "add", "-q", &path_str, "-b", &alias],
    );
    register_dir(world, &alias, &path);
}

#[given(expr = "a directory {string} outside any git repository")]
async fn a_directory_outside_git(world: &mut TacksWorld, alias: String) {
    let path = fs_root(world).join(&alias);
    std::fs::create_dir_all(&path).expect("create plain dir");
    register_dir(world, &alias, &path);
}

/// Registers the subdirectory under the alias `"<parent>/<name>"`.
#[given(expr = "a subdirectory {string} inside {string}")]
async fn a_subdirectory_inside(world: &mut TacksWorld, name: String, parent: String) {
    let path = dir_of(world, &parent).join(&name);
    std::fs::create_dir_all(&path).expect("create subdirectory");
    register_dir(world, &format!("{parent}/{name}"), &path);
}

#[given(expr = "the environment variable {string} is set to {string}")]
async fn env_var_is_set(world: &mut TacksWorld, key: String, value: String) {
    let value = resolve(world, &value);
    world.extra_env.push((key, value));
}

// ---------------------------------------------------------------------------
// When steps (CLI)
// ---------------------------------------------------------------------------

#[given(expr = "I run tk {string} in {string}")]
#[when(expr = "I run tk {string} in {string}")]
async fn i_run_tk_in(world: &mut TacksWorld, line: String, dir: String) {
    run_in(world, &dir, &line);
}

#[given(expr = "I run tk {string} in {string} and save the id as {string}")]
#[when(expr = "I run tk {string} in {string} and save the id as {string}")]
async fn i_run_tk_in_and_save_id(world: &mut TacksWorld, line: String, dir: String, alias: String) {
    let output = run_in(world, &dir, &line);
    assert!(
        output.status.success(),
        "tk {line} failed: {}",
        world.last_stderr
    );
    let json: Value = serde_json::from_str(&world.last_stdout).expect("output is JSON");
    let id = json["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no 'id' in output: {json}"))
        .to_string();
    world.task_ids.insert(alias, id);
}

#[given(expr = "I remove the directory {string}")]
#[when(expr = "I remove the directory {string}")]
async fn i_remove_the_directory(world: &mut TacksWorld, alias: String) {
    let path = dir_of(world, &alias);
    std::fs::remove_dir_all(&path).expect("remove directory");
}

// ---------------------------------------------------------------------------
// Then steps (CLI)
// ---------------------------------------------------------------------------

#[then("the tk command succeeds")]
async fn the_tk_command_succeeds(world: &mut TacksWorld) {
    assert_eq!(
        world.last_exit_code, 0,
        "expected success but got exit {}: {}",
        world.last_exit_code, world.last_stderr
    );
}

#[then("the tk command fails with exit code 1")]
async fn the_tk_command_fails(world: &mut TacksWorld) {
    assert_eq!(
        world.last_exit_code, 1,
        "expected exit 1; stdout: {} stderr: {}",
        world.last_stdout, world.last_stderr
    );
}

#[then(expr = "the tk output contains {string}")]
async fn the_tk_output_contains(world: &mut TacksWorld, expected: String) {
    assert!(
        world.last_stdout.contains(&expected),
        "expected stdout to contain {expected:?}; stdout:\n{}",
        world.last_stdout
    );
}

/// Read a task's JSON through the CLI (unscoped, by id).
fn show_task(world: &mut TacksWorld, alias: &str) -> Value {
    let id = world.task_ids.get(alias).expect("task alias").clone();
    let root = fs_root(world);
    let mut cmd = tk_command(world, &root);
    let out = cmd.args(["--json", "show", &id]).output().expect("tk show");
    serde_json::from_slice(&out.stdout).expect("show output is JSON")
}

#[given(expr = "I remember the updated_at of task {string}")]
async fn remember_updated_at(world: &mut TacksWorld, alias: String) {
    let json = show_task(world, &alias);
    let value = render(&json["updated_at"]);
    world.stored_updated_at.insert(alias, value);
}

#[then(expr = "the updated_at of task {string} is unchanged")]
async fn updated_at_unchanged(world: &mut TacksWorld, alias: String) {
    let before = world
        .stored_updated_at
        .get(&alias)
        .expect("not remembered")
        .clone();
    assert_eq!(render(&show_task(world, &alias)["updated_at"]), before);
}

#[then(expr = "the updated_at of task {string} has changed")]
async fn updated_at_changed(world: &mut TacksWorld, alias: String) {
    let before = world
        .stored_updated_at
        .get(&alias)
        .expect("not remembered")
        .clone();
    assert_ne!(render(&show_task(world, &alias)["updated_at"]), before);
}

#[then(expr = "the task {string} has title {string}")]
async fn task_has_title(world: &mut TacksWorld, alias: String, expected: String) {
    assert_eq!(render(&show_task(world, &alias)["title"]), expected);
}

#[then("the tk output is empty")]
async fn the_tk_output_is_empty(world: &mut TacksWorld) {
    assert_eq!(world.last_exit_code, 0, "stderr: {}", world.last_stderr);
    assert!(
        world.last_stdout.trim().is_empty(),
        "expected empty stdout but got:\n{}",
        world.last_stdout
    );
}

fn sorted_titles(json: &Value) -> Vec<String> {
    let mut titles: Vec<String> = json
        .as_array()
        .unwrap_or_else(|| panic!("expected a JSON array but got: {json}"))
        .iter()
        .map(|t| t["title"].as_str().unwrap_or_default().to_string())
        .collect();
    titles.sort();
    titles
}

fn expected_titles(csv: &str) -> Vec<String> {
    let mut v: Vec<String> = csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    v.sort();
    v
}

/// Assert the JSON array on stdout holds exactly these titles (comma separated, "" for none).
#[then(expr = "the JSON output lists exactly the titles {string}")]
async fn json_output_lists_titles(world: &mut TacksWorld, csv: String) {
    assert_eq!(world.last_exit_code, 0, "stderr: {}", world.last_stderr);
    let json: Value = serde_json::from_str(&world.last_stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", world.last_stdout));
    assert_eq!(sorted_titles(&json), expected_titles(&csv));
}

#[then(expr = "the stats JSON counts {int} tasks in total")]
async fn stats_counts_total(world: &mut TacksWorld, expected: i64) {
    let json: Value = serde_json::from_str(&world.last_stdout).expect("stats output is JSON");
    let total: i64 = json["by_status"]
        .as_object()
        .expect("by_status is an object")
        .values()
        .filter_map(Value::as_i64)
        .sum();
    assert_eq!(total, expected, "stats: {json}");
}

fn render(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn path_value<'a>(json: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .fold(json, |cur, key| match key.parse::<usize>() {
            Ok(i) if cur.is_array() => &cur[i],
            _ => &cur[key],
        })
}

/// Assert a dot-separated path in the stdout JSON renders as the expected text.
/// Numbers, booleans and null render as `5`, `true`, `null`.
#[then(expr = "the JSON output path {string} equals {string}")]
async fn json_output_path_equals(world: &mut TacksWorld, path: String, expected: String) {
    let expected = resolve(world, &expected);
    let json: Value = serde_json::from_str(&world.last_stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", world.last_stdout));
    assert_eq!(render(path_value(&json, &path)), expected, "json: {json}");
}

fn assert_row(rows: &[Value], path: &str, field: &str, expected: &str) {
    let row = rows
        .iter()
        .find(|r| r["path"].as_str() == Some(path))
        .unwrap_or_else(|| panic!("no workspace row with path {path} in {rows:?}"));
    assert_eq!(render(&row[field]), expected, "row: {row}");
}

/// Assert on the row of `tk workspaces --json` for a directory.
#[then(expr = "the workspaces JSON row for {string} has {string} equal to {string}")]
async fn workspaces_row_has(
    world: &mut TacksWorld,
    alias: String,
    field: String,
    expected: String,
) {
    let path = dir_of(world, &alias).to_string_lossy().into_owned();
    let expected = resolve(world, &expected);
    let rows = workspace_rows(world);
    assert_row(&rows, &path, &field, &expected);
}

#[then(expr = "the workspaces output has {int} workspaces")]
async fn workspaces_output_count(world: &mut TacksWorld, expected: usize) {
    let json: Value = serde_json::from_str(&world.last_stdout).expect("stdout is JSON");
    assert_eq!(json.as_array().map_or(0, Vec::len), expected, "{json}");
}

// ---------------------------------------------------------------------------
// Web steps
// ---------------------------------------------------------------------------

#[when(expr = "I GET resolved {string}")]
async fn i_get_resolved(world: &mut TacksWorld, path: String) {
    let path = resolve(world, &path);
    http_get(world, &path).await;
}

#[when(expr = "I HTMX GET resolved {string}")]
async fn i_htmx_get_resolved(world: &mut TacksWorld, path: String) {
    let path = resolve(world, &path);
    let port = world.server_port.expect("server not started");
    let url = format!("http://127.0.0.1:{port}{path}");
    let resp = world
        .http_client
        .get(&url)
        .header("HX-Request", "true")
        .send()
        .await
        .unwrap_or_else(|e| panic!("HTMX GET {url} failed: {e}"));
    world.last_response_status = Some(resp.status().as_u16());
    world.last_response_body = Some(resp.text().await.expect("read body"));
}

#[when(expr = "I POST resolved {string} with body {string}")]
async fn i_post_resolved(world: &mut TacksWorld, path: String, body: String) {
    let path = resolve(world, &path);
    let body = resolve(world, &body);
    let body: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("bad body: {e}"));
    http_post(world, &path, body).await;
}

#[when(expr = "I PATCH resolved {string} with body {string}")]
async fn i_patch_resolved(world: &mut TacksWorld, path: String, body: String) {
    let path = resolve(world, &path);
    let body = resolve(world, &body);
    let body: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("bad body: {e}"));
    http_patch(world, &path, body).await;
}

fn last_body(world: &TacksWorld) -> String {
    world
        .last_response_body
        .clone()
        .expect("no HTTP response recorded")
}

#[then("the response status is an error")]
async fn response_status_is_error(world: &mut TacksWorld) {
    let status = world.last_response_status.expect("no HTTP response");
    assert!(status >= 400, "expected an error status but got {status}");
}

#[then(expr = "the response body contains resolved {string}")]
async fn body_contains_resolved(world: &mut TacksWorld, expected: String) {
    let expected = resolve(world, &expected);
    let body = last_body(world);
    assert!(
        body.contains(&expected),
        "expected body to contain {expected:?}; body:\n{body}"
    );
}

#[then(expr = "the response body does not contain resolved {string}")]
async fn body_not_contains_resolved(world: &mut TacksWorld, unexpected: String) {
    let unexpected = resolve(world, &unexpected);
    let body = last_body(world);
    assert!(
        !body.contains(&unexpected),
        "expected body NOT to contain {unexpected:?}; body:\n{body}"
    );
}

/// Extract the sidebar anchor for a workspace (from its `data-scope-prefix` to `</a>`).
fn sidebar_link(world: &mut TacksWorld, alias: &str) -> Option<String> {
    let row = row_for(world, alias);
    let marker = format!(
        "data-scope-prefix=\"/p/{}/w/{}\"",
        row["project_id"], row["workspace_id"]
    );
    let body = last_body(world);
    let start = body.find(&marker)?;
    let end = body[start..].find("</a>")? + start;
    Some(body[start..end].to_string())
}

#[then(expr = "the sidebar lists workspace {string}")]
async fn sidebar_lists_workspace(world: &mut TacksWorld, alias: String) {
    assert!(
        sidebar_link(world, &alias).is_some(),
        "sidebar has no link for workspace '{alias}'"
    );
}

#[then(expr = "the sidebar marks workspace {string} as missing")]
async fn sidebar_marks_missing(world: &mut TacksWorld, alias: String) {
    let link = sidebar_link(world, &alias).expect("sidebar link not found");
    assert!(
        link.contains("scope-missing") && link.contains("scope-missing-label"),
        "workspace '{alias}' not marked missing: {link}"
    );
}

#[then(expr = "the sidebar does not mark workspace {string} as missing")]
async fn sidebar_not_marked_missing(world: &mut TacksWorld, alias: String) {
    let link = sidebar_link(world, &alias).expect("sidebar link not found");
    assert!(
        !link.contains("scope-missing"),
        "workspace '{alias}' unexpectedly marked missing: {link}"
    );
}

#[then(expr = "the sidebar lists project {string}")]
async fn sidebar_lists_project(world: &mut TacksWorld, alias: String) {
    let pid = row_for(world, &alias)["project_id"].to_string();
    let marker = format!("data-scope-prefix=\"/p/{pid}\"");
    let body = last_body(world);
    assert!(
        body.contains(&marker),
        "sidebar has no project link {marker}"
    );
}

#[then(expr = "the response JSON path {string} equals {string}")]
async fn response_json_path_equals(world: &mut TacksWorld, path: String, expected: String) {
    let expected = resolve(world, &expected);
    let body = last_body(world);
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    assert_eq!(render(path_value(&json, &path)), expected, "json: {json}");
}

#[then(expr = "the response JSON array lists exactly the titles {string}")]
async fn response_array_lists_titles(world: &mut TacksWorld, csv: String) {
    let body = last_body(world);
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    assert_eq!(sorted_titles(&json), expected_titles(&csv));
}

#[then(expr = "the workspaces response row for {string} has {string} equal to {string}")]
async fn workspaces_response_row_has(
    world: &mut TacksWorld,
    alias: String,
    field: String,
    expected: String,
) {
    let path = dir_of(world, &alias).to_string_lossy().into_owned();
    let body = last_body(world);
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    let expected = resolve(world, &expected);
    let rows = json.as_array().expect("array").clone();
    assert_row(&rows, &path, &field, &expected);
}

/// Task titled `title` has the given `workspace_id` rendering, read through the CLI.
#[then(expr = "the task {string} has workspace_id {string}")]
async fn task_has_workspace_id(world: &mut TacksWorld, alias: String, expected: String) {
    let expected = resolve(world, &expected);
    let id = world.task_ids.get(&alias).expect("task alias").clone();
    let root = fs_root(world);
    let mut cmd = tk_command(world, &root);
    let out = cmd.args(["--json", "show", &id]).output().expect("tk show");
    let json: Value = serde_json::from_slice(&out.stdout).expect("show output is JSON");
    assert_eq!(render(&json["workspace_id"]), expected, "show: {json}");
}

// ---------------------------------------------------------------------------
// Archived assertions (auto-archive)
// ---------------------------------------------------------------------------

fn assert_row_archived(rows: &[Value], path: &str) {
    let row = rows
        .iter()
        .find(|r| r["path"].as_str() == Some(path))
        .unwrap_or_else(|| panic!("no workspace row with path {path} in {rows:?}"));
    assert!(
        row["archived_at"].as_str().is_some_and(|s| !s.is_empty()),
        "expected archived_at to be set, row: {row}"
    );
}

/// The `tk workspaces --json` row for a directory has `archived_at` set.
#[then(expr = "the workspaces JSON row for {string} is archived")]
async fn workspaces_row_is_archived(world: &mut TacksWorld, alias: String) {
    let path = dir_of(world, &alias).to_string_lossy().into_owned();
    let rows = workspace_rows(world);
    assert_row_archived(&rows, &path);
}

/// The `GET /api/workspaces` row for a directory has `archived_at` set.
#[then(expr = "the workspaces response row for {string} is archived")]
async fn workspaces_response_row_is_archived(world: &mut TacksWorld, alias: String) {
    let path = dir_of(world, &alias).to_string_lossy().into_owned();
    let body = last_body(world);
    let json: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    let rows = json.as_array().expect("array").clone();
    assert_row_archived(&rows, &path);
}
