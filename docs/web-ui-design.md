# Web UI Design

Design rules for `tk serve`. The UI is for a human who watches agents work
across projects and workspaces, and who leaves feedback in comments.

## Principles

- Show what agents are doing: status, assignee, working notes, blockers and
  unanswered feedback must be visible without extra clicks.
- One layout for every scope. The scope (All / project / workspace) changes the
  data, not the layout.
- Quiet by default. Text in the normal text color, links without underlines
  except in prose (markdown), color only for status, priority and warnings.
- Works at phone width (390px): no horizontal page scroll, 16px side gutter.
- Dark and light themes both first-class.
- Compact density. Pico's defaults (about 16-18px text, 44px+ controls, large
  headings) are too big for a work tool. Base text about 14px, small text 12px,
  controls and buttons about 32px high with tight padding, page headings
  about 1.25rem, section headings about 1rem. Spacing follows a 4px scale.
- Board is the default view: `/` and the sidebar scope links open the board.

## App shell

```
+--------------------------------------------------------------+
| Tacks   Issues  Board  Epics          [New issue] [theme]    |  top bar, full width
+-------------+------------------------------------------------+
| All      13 | shop / shop-checkout                (path tip) |  scope header
| shop     11 | [filters ...........................]          |
|   shop    3 |                                                |
|   checkout 5| content                                        |
|   old  miss |                                                |
| blog      2 |                                                |
+-------------+------------------------------------------------+
```

- Top bar spans the full width. The brand aligns with the sidebar's left edge.
  Nav tabs follow the brand. "New issue" (primary) and the theme toggle (sun /
  moon icon button with aria-label) sit at the right.
- Sidebar: fixed width (~240px), sticky, scrolls on its own. Project rows are
  headings; workspace rows are indented. Counts are right-aligned and muted.
  The active row is highlighted. A missing workspace is muted italic with a
  small "missing" chip.
