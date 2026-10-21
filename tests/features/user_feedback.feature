Feature: User feedback loop
  The user comments on tasks (web UI, or `tk comment --author user`). A user comment is
  "pending" (awaiting reply) when no later comment by a non-user author exists and either
  its task is not done or the comment was written after the task was closed. Comments
  written before a close are not pending; reopening the task makes unanswered ones pending
  again. Agents see pending comments in `tk show`, `tk prime` and the
  `tk hook post-tool-use` hook, and `tk close` refuses to close a task with pending feedback
  unless `--force` is given. See docs/user-feedback.md.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a git repository "fresh"
    And I run tk "--json create 'Feedback task'" in "A" and save the id as "t"

  # ---------------------------------------------------------------------------
  # Authors and pending state in show
  # ---------------------------------------------------------------------------

  Scenario: comments default to the agent author
    Given I run tk "comment @id(t) 'agent note'" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "comments.0.author" equals "agent"
    And the JSON output path "pending_user_comments" is an empty array

  Scenario: --author user stores a user comment that is pending
    Given I run tk "comment @id(t) 'please add tests' --author user" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "comments.0.author" equals "user"
    And the JSON output path "pending_user_comments.0" equals "1"

  Scenario: human show marks pending user comments as awaiting reply
    Given I run tk "comment @id(t) 'please add tests' --author user" in "A"
    When I run tk "show @id(t)" in "A"
    Then the tk output contains "please add tests"
    And the tk output contains "awaiting reply"

  Scenario: an agent reply clears the pending state
    Given I run tk "comment @id(t) 'please add tests' --author user" in "A"
    And I run tk "comment @id(t) 'tests added'" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "pending_user_comments" is an empty array
    When I run tk "show @id(t)" in "A"
    Then the tk output does not contain "awaiting reply"

  Scenario: a new user comment after the reply is pending again
    Given I run tk "comment @id(t) 'first request' --author user" in "A"
    And I run tk "comment @id(t) 'done with first'" in "A"
    And I run tk "comment @id(t) 'second request' --author user" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "pending_user_comments.0" equals "3"

  Scenario: one agent reply answers every earlier user comment
    Given I run tk "comment @id(t) 'request one' --author user" in "A"
    And I run tk "comment @id(t) 'request two' --author user" in "A"
    And I run tk "comment @id(t) 'both handled'" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "pending_user_comments" is an empty array

  Scenario: comments written before the close are not pending on a done task
    Given I run tk "comment @id(t) 'late request' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "pending_user_comments" is an empty array

  # ---------------------------------------------------------------------------
  # Feedback on done tasks
  # ---------------------------------------------------------------------------

  Scenario: a user comment written after the close is pending in show
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "pending_user_comments.0" equals "1"

  Scenario: human show marks a post-close user comment as awaiting reply
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    When I run tk "show @id(t)" in "A"
    Then the tk output contains "one more thing"
    And the tk output contains "awaiting reply"

  Scenario: an agent reply clears a post-close user comment on a done task
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    And I run tk "comment @id(t) 'answered'" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "pending_user_comments" is an empty array
    When I run tk "--json prime" in "A"
    Then the JSON output path "user_feedback" is an empty array

  Scenario: a pre-close comment stays not pending when a later user comment is added
    Given I run tk "comment @id(t) 'before close' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    And I run tk "comment @id(t) 'after close' --author user" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "pending_user_comments.0" equals "2"

  Scenario: reopening a done task makes its earlier unanswered user comments pending again
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    And I run tk "update @id(t) -s in_progress" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "in_progress"
    And the JSON output path "pending_user_comments.0" equals "1"

  Scenario: an agent reply after reopening clears the pending state
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    And I run tk "update @id(t) -s in_progress" in "A"
    And I run tk "comment @id(t) 'docs added'" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "pending_user_comments" is an empty array

  Scenario: closing again after a reopen ends the pending state again
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    And I run tk "update @id(t) -s in_progress" in "A"
    And I run tk "close @id(t) --force" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "pending_user_comments" is an empty array

  Scenario: closed_at is internal and not part of the task JSON
    Given I run tk "close @id(t)" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output has no field "closed_at"

  # ---------------------------------------------------------------------------
  # Prime
  # ---------------------------------------------------------------------------

  Scenario: prime lists pending user feedback
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    When I run tk "prime" in "A"
    Then the tk command succeeds
    And the tk output contains "User feedback awaiting reply"
    And the tk output contains resolved "@id(t)"
    And the tk output contains "please rename the flag"

  Scenario: prime --json always has a user_feedback array
    When I run tk "--json prime" in "A"
    Then the tk command succeeds
    And the JSON output path "user_feedback" is an empty array

  Scenario: prime --json lists pending feedback with the full body
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    When I run tk "--json prime" in "A"
    Then the JSON output path "user_feedback.0.task_id" equals "@id(t)"
    And the JSON output path "user_feedback.0.body" equals "please rename the flag"
    And the JSON output path "user_feedback.0.task_title" equals "Feedback task"

  Scenario: prime does not list feedback that was answered
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    And I run tk "comment @id(t) 'renamed'" in "A"
    When I run tk "--json prime" in "A"
    Then the JSON output path "user_feedback" is an empty array

  Scenario: prime does not show feedback on another workspace's task
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    When I run tk "--json prime" in "B"
    Then the JSON output path "user_feedback" is an empty array
    When I run tk "prime" in "B"
    Then the tk output does not contain "please rename the flag"

  Scenario: prime marks a done task with pending feedback and hints how to reopen it
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    When I run tk "prime" in "A"
    Then the tk command succeeds
    And the tk output contains "User feedback awaiting reply"
    And the tk output contains resolved "@id(t) (done)"
    And the tk output contains "one more thing"
    And the tk output contains "tk update <id> -s in_progress"

  Scenario: prime --json lists feedback on a done task
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    When I run tk "--json prime" in "A"
    Then the JSON output path "user_feedback.0.task_id" equals "@id(t)"
    And the JSON output path "user_feedback.0.body" equals "one more thing"

  Scenario: prime does not mark open tasks as done
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    When I run tk "prime" in "A"
    Then the tk output does not contain "(done)"

  Scenario: prime does not list a pre-close comment on a done task
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    When I run tk "--json prime" in "A"
    Then the JSON output path "user_feedback" is an empty array

  Scenario: prime lists a reopened task's earlier user comment again
    Given I run tk "comment @id(t) 'please rename the flag' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    And I run tk "update @id(t) -s in_progress" in "A"
    When I run tk "prime" in "A"
    Then the tk output contains "please rename the flag"
    And the tk output does not contain "(done)"

  # ---------------------------------------------------------------------------
  # Close guard
  # ---------------------------------------------------------------------------

  Scenario: close is refused while user feedback is unanswered
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    When I run tk "close @id(t)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "unanswered user comment"
    And the tk error contains "tk comment"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "open"

  Scenario: close --force overrides the feedback guard
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    When I run tk "close @id(t) --force" in "A"
    Then the tk command succeeds
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"

  Scenario: close works after the agent replies
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    And I run tk "comment @id(t) 'docs added'" in "A"
    When I run tk "close @id(t)" in "A"
    Then the tk command succeeds
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"

  Scenario: closing a done task again is refused while a post-close user comment is pending
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    When I run tk "close @id(t)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "unanswered user comment"

  Scenario: closing a done task again works after the agent replies
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    And I run tk "comment @id(t) 'answered'" in "A"
    When I run tk "close @id(t)" in "A"
    Then the tk command succeeds

  Scenario: the workspace write guard takes precedence over the feedback guard
    Given I run tk "comment @id(t) 'please add docs' --author user" in "A"
    When I run tk "close @id(t)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"
    And the tk error does not contain "unanswered user comment"

  # ---------------------------------------------------------------------------
  # PostToolUse hook
  # ---------------------------------------------------------------------------

  Scenario: the hook delivers pending feedback once
    Given I run tk "comment @id(t) 'use the new API' --author user" in "A"
    When I run the post-tool-use hook in "A"
    Then the hook delivers context containing "@id(t)"
    And the hook delivers context containing "use the new API"
    When I run the post-tool-use hook in "A"
    Then the hook prints nothing and exits 0

  Scenario: the hook delivers feedback on a done task with a done marker
    Given I run tk "close @id(t)" in "A"
    And I run tk "comment @id(t) 'one more thing' --author user" in "A"
    When I run the post-tool-use hook in "A"
    Then the hook delivers context containing "@id(t) (done)"
    And the hook delivers context containing "one more thing"
    And the hook delivers context containing "tk update <id> -s in_progress"
    When I run the post-tool-use hook in "A"
    Then the hook prints nothing and exits 0

  Scenario: the hook does not mark open tasks as done
    Given I run tk "comment @id(t) 'use the new API' --author user" in "A"
    When I run the post-tool-use hook in "A"
    Then the hook delivers context containing "@id(t) (Feedback task)"

  Scenario: the hook is silent for a pre-close comment on a done task
    Given I run tk "comment @id(t) 'use the new API' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    When I run the post-tool-use hook in "A"
    Then the hook prints nothing and exits 0

  Scenario: the hook delivers a new user comment on the next call
    Given I run tk "comment @id(t) 'first' --author user" in "A"
    And I run the post-tool-use hook in "A"
    And I run tk "comment @id(t) 'second thought' --author user" in "A"
    When I run the post-tool-use hook in "A"
    Then the hook delivers context containing "second thought"
    And the hook context does not contain "first"

  Scenario: the hook is silent when there is no feedback
    When I run the post-tool-use hook in "A"
    Then the hook prints nothing and exits 0

  Scenario: feedback printed by prime is not repeated by the hook
    Given I run tk "comment @id(t) 'use the new API' --author user" in "A"
    And I run tk "prime" in "A"
    When I run the post-tool-use hook in "A"
    Then the hook prints nothing and exits 0

  Scenario: the hook is silent for an unregistered workspace
    Given I run tk "comment @id(t) 'use the new API' --author user" in "A"
    When I run the post-tool-use hook in "fresh"
    Then the hook prints nothing and exits 0

  Scenario: the hook never creates a missing database
    Given I delete the database file
    When I run the post-tool-use hook in "A"
    Then the hook prints nothing and exits 0
    And the database file does not exist

  Scenario: the hook ignores invalid JSON on stdin
    Given I run tk "comment @id(t) 'use the new API' --author user" in "A"
    When I run the post-tool-use hook in "A" with raw stdin "this is { not json"
    Then the hook prints nothing and exits 0

  # ---------------------------------------------------------------------------
  # Author validation and atomic close
  # ---------------------------------------------------------------------------

  Scenario: a blank author is rejected
    When I run tk "comment @id(t) 'note' --author '  '" in "A"
    Then the tk command fails with exit code 1
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "comments" is an empty array

  Scenario: an author longer than 64 characters is rejected
    When I run tk "comment @id(t) 'note' --author aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" in "A"
    Then the tk command fails with exit code 1
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "comments" is an empty array

  Scenario: the author is trimmed before it is stored
    Given I run tk "comment @id(t) 'note' --author ' alice '" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "comments.0.author" equals "alice"

  Scenario: closing with a comment stores it as the agent and closes the task
    When I run tk "close @id(t) -c 'all done'" in "A"
    Then the tk command succeeds
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "comments.0.body" equals "all done"
    And the JSON output path "comments.0.author" equals "agent"

  # ---------------------------------------------------------------------------
  # Web close writes the user's comment before closing
  # ---------------------------------------------------------------------------

  Scenario: the web close comment is not pending feedback
    Given the web server is running
    When I POST resolved "/api/tasks/@id(t)/close" with body '{"comment":"wrapping up"}'
    Then the response status is 200
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "comments.0.body" equals "wrapping up"
    And the JSON output path "comments.0.author" equals "user"
    And the JSON output path "pending_user_comments" is an empty array
    When I run tk "--json prime" in "A"
    Then the JSON output path "user_feedback" is an empty array

  Scenario: a user comment posted on the web after a web close is pending
    Given the web server is running
    When I POST resolved "/api/tasks/@id(t)/close" with body '{"comment":"wrapping up"}'
    And I run tk "comment @id(t) 'actually, one more thing' --author user" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "pending_user_comments.0" equals "2"

  Scenario: the web workspace close-all comment is not pending feedback
    Given the web server is running
    When I POST resolved "/api/workspaces/@workspace(A)/close-all" with body '{"comment":"wrap up"}'
    Then the response status is 200
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "comments.0.body" equals "wrap up"
    And the JSON output path "pending_user_comments" is an empty array

  # ---------------------------------------------------------------------------
  # Agent rules
  # ---------------------------------------------------------------------------

  Scenario: init-rules writes the definition of done and done-task feedback guidance
    When I run tk "init-rules" in "fresh"
    Then the tk command succeeds
    And the file ".claude/rules/tacks.md" in "fresh" contains "## Definition of done"
    And the file ".claude/rules/tacks.md" in "fresh" contains "tk update <id> -s in_progress"

