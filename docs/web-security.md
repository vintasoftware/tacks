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

## Response headers

The same middleware adds these headers to every response (including 403 rejections):

- `X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`: the UI cannot
  be framed (clickjacking).
- `X-Content-Type-Options: nosniff`.
- `Referrer-Policy: same-origin`.

The CSP is deliberately limited to `frame-ancestors`. The templates and `static/app.js` use
inline scripts and `style` attributes plus htmx, so a `script-src`/`style-src` policy would
need `'unsafe-inline'` and give no protection. A strict CSP would first require moving every
inline script/style out of the templates.

## Client-side rendering

`static/app.js` builds dynamic DOM (filter pills, edit-form messages, dialogs) with
`createElement`/`textContent`/`setAttribute`, never by concatenating data into HTML strings.
Task ids read from `location.hash` are validated against
`^[A-Za-z0-9_-]+-[A-Za-z0-9]+(\.[0-9]+)*$` and URL-encoded before use; invalid hashes are ignored.

## Archived workspaces

Removing a workspace in the UI archives it, and the HTML views (sidebar, board, lists, epics,
counts) hide its tasks. The JSON API is a stable machine interface and does not apply that
filter: `/api/*` with `?project=` or `?workspace=` still includes tasks of archived workspaces.

## Authorship

Web comments (`POST /api/tasks/{id}/comments`, and the comment sent with
`POST /api/tasks/{id}/close`) are stored with author `user`. The comments endpoint accepts
an optional `author` (trimmed, max 64 chars, empty means `user`). The web close is
unrestricted: it does not apply the CLI pending-feedback guard.