- Scope header at the top of the content: `project / workspace` (or "All
  projects"), with the workspace path in a tooltip.
- Content has consistent padding (24px desktop, 16px mobile). The board uses
  the full width; lists and detail pages are capped at a readable width
  (~1100px for lists, ~860px for detail).
- Below 900px: the sidebar is hidden. A "Scope" button in the top bar opens the
  same tree as a drawer/disclosure. The nav tabs move to a second row of the
  top bar.

## Filter bar

- All controls share height, radius, border, font size and left-aligned text.
  No control gets a different shape (no pill edge on the last control).
- Controls sit in one row with a fixed gap and wrap on narrow screens (two per
  row on phones).

## Board

- Column headers: compact (about 1.1rem), with the count as a muted chip.
- Cards are a single click target. Title in the normal text color, no
  underline, hover highlight.
- Card meta row (small, muted): task id (monospace), priority badge, assignee,
  workspace badge (only in project/All scope), `💬 N` for unanswered user
  feedback, "blocked by" chip for dependency-blocked tasks.
- On phones, columns scroll horizontally with snap (each column ~85vw), not
  stacked.

## Task list

- ID column: monospace, muted, never wraps.
- Title column: title, with small chips under it (epic, blocked, feedback).
- Workspace column only in project/All scope (replaces the inline badge).
- Assignee column. Updated column shows relative time ("2h ago") with the
  absolute time as a tooltip.
- The table scrolls horizontally inside its wrapper on narrow screens.

## Task detail (full page and modal share one partial)

1. Breadcrumb at the top (`Issues › tk-a1b2`), with spacing below the top bar.
2. Title.
3. Metadata as a two-column definition list: Status, Priority, Assignee, Epic,
   Tags, Workspace (badge + project, path in small muted monospace, truncated
   with a tooltip, plus a small "Move to…" select), Created / Updated, Close
   reason (when done).
4. Sections in this order:
   - Agent notes (`notes`, markdown), when present.
   - Description, or a muted "No description".
   - Blocked by / Blocks: shown expanded when not empty.
   - Subtasks (for epics).
   - Comments, then the comment form.
- Comment header: author badge, time and markers aligned left in one row.
- Modal: no "View full page" button in the footer. A small "open in full
  page" icon link (with aria-label and tooltip) sits in the modal header next
  to the close button.

## Workspace actions

Shown in the scope header when a single workspace is selected (and as a small
menu on the workspace row in the sidebar):

- **Close all tasks**: closes every task of the workspace that is not done
  (reason `done`, optional comment). A confirmation dialog (in-page, not a
  browser `confirm()`) shows how many tasks will close.
- **Remove workspace**: archives the workspace in tacks. Its tasks are kept
  exactly as they are (no status change), but the web UI no longer shows the
  workspace or any of its tasks: not in the sidebar, not in project/All
  boards, lists, epics, counts or the create/move selects. Task detail by
  direct URL still works. It never touches the git worktree on disk. The
  confirmation dialog says how many tasks will be hidden. The CLI is not
  affected (`tk workspaces` marks the workspace as archived). If an agent
  registers the same path again (for example `tk create` in a recreated
  worktree), the workspace and its tasks come back automatically.
- **Restore workspace**: archived workspaces are listed in a collapsed
  "Archived" group at the bottom of the sidebar (with a count). Each has a
  "Restore" action. Restoring brings the workspace and all its tasks back
  exactly as they were (same status, assignee, notes, comments), because
  archiving never changed them.
- API: `POST /api/workspaces/{id}/archive` and `POST /api/workspaces/{id}/restore`
  (idempotent). DB: nullable `workspaces.archived_at`.

## Epics

- Progress is a bar (done / in progress / open segments) with text such as
  "1 of 3 done". No bare `0/1/2 (3)` numbers.
- Epic detail uses the same metadata layout as task detail.

## Create modal

- Every field has a visible label. Rows have a consistent vertical gap.
- The workspace select is labeled "Workspace".

## Decisions (UI pass 1)

Shell, density and workspace actions. Pass 2 covers board cards, list columns,
task detail layout, epics progress and the create modal fields.

- **Default view.** `/` redirects (307, temporary, so a cached redirect cannot
  stick) to `/board`. Sidebar scope links always open `<scope>/board`; they no
  longer follow the current view. The scope switcher is the sidebar.
- **Top bar.** Full width, sticky, three columns: brand (same width as the
  sidebar, text aligned with sidebar rows) | tabs | actions. The settings gear
  menu is gone; the theme toggle is a sun/moon icon button whose `aria-label`
  and `title` name the action ("Switch to light theme"). The primary button
  keeps the label "New Issue" (existing scenarios assert that text).
- **Layout.** Sidebar 240px, sticky under the top bar, own scroll. Content gutter
  24px (16px below 900px). `main` gets a `view-<tasks|board|epics>` class: lists
  and epics are capped at 1100px, the board is full width; task detail wraps its
  content in `.page-detail` (860px).
- **Below 900px.** The sidebar becomes a left drawer opened by a "Scope" button
  in the top bar (backdrop click, Esc or a link closes it); tabs move to a second
  row. The button only exists on full page loads that render a sidebar.
- **Scope header.** Rendered by `base.html` outside `#main`, on full page loads
  only (HTMX tab switches keep the same scope, so it never needs a refresh). Shows
  `project / workspace` (path in a tooltip) or the project name or "All projects".
  Task detail pages (unscoped URLs) show "All projects".
- **Density.** Variables are overridden once at `:root` in `app.css` (font size
  87.5% = 14px, line height 1.45, spacing 12px, form element spacing 4px/10px,
  radius 4px) plus `--tk-control-h: 32px`. Headings: h1 1.25rem, h2 1.1rem,
  h3 1rem. Pico sets `--pico-font-size` again at wider breakpoints; the later
  `:root` rule wins, so no `!important`.
- **Filter bar.** `.filter-bar` is a flex row (8px gap, wraps; two per row at
  480px and below; search takes a full row on phones). All controls: 32px
  min-height, 4px radius, left-aligned text. The search wrapper is a `<search>`
  element instead of `role="search"` because Pico gives `[role=search]` a pill
  radius and joins its children.
- **Modal.** The footer "View full page" button is replaced by an icon link in
  the header (`aria-label` and `title` "Open in full page").
- **Workspace actions.** A "..." menu (`<details>`) in the scope header with
  "Close all tasks" and "Remove workspace"; both open the shared in-page
  `#ws-dialog` (filled by `app.js`, workspace names set via `textContent`). The
  dialog fetches `GET /api/workspaces/{id}` for the live open count. Close-all
  uses reason `done`, an optional comment (author `user`), then reloads. Remove
  archives and navigates to `<project>/board`. Restore (sidebar) reloads the page.
  Not done: the per-row menu in the sidebar (the header menu covers it).
- **Archived handling in the web UI.** Web scopes use `AllVisible` /
  `ProjectVisible`. `/p/{pid}/w/{wid}/...` of an archived workspace is 404. The
  create and move selects skip archived workspaces except the task's current one
  (shown as "(archived)"). Task detail by URL works and the workspace badge reads
  "archived". Sidebar counts and the "All" total exclude archived workspaces; a
  project whose workspaces are all archived is hidden unless it is the active scope.
- **API.** `GET /api/workspaces/{id}` returns the workspace plus `project_name`
  and `open_count` (tasks not done). `POST .../archive` and `.../restore` return
  the workspace (404 unknown or non-numeric id). `POST .../close-all` takes an
  optional JSON body `{reason, comment}` and returns `{"closed": N}` (422 for an
  invalid reason). `GET /api/workspaces` (and `tk workspaces --json`) gain
  `archived_at`. The JSON API scopes (`?project=`, `?workspace=`, no param) are
  unchanged and still include archived workspaces' tasks; only HTML views hide
  them. All new POSTs go through the same-origin guard.
