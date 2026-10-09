Feature: User feedback loop
  The user comments on tasks (web UI, or `tk comment --author user`). A user comment is
  "pending" (awaiting reply) when its task is not done and no later comment by a non-user
  author exists. Agents see pending comments in `tk show`, `tk prime` and the
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

  Scenario: a done task never has pending comments
    Given I run tk "comment @id(t) 'late request' --author user" in "A"
    And I run tk "close @id(t) --force" in "A"
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "pending_user_comments" is an empty array

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
