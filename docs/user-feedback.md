# User feedback loop

The user comments on tasks in the web UI; agents must see those comments and act on them.

## Author

Every comment has an optional `author` (`comments.author`, schema v4):

- `user`: written in the web UI, or with `tk comment <id> <body> --author user`.
- `agent`: the default for `tk comment` and for `tk close -c`.
- `NULL`: comments created before v4. Treated as agent-authored.

`--author` is free text; only `user` has meaning (user feedback). The CLI trims it and
rejects an empty value or one longer than 64 characters (same rules as the web UI).

Trust model: tacks is a local, single-user tool. The author is a label, not authentication;
anything running as the same OS user (including an agent) can write `--author user` or
answer feedback with a non-user comment, and the DB can be edited directly. The rules text
(`tk init-rules`), the plugin agent and the `comment` command tell agents never to use
`--author user`; that is a convention, not an enforced boundary.

## Pending ("awaiting reply")

A user comment is pending when no later comment by a non-user author (including legacy
`NULL`) exists on that task, and either the task is not `done` or the comment was created
after the task's `closed_at`. One agent reply answers all earlier user comments on the task.
A new user comment after the reply is pending again.

`tasks.closed_at` (schema v8, nullable, internal: not in the CLI or JSON) is set when the
status becomes `done`, kept when a done task is closed again, and cleared when the task is
reopened. Consequences:

- Comments written before the close are not pending: closing ends the pending state of
  everything the user said so far. This includes `tk close --force` and the web close
  comment.
- A user comment on a `done` task, written after the close, is pending. The agent sees it
  in `tk prime` and the hook (marked `(done)`), and `tk close` guards it as for any
  task.
- Reopening a task (`tk update <id> -s in_progress`) clears `closed_at`, so its unanswered
  earlier user comments are pending again until the agent replies.
- Legacy done tasks (closed before v8) have `closed_at` NULL and never have pending
  comments, so upgrading does not resurface old comments.
- The web close (`POST /api/tasks/:id/close` with a comment) writes the user's comment
  before closing the task, in one transaction, so it predates `closed_at` and is not
  pending. `close_workspace_tasks` does the same with its comment.

## Delivery

An internal `comments.delivered_at` column records that a pending comment was already pushed
into an agent session, so the hook does not repeat it. It is not exposed in the CLI or JSON.

- `tk prime` (SessionStart / PreCompact): prints "User feedback awaiting reply" at the top
  (task id, title, body truncated to 300 chars, time) for the current scope, then claims
  the undelivered ones (marks them delivered, see below). Prime still shows all pending ones. `prime --json` adds `user_feedback: [{task_id, task_title,
  comment_id, body, created_at}]` (always present, empty when none; body is not truncated).
  Prime's silent rules are unchanged.
- `tk show <id>`: comments print as `[time] [author] body`, pending ones end with
  `⚠ awaiting reply`. `show --json` adds `pending_user_comments: [comment ids]` and each
  comment has `author`.
- `tk hook post-tool-use` (plugin PostToolUse hook, matcher `*`, timeout 10): reads the hook
  JSON on stdin, resolves the workspace from `cwd` (`TACKS_WORKSPACE` wins; DB from
  `TACKS_DB` or `~/.tacks/tacks.db`), and when undelivered pending comments exist prints
  `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"..."}}`. The message stays under 8000 chars (bodies cut to 500 chars); if not
  everything fits, only the comments shown are marked delivered and the rest follow on the
  next tool call. If the database file is missing, the workspace is unregistered, or
  anything fails, it prints nothing and exits 0; it never creates the database. Stdin is
  read up to 1 MiB.

Claiming is atomic: `claim_undelivered_pending_user_comments` runs in one `BEGIN IMMEDIATE`
transaction, lists the undelivered pending comments of the scope, and marks each one it
will print with `UPDATE ... WHERE delivered_at IS NULL`, keeping only rows that changed. Parallel hook
processes (parallel tool calls) and `tk prime` therefore never deliver the same comment
twice via the hook; only what fits in the 8000-char budget is claimed.

Cost when nothing is pending: the hook checks the DB file exists, opens it read-only (no
WAL pragma, no migration, no git) and runs one query "any undelivered pending user comment?".
Only when that says yes does it resolve the workspace via git and take the write lock. A
database at an older schema (no `author`/`delivered_at` columns) simply yields "nothing".

## Close guard

`tk close <id>` exits 1 without writing anything when the task has pending user comments,
listing them briefly and telling the agent to reply with `tk comment` first. `--force`
overrides (also the open-subtask guard); use it only if the user agreed. Order of checks:
workspace write guard, reason validation, feedback guard, open-subtask guard. `-c` on close
does not count as a reply, because the comment is written after the close.

## Agent instructions

Treat user comments as instructions for that task: address them, then reply with
`tk comment <id> "..."`. Comments are delivered on tasks of any status, including `done`
(`tk prime` and the hook mark them `(done)` and hint how to reopen). On a `done` task, if
the comment needs more work, reopen it (`tk update <id> -s in_progress`), do the work, reply
with `tk comment`, then close it again; if it needs only an answer, reply and leave it
`done`.

Definition of done: a task is done when the agent has written the code and verified it per
the project's rules (tests, lint, ...). The agent closes it then, and does not keep it
`in_progress` or `open` waiting for a commit, push, PR, review, merge or deploy, unless the
task explicitly asks for them. If the user wants more work, they comment on the task.

This is stated in `tk init-rules`, the plugin agent and the `comment`, `close`, `prime` and
`update` commands.

## Web UI

The task detail (page and modal) has a comment form; comments post with author `user` and the list
re-renders in place (`GET /tasks/{id}/comments` returns the list fragment). Pending comments show an
"awaiting agent reply" marker, and board cards and list rows show a `💬 N` indicator. The web close
is unrestricted. Security guards: `docs/web-security.md`.
