# Web UI security

`tk serve` binds to `127.0.0.1` only, but a browser on the same machine can still be used
against it. Two guards apply (`src/web/security.rs`, `src/web/mod.rs`).

## Markdown sanitization

Descriptions, notes and comments are rendered as markdown and emitted unescaped, and they
can come from web input. `render_markdown` therefore passes the HTML through `ammonia`:
`<script>`, event handlers (`onerror=`), `style`, `javascript:` URLs and unknown tags are
removed. Tables, code, lists, headings, links and task-list checkboxes (`input
type=checkbox`, no other input type) are kept; links get `rel="noopener noreferrer"`.
Remote images (`http(s)` URLs) are still allowed.

## Cross-origin and DNS-rebinding protection

Middleware on every route:

- `Host` must be `127.0.0.1`, `localhost` or `[::1]` (optional numeric port), else 403.
  This blocks DNS rebinding. The port is not compared with the listening port.
- For `POST`, `PUT`, `PATCH`, `DELETE`: 403 when `Sec-Fetch-Site` is `cross-site`, or when an
  `Origin` header is present and its host:port differs from `Host`.
- Requests without `Origin` (curl, scripts, tests) are allowed. The page's own HTMX/fetch
  calls are same-origin and pass.

Limits: any local process can still call the API (no authentication; local-only tool), and
a reverse proxy that rewrites `Host` will be rejected.

## Authorship

Web comments (`POST /api/tasks/{id}/comments`, and the comment sent with
`POST /api/tasks/{id}/close`) are stored with author `user`. The comments endpoint accepts
an optional `author` (trimmed, max 64 chars, empty means `user`). The web close is
unrestricted: it does not apply the CLI pending-feedback guard.
