Feature: Auto-archive of removed workspaces
  A workspace is archived automatically (archived_at is set) when it is not archived,
  was never restored, has no task that is not done (zero tasks qualifies) and its
  directory no longer exists on disk. The check runs from `tk workspaces`,
  `GET /api/workspaces` and when the web sidebar is built. Tasks are never modified.
  See docs/web-ui-design.md ("Auto-archive").

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And I run tk "--json create 'Beta open'" in "B"

  Scenario: a removed workspace with all tasks done is archived
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    And I remove the directory "A"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" has "missing" equal to "true"
    And the workspaces JSON row for "A" is archived

  Scenario: a removed workspace with an open task is not archived
    Given I run tk "--json create 'Alpha open'" in "A"
    And I remove the directory "A"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" has "archived_at" equal to "null"

  Scenario: a removed workspace with an in-progress task is not archived
    Given I run tk "--json create 'Alpha wip'" in "A" and save the id as "a1"
    And I run tk "update @id(a1) --claim" in "A"
    And I remove the directory "A"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" has "archived_at" equal to "null"

  Scenario: a workspace whose directory still exists is not archived
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" has "archived_at" equal to "null"

  Scenario: a removed workspace with no tasks is archived
    Given I run tk "--json create 'Moved away'" in "A" and save the id as "moved"
    And I run tk "--scope project update @id(moved) --move-to @path(B)" in "A"
    And I remove the directory "A"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" is archived

  Scenario: auto-archiving leaves the tasks of the workspace unchanged
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    And I remember the updated_at of task "a1"
    And I remove the directory "A"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" is archived
    When I run tk "--json show @id(a1)" in "B"
    Then the JSON output path "status" equals "done"
    And the updated_at of task "a1" is unchanged

  Scenario: a restored workspace is not archived again after its directory is removed
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    And the web server is running
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(A)/restore" with body "{}"
    Then the response status is 200
    When I remove the directory "A"
    And I GET resolved "/board"
    And I GET resolved "/api/workspaces"
    Then the workspaces response row for "A" has "archived_at" equal to "null"
    When I run tk "--json workspaces" in "B"
    Then the workspaces JSON row for "A" has "archived_at" equal to "null"

  Scenario: the web sidebar moves a removed all-done workspace into the Archived group
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    And the web server is running
    When I GET resolved "/board"
    Then the sidebar lists workspace "A"
    And the response body does not contain "Archived"
    When I remove the directory "A"
    And I GET resolved "/board"
    Then the response status is 200
    And the response body contains "Archived"
    And the response body does not contain resolved 'data-scope-prefix="/p/@project(A)/w/@workspace(A)"'
    And the response body contains resolved 'data-ws-restore="@workspace(A)"'
    And the sidebar lists workspace "B"

  Scenario: GET /api/workspaces archives a removed all-done workspace
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    And the web server is running
    And I remove the directory "A"
    When I GET resolved "/api/workspaces"
    Then the response status is 200
    And the workspaces response row for "A" is archived

  Scenario: tk workspaces marks an auto-archived workspace in the human-readable output
    Given I run tk "--json create 'Alpha done'" in "A" and save the id as "a1"
    And I run tk "close @id(a1)" in "A"
    And I remove the directory "A"
    When I run tk "workspaces" in "B"
    Then the tk output contains "(archived)"
