use crate::db::Database;
use axum::{
    Router,
    extract::Path as AxumPath,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use pulldown_cmark::{Options, Parser, html};
use rust_embed::Embed;
use std::sync::{Arc, Mutex, atomic::AtomicI64};

/// Shared application state for the web server.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    /// Last known SQLite `PRAGMA data_version` value, used for polling-based live updates.
    pub last_data_version: Arc<AtomicI64>,
}

pub mod errors;
mod handlers;
pub mod scope;
pub mod security;

/// Render a markdown string to an HTML string.
///
/// Enables tables, strikethrough, task lists, and heading attributes. The resulting HTML is
/// sanitized with ammonia (scripts, event handlers, `javascript:` URLs, styles and unknown
/// tags are removed) because text can come from web input; the output is safe to emit with
/// the askama `|safe` filter. Links get `rel="noopener noreferrer"`; task-list checkboxes
/// (disabled `input type=checkbox`) are kept.
pub fn render_markdown(input: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    let parser = Parser::new_ext(input, options);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    sanitize_html(&output)
}

/// Sanitize rendered markdown HTML (see [`render_markdown`]).
fn sanitize_html(dirty: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(["input"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        .attribute_filter(|element, attribute, value| {
            if element == "input" {
                // Only disabled checkboxes (task lists); no other input type survives.
                return match attribute {
                    "type" if value == "checkbox" => Some(value.into()),
                    "type" => None,
                    "checked" | "disabled" => Some("".into()),
                    _ => None,
                };
            }
            Some(value.into())
        })
        .link_rel(Some("noopener noreferrer"));
    builder.clean(dirty).to_string()
}

#[cfg(test)]
mod tests {
    use super::render_markdown;

    #[test]
    fn test_render_markdown_strips_script_tags() {
        let html = render_markdown("hi <script>alert(1)</script> there");
        assert!(!html.contains("<script"), "got: {html}");
        assert!(
            !html.contains("alert(1)") || !html.contains("<script"),
            "got: {html}"
        );
    }

    #[test]
    fn test_render_markdown_strips_event_handlers() {
        let html = render_markdown("<img src=x onerror=alert(1)>");
        assert!(!html.contains("onerror"), "got: {html}");
        let html = render_markdown("<a href=\"https://a.b\" onclick=\"x()\">l</a>");
        assert!(!html.contains("onclick"), "got: {html}");
    }

    #[test]
    fn test_render_markdown_strips_javascript_links() {
        let html = render_markdown("[x](javascript:alert(1))");
        assert!(!html.contains("javascript:"), "got: {html}");
        let html = render_markdown("<a href=\"JaVaScRiPt:alert(1)\">x</a>");
        assert!(!html.contains("alert"), "got: {html}");
    }

    #[test]
    fn test_render_markdown_link_rel_and_task_list() {
        let html = render_markdown("[ok](https://example.com)");
        assert!(html.contains("href=\"https://example.com\""), "got: {html}");
        assert!(html.contains("rel=\"noopener noreferrer\""), "got: {html}");
        let html = render_markdown("- [x] done\n- [ ] todo");
        assert!(html.contains("type=\"checkbox\""), "got: {html}");
        assert!(html.contains("disabled"), "got: {html}");
        assert!(html.contains("checked"), "got: {html}");
        let html = render_markdown("<input type=\"text\" name=\"q\">");
        assert!(!html.contains("type=\"text\""), "got: {html}");
    }

    #[test]
    fn test_render_markdown_basics() {
        // Headings
        let html = render_markdown("# Hello");
        assert!(
            html.contains("<h1>Hello</h1>"),
            "expected h1 tag, got: {html}"
        );

        // Code blocks
        let html = render_markdown("```\nlet x = 1;\n```");
        assert!(html.contains("<code>"), "expected code block, got: {html}");

        // Lists
        let html = render_markdown("- item one\n- item two");
        assert!(html.contains("<ul>"), "expected ul tag, got: {html}");
        assert!(
            html.contains("<li>item one</li>"),
            "expected li item, got: {html}"
        );

        // Inline code
        let html = render_markdown("Use `cargo build` to compile.");
        assert!(
            html.contains("<code>cargo build</code>"),
            "expected inline code, got: {html}"
        );

        // Tables (ENABLE_TABLES option)
        let html = render_markdown("| A | B |\n|---|---|\n| 1 | 2 |");
        assert!(html.contains("<table>"), "expected table tag, got: {html}");

        // Strikethrough (ENABLE_STRIKETHROUGH option)
        let html = render_markdown("~~removed~~");
        assert!(
            html.contains("<del>removed</del>"),
            "expected del tag, got: {html}"
        );
    }
}

/// Embedded static assets (htmx, pico CSS, etc.) compiled into the binary.
#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

/// Serve embedded static files at /static/{path}.
async fn static_handler(AxumPath(path): AxumPath<String>) -> Response {
    match StaticAssets::get(&path) {
        Some(content) => {
            let mime = if path.ends_with(".js") {
                "application/javascript"
            } else if path.ends_with(".css") {
                "text/css"
            } else {
                "application/octet-stream"
            };
            ([(header::CONTENT_TYPE, mime)], content.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Register the scope-aware HTML views under `prefix`: `""` (all projects),
/// `"/p/{project_id}"` (one project) and `"/p/{project_id}/w/{workspace_id}"` (one
/// workspace). The same handlers serve all three; the `Scope` extractor reads the
/// path parameters. Specific routes come before parameterized ones.
fn add_scoped_routes(router: Router<AppState>, prefix: &str) -> Router<AppState> {
    router
        .route(&format!("{prefix}/tasks/new"), get(handlers::task_new))
        .route(
            &format!("{prefix}/tasks/new/modal"),
            get(handlers::task_create_modal),
        )
        .route(
            &format!("{prefix}/tasks"),
            get(handlers::task_list).post(handlers::task_create_form),
        )
        .route(&format!("{prefix}/board"), get(handlers::board))
        .route(&format!("{prefix}/epics"), get(handlers::epics))
        .route(
            &format!("{prefix}/epics/{{id}}"),
            get(handlers::epic_detail),
        )
}

/// Build the axum router with all routes.
pub fn create_router(state: AppState) -> Router {
    let router = Router::new()
        // HTML routes — specific routes before parameterized ones
        .route("/", get(handlers::index));
    let router = add_scoped_routes(router, "");
    let router = add_scoped_routes(router, "/p/{project_id}");
    let router = add_scoped_routes(router, "/p/{project_id}/w/{workspace_id}");
    router
        // Task detail and dep trees stay global (any task, whatever the scope)
        .route("/tasks/{id}/dep-tree", get(handlers::task_dep_tree))
        .route("/tasks/{id}/comments", get(handlers::task_comments))
        .route("/tasks/{id}", get(handlers::task_detail))
        .route("/epics/{id}/dep-tree", get(handlers::epic_dep_tree))
        .route("/static/{*path}", get(static_handler))
        // API routes — specific routes before parameterized ones
        .route(
            "/api/tasks",
            get(handlers::api_list_tasks).post(handlers::api_create_task),
        )
        .route("/api/tasks/ready", get(handlers::api_ready_tasks))
        .route("/api/tasks/blocked", get(handlers::api_blocked_tasks))
        .route("/api/tags", get(handlers::api_tags))
        .route("/api/epics", get(handlers::api_epics))
        .route("/api/workspaces", get(handlers::api_workspaces))
        .route("/api/prime", get(handlers::api_prime))
        .route(
            "/api/tasks/{id}",
            get(handlers::api_show_task).patch(handlers::api_update_task),
        )
        .route("/api/tasks/{id}/close", post(handlers::api_close_task))
        .route("/api/tasks/{id}/deps", post(handlers::api_add_dep))
        .route(
            "/api/tasks/{child_id}/deps/{parent_id}",
            delete(handlers::api_remove_dep),
        )
        .route(
            "/api/tasks/{id}/comments",
            get(handlers::api_list_comments).post(handlers::api_add_comment),
        )
        .route("/api/tasks/{id}/children", get(handlers::api_children))
        .route("/api/tasks/{id}/blockers", get(handlers::api_blockers))
        .route("/api/tasks/{id}/dependents", get(handlers::api_dependents))
        .route("/api/stats", get(handlers::api_stats))
        .route("/api/poll", get(handlers::api_poll))
        .layer(axum::middleware::from_fn(security::guard))
        .with_state(state)
}

/// Start the web server on the given port, shutting down gracefully on Ctrl+C.
pub async fn serve(db_path: &std::path::Path, port: u16) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        last_data_version: Arc::new(AtomicI64::new(0)),
    };
    let app = create_router(state);
    let addr = format!("127.0.0.1:{port}");
    println!("Listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("failed to bind to {addr}: {e}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to listen for ctrl_c");
        })
        .await
        .map_err(|e| format!("server error: {e}"))
}
