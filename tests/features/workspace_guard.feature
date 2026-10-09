Feature: Workspace write guard
  ID-based write commands (update, close, comment, dep add/remove, create --parent)
  refuse, with exit 1 and nothing written, to touch a task outside the current
  workspace. The global --scope flag widens what is allowed: "project" permits other
  workspaces of the same project, "all" permits anything. Reads by ID (show, children)
  and the web UI / JSON API are not restricted.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a git repository "other"
    And a worktree "C" of repository "other"
    And a directory "nogit" outside any git repository

  # ---------------------------------------------------------------------------
  # Refusals from another workspace of the same project
  # ---------------------------------------------------------------------------

  Scenario Outline: <command> is refused for a task of another workspace
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    And I remember the updated_at of task "ta"
    When I run tk "<command>" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A (project repo)"
    And the tk error contains "do not act on it unless the user explicitly asked"
    And the tk error contains "--scope project"
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "title" equals "Task in A"
    And the JSON output path "status" equals "open"
    And the JSON output path "priority" equals "2"
    And the JSON output path "assignee" equals "null"
    And the JSON output path "notes" equals "null"
    And the JSON output path "close_reason" equals "null"
    And the JSON output path "comments" equals "[]"
    And the JSON output path "workspace_id" equals "@workspace(A)"
    And the updated_at of task "ta" is unchanged

    Examples:
      | command                              |
      | update @id(ta) --title Renamed       |
      | update @id(ta) -p 0                  |
      | update @id(ta) --claim               |
      | update @id(ta) --notes secret-notes  |
      | update @id(ta) --move-to @path(B)    |
      | update @id(ta) --move-to none        |
      | close @id(ta)                        |
      | close @id(ta) -r stale               |
      | comment @id(ta) hello                |

  Scenario: update --parent is refused when the task is out of scope
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    And I run tk "--json create 'Epic in B'" in "B" and save the id as "epic"
    When I run tk "update @id(ta) --parent @id(epic)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "parent_id" equals "null"
    And the JSON output path "workspace_id" equals "@workspace(A)"

  Scenario: update --parent is refused when the new parent is out of scope
    Given I run tk "--json create 'Epic in A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Mover'" in "B" and save the id as "mover"
    When I run tk "update @id(mover) --parent @id(epic)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"
    And the tk error contains "do not act on it unless the user explicitly asked"
    And the task "mover" has workspace_id "@workspace(B)"
    When I run tk "--json show @id(mover)" in "B"
    Then the JSON output path "parent_id" equals "null"

  Scenario: promoting a subtask of another workspace with --parent none is refused
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    And I run tk "--json create 'Child' --parent @id(parent)" in "A" and save the id as "child"
    When I run tk "update @id(child) --parent none" in "B"
    Then the tk command fails with exit code 1
    When I run tk "--json show @id(child)" in "B"
    Then the JSON output path "parent_id" equals "@id(parent)"

  Scenario: --move-to together with --parent is refused for a task of another workspace
    Given I run tk "--json create 'Epic in A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "update @id(ta) --parent @id(epic) --move-to @path(C)" in "B"
    Then the tk command fails with exit code 1
    And the task "ta" has workspace_id "@workspace(A)"

  Scenario: create --parent is refused when the parent is in another workspace
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    When I run tk "create Child --parent @id(parent)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A (project repo)"
    When I run tk "--json children @id(parent)" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: dep add is refused when the child task is out of scope
    Given I run tk "--json create 'In A'" in "A" and save the id as "ta"
    And I run tk "--json create 'In B'" in "B" and save the id as "tb"
    When I run tk "dep add @id(ta) @id(tb)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "blockers" equals "[]"

  Scenario: dep add is refused when the parent task is out of scope
    Given I run tk "--json create 'In A'" in "A" and save the id as "ta"
    And I run tk "--json create 'In B'" in "B" and save the id as "tb"
    When I run tk "dep add @id(tb) @id(ta)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"
    When I run tk "--json show @id(tb)" in "B"
    Then the JSON output path "blockers" equals "[]"

  Scenario: dep remove is refused when a task is out of scope
    Given I run tk "--json create 'In A'" in "A" and save the id as "ta"
    And I run tk "--json create 'In A too'" in "A" and save the id as "tb"
    And I run tk "dep add @id(ta) @id(tb)" in "A"
    When I run tk "dep remove @id(ta) @id(tb)" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"
    When I run tk "--json show @id(ta)" in "A"
    Then the JSON output path "blockers.0.id" equals "@id(tb)"

  Scenario: a task of a missing workspace is refused without --scope
    Given I run tk "--json create 'Orphaned task'" in "B" and save the id as "orphan"
    And I remove the directory "B"
    When I run tk "update @id(orphan) --move-to @path(A)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "do not act on it unless the user explicitly asked"
    And the task "orphan" has workspace_id "@workspace(B)"

  Scenario: an unknown id is not a scope error
    When I run tk "close tk-nope" in "A"
    Then the tk command fails with exit code 1
    And the tk error does not contain "do not act on it"

  # ---------------------------------------------------------------------------
  # Allowed with --scope
  # ---------------------------------------------------------------------------

  Scenario Outline: <command> is allowed with --scope project
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--scope project <command>" in "B"
    Then the tk command succeeds

    Examples:
      | command                          |
      | update @id(ta) --title Renamed   |
      | update @id(ta) --notes some-note |
      | comment @id(ta) hello            |
      | close @id(ta)                    |

  Scenario: --scope project update really changes the task
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--scope project update @id(ta) --title Renamed" in "B"
    Then the tk command succeeds
    And the task "ta" has title "Renamed"

  Scenario: --scope project allows dep add and remove across workspaces
    Given I run tk "--json create 'In A'" in "A" and save the id as "ta"
    And I run tk "--json create 'In B'" in "B" and save the id as "tb"
    When I run tk "--scope project dep add @id(ta) @id(tb)" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "blockers.0.id" equals "@id(tb)"
    When I run tk "--scope project dep remove @id(ta) @id(tb)" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "blockers" equals "[]"

  Scenario: --scope project allows create --parent under another workspace's task
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    When I run tk "--json --scope project create Child --parent @id(parent)" in "B"
    Then the tk command succeeds
    And the JSON output path "workspace_id" equals "@workspace(A)"

  Scenario: --scope project allows moving a task of another workspace into this one
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--scope project update @id(ta) --move-to @path(B)" in "B"
    Then the tk command succeeds
    And the task "ta" has workspace_id "@workspace(B)"

  # ---------------------------------------------------------------------------
  # Other project and unscoped tasks
  # ---------------------------------------------------------------------------

  Scenario: a task of another project is refused and the hint says --scope all
    Given I run tk "--json create 'Task in C'" in "C" and save the id as "tc"
    When I run tk "update @id(tc) --claim" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace C (project other)"
    And the tk error contains "do not act on it unless the user explicitly asked"
    And the tk error contains "rerun with --scope all"

  Scenario: --scope project does not reach another project
    Given I run tk "--json create 'Task in C'" in "C" and save the id as "tc"
    When I run tk "--scope project update @id(tc) --claim" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "--scope all"
    And the task "tc" has workspace_id "@workspace(C)"

  Scenario: --scope all reaches a task of another project
    Given I run tk "--json create 'Task in C'" in "C" and save the id as "tc"
    When I run tk "--scope all update @id(tc) --claim" in "A"
    Then the tk command succeeds
    When I run tk "--json show @id(tc)" in "A"
    Then the JSON output path "status" equals "in_progress"

  Scenario: an unscoped task is refused from a worktree
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "update @id(loose) --claim" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "is unscoped (no workspace, no project)"
    And the tk error contains "do not act on it unless the user explicitly asked"
    And the tk error contains "--scope all"
    When I run tk "--json show @id(loose)" in "A"
    Then the JSON output path "status" equals "open"

  Scenario: --scope project does not reach an unscoped task
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "--scope project close @id(loose)" in "A"
    Then the tk command fails with exit code 1

  Scenario: --scope all reaches an unscoped task
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "--scope all close @id(loose)" in "A"
    Then the tk command succeeds

  Scenario: a worktree task is refused from outside any repository
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "close @id(ta)" in "nogit"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A"

  Scenario: tasks in the current workspace are unaffected
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "update @id(ta) --claim" in "A"
    Then the tk command succeeds
    When I run tk "comment @id(ta) hello" in "A"
    Then the tk command succeeds
    When I run tk "close @id(ta)" in "A"
    Then the tk command succeeds

  # ---------------------------------------------------------------------------
  # Unrestricted reads and web
  # ---------------------------------------------------------------------------

  Scenario: show works across workspaces without a flag
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--json show @id(ta)" in "B"
    Then the tk command succeeds
    And the JSON output path "title" equals "Task in A"

  Scenario: children works across workspaces without a flag
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    And I run tk "--json create 'Child' --parent @id(parent)" in "A"
    When I run tk "--json children @id(parent)" in "B"
    Then the tk command succeeds
    And the JSON output lists exactly the titles "Child"

  Scenario: the web API can still update a task of any workspace
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    And the web server is running
    When I PATCH resolved "/api/tasks/@id(ta)" with body '{"title":"Renamed via web"}'
    Then the response status is 200
    And the task "ta" has title "Renamed via web"
