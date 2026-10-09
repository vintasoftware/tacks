# User feedback loop

The user comments on tasks in the web UI; agents must see those comments and act on them.

## Author

Every comment has an optional `author` (`comments.author`, schema v4):

- `user`: written in the web UI, or with `tk comment <id> <body> --author user`.
- `agent`: the default for `tk comment` and for `tk close -c`.
- `NULL`: comments created before v4. Treated as agent-authored.

`--author` is free text; only `user` has meaning (user feedback).

## Pending ("awaiting reply")

A user comment is pending when its task is not `done` and no later comment by a non-user
author (including legacy `NULL`) exists on that task. One agent reply answers all earlier
user comments on the task. A new user comment after the reply is pending again. Closing the
task ends pending state.

## Delivery

An internal `comments.delivered_at` column records that a pending comment was already pushed
into an agent session, so the hook does not repeat it. It is not exposed in the CLI or JSON.

- `tk prime` (SessionStart / PreCompact): prints "User feedback awaiting reply" at the top
  (task id, title, body truncated to 300 chars, time) for the current scope, then marks
  those comments delivered. `prime --json` adds `user_feedback: [{task_id, task_title,
  comment_id, body, created_at}]` (always present, empty when none; body is not truncated).
  Prime's silent rules are unchanged.
- `tk show <id>`: comments print as `[time] [author] body`, pending ones end with
  `⚠ awaiting reply`. `show --json` adds `pending_user_comments: [comment ids]` and each
  comment has `author`.
- `tk hook post-tool-use` (plugin PostToolUse hook, matcher `*`, timeout 10): reads the hook
  JSON on stdin, resolves the workspace from `cwd` (`TACKS_WORKSPACE` wins; DB from
  `TACKS_DB` or `~/.tacks/tacks.db`), and when undelivered pending comments exist prints
  `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"..."}}`, then
  marks them delivered. The message stays under 8000 chars (bodies cut to 500 chars); if not
  everything fits, only the comments shown are marked delivered and the rest follow on the
  next tool call. If the database file is missing, the workspace is unregistered, or
  anything fails, it prints nothing and exits 0; it never creates the database. Cost when
  there is nothing to say: one `git rev-parse` plus two queries.

## Close guard

`tk close <id>` exits 1 without writing anything when the task has pending user comments,
listing them briefly and telling the agent to reply with `tk comment` first. `--force`
overrides (also the open-subtask guard); use it only if the user agreed. Order of checks:
workspace write guard, reason validation, feedback guard, open-subtask guard. `-c` on close
does not count as a reply, because the comment is written after the close.

## Agent instructions

Treat user comments as instructions for that task: address them, then reply with
`tk comment <id> "..."`. This is stated in `tk init-rules`, the plugin agent and the
`comment`, `close` and `prime` commands.

## Web UI

The task detail (page and modal) has a comment form; comments post with author `user` and the list
re-renders in place (`GET /tasks/{id}/comments` returns the list fragment). Pending comments show an
"awaiting agent reply" marker, and board cards and list rows show a `💬 N` indicator. The web close
is unrestricted. Security guards: `docs/web-security.md`.
