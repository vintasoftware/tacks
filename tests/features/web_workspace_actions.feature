Feature: Workspace actions and pass-2 views in the web UI
  The web UI can archive ("remove") a workspace, restore it, and close all of its
  tasks. Archiving only hides the workspace from HTML views; tasks and the CLI are
  untouched. See docs/web-ui-design.md ("Workspace actions", "Decisions (UI pass 1/2)").

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And I run tk "--json create 'Alpha one'" in "A" and save the id as "a1"
    And I run tk "--json create 'Alpha two'" in "A" and save the id as "a2"
    And I run tk "--json create 'Alpha done'" in "A" and save the id as "a3"
    And I run tk "--json create 'Beta one'" in "B" and save the id as "b1"
    And the web server is running

  # ---------------------------------------------------------------------------
  # Default view
  # ---------------------------------------------------------------------------

  Scenario: the root URL redirects temporarily to the board
    When I GET "/" without following redirects it answers 307 with location "/board"

  # ---------------------------------------------------------------------------
  # Archive
  # ---------------------------------------------------------------------------

  Scenario: archiving a workspace sets archived_at
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    Then the response status is 200
    And the response JSON path "archived_at" is set

  Scenario: archiving twice is idempotent
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    Then the response status is 200
    And the response JSON path "archived_at" is set

  Scenario: archiving an unknown workspace returns 404
    When I POST resolved "/api/workspaces/999999/archive" with body "{}"
    Then the response status is 404

  Scenario: the board of an archived workspace is 404
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I GET resolved "/p/@project(A)/w/@workspace(A)/board"
    Then the response status is 404

  Scenario Outline: tasks of an archived workspace are hidden from <view>
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I GET resolved "<view>"
    Then the response status is 200
    And the response body does not contain "Alpha one"
    And the response body does not contain "Alpha two"
    And the response body contains "Beta one"

    Examples:
      | view                  |
      | /board                |
      | /p/@project(A)/board  |
      | /tasks                |

  Scenario: the sidebar moves an archived workspace into the Archived group
    When I GET resolved "/board"
    Then the sidebar lists workspace "A"
    And the response body does not contain "Archived"
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I GET resolved "/board"
    Then the response body does not contain resolved 'data-scope-prefix="/p/@project(A)/w/@workspace(A)"'
    And the sidebar lists workspace "B"
    And the response body contains "Archived"
    And the response body contains resolved 'data-ws-restore="@workspace(A)"'

  Scenario: task detail of an archived workspace still works and says archived
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I GET resolved "/tasks/@id(a1)"
    Then the response status is 200
    And the response body contains "Alpha one"
    And the response body contains "archived"

  Scenario: the CLI still lists tasks of an archived workspace
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I run tk "--json list --scope all" in "A"
    Then the JSON output lists exactly the titles "Alpha one,Alpha two,Alpha done,Beta one"

  Scenario: tk workspaces marks an archived workspace
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I run tk "workspaces" in "A"
    Then the tk output contains "(archived)"

  Scenario: the workspaces API exposes archived_at
    When I GET resolved "/api/workspaces"
    Then the workspaces response row for "A" has "archived_at" equal to "null"
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I GET resolved "/api/workspaces"
    Then the response status is 200
    And the response body contains "archived_at"
    And the workspaces response row for "B" has "archived_at" equal to "null"

  # ---------------------------------------------------------------------------
  # Restore
  # ---------------------------------------------------------------------------

  Scenario: restoring brings the tasks back unchanged
    Given I run tk "update @id(a1) --claim --assignee alice --notes 'remember the milk'" in "A"
    And I run tk "comment @id(a1) 'first comment'" in "A"
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(A)/restore" with body "{}"
    Then the response status is 200
    And the response JSON path "archived_at" equals "null"
    When I GET resolved "/board"
    Then the response body contains "Alpha one"
    And the response body contains "Alpha two"
    When I run tk "--json show @id(a1)" in "A"
    Then the JSON output path "status" equals "in_progress"
    And the JSON output path "assignee" equals "alice"
    And the JSON output path "notes" equals "remember the milk"
    And the JSON output path "comments.0.body" equals "first comment"

  Scenario: restoring twice is idempotent
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I POST resolved "/api/workspaces/@workspace(A)/restore" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(A)/restore" with body "{}"
    Then the response status is 200
    And the response JSON path "archived_at" equals "null"

  Scenario: restoring an unknown workspace returns 404
    When I POST resolved "/api/workspaces/999999/restore" with body "{}"
    Then the response status is 404

  Scenario: creating a task in an archived worktree restores the workspace
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    When I run tk "--json create 'Alpha again'" in "A"
    And I GET resolved "/p/@project(A)/w/@workspace(A)/board"
    Then the response status is 200
    And the response body contains "Alpha again"
    And the response body contains "Alpha one"
    And the sidebar lists workspace "A"

  # ---------------------------------------------------------------------------
  # Close all
  # ---------------------------------------------------------------------------

  Scenario: the workspace API reports the open count
    Given I run tk "close @id(a3)" in "A"
    When I GET resolved "/api/workspaces/@workspace(A)"
    Then the response status is 200
    And the response JSON path "open_count" equals "2"

  Scenario: close-all closes only the non-done tasks of that workspace
    Given I run tk "close @id(a3)" in "A"
    And I remember the updated_at of task "a3"
    When I POST resolved "/api/workspaces/@workspace(A)/close-all" with body "{}"
    Then the response status is 200
    And the response JSON path "closed" equals "2"
    And the updated_at of task "a3" is unchanged
    When I run tk "--json show @id(a1)" in "A"
    Then the JSON output path "status" equals "done"
    When I run tk "--json show @id(a2)" in "A"
    Then the JSON output path "status" equals "done"
    When I run tk "--json show @id(b1)" in "B"
    Then the JSON output path "status" equals "open"

  Scenario: close-all with a comment adds it as the user
    When I POST resolved "/api/workspaces/@workspace(A)/close-all" with body '{"comment":"wrap up"}'
    Then the response JSON path "closed" equals "3"
    When I run tk "--json show @id(a1)" in "A"
    Then the JSON output path "comments.0.body" equals "wrap up"
    And the JSON output path "comments.0.author" equals "user"
    When I run tk "--json show @id(a2)" in "A"
    Then the JSON output path "comments.0.body" equals "wrap up"
    And the JSON output path "comments.0.author" equals "user"

  Scenario: repeating close-all closes nothing
    When I POST resolved "/api/workspaces/@workspace(A)/close-all" with body "{}"
    When I POST resolved "/api/workspaces/@workspace(A)/close-all" with body "{}"
    Then the response status is 200
    And the response JSON path "closed" equals "0"

  Scenario: close-all with an invalid reason returns 422
    When I POST resolved "/api/workspaces/@workspace(A)/close-all" with body '{"reason":"bogus"}'
    Then the response status is 422
    When I run tk "--json show @id(a1)" in "A"
    Then the JSON output path "status" equals "open"

  Scenario: close-all on an unknown workspace returns 404
    When I POST resolved "/api/workspaces/999999/close-all" with body "{}"
    Then the response status is 404

  # ---------------------------------------------------------------------------
  # Cross-origin guard
  # ---------------------------------------------------------------------------

  Scenario Outline: a cross-origin POST to <action> is rejected
    When I POST resolved "/api/workspaces/@workspace(A)/<action>" with body "{}" from a foreign origin
    Then the response status is 403
    When I run tk "--json show @id(a1)" in "A"
    Then the JSON output path "status" equals "open"

    Examples:
      | action    |
      | archive   |
      | restore   |
      | close-all |

  Scenario: a cross-origin archive does not archive the workspace
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}" from a foreign origin
    Then the response status is 403
    When I GET resolved "/p/@project(A)/w/@workspace(A)/board"
    Then the response status is 200

  # ---------------------------------------------------------------------------
  # Pass 2 views
  # ---------------------------------------------------------------------------

  Scenario: a board card shows the task id and a blocked chip
    Given I run tk "dep add @id(a1) @id(a2)" in "A"
    When I GET resolved "/p/@project(A)/w/@workspace(A)/board"
    Then the response status is 200
    And the response body contains resolved "@id(a1)"
    And the response body contains "blocked by 1"

  Scenario: the task list has a Workspace column in All scope
    When I GET resolved "/tasks"
    Then the response body contains "<th>Workspace</th>"

  Scenario: the task list has a Workspace column in project scope
    When I GET resolved "/p/@project(A)/tasks"
    Then the response body contains "<th>Workspace</th>"

  Scenario: the task list has no Workspace column in workspace scope
    When I GET resolved "/p/@project(A)/w/@workspace(A)/tasks"
    Then the response status is 200
    And the response body contains "Alpha one"
    And the response body does not contain "<th>Workspace</th>"

  Scenario: task detail shows agent notes and the assignee
    Given I run tk "update @id(a1) --assignee alice --notes 'halfway through the parser'" in "A"
    When I GET resolved "/tasks/@id(a1)"
    Then the response status is 200
    And the response body contains "Agent notes"
    And the response body contains "halfway through the parser"
    And the response body contains "alice"

  Scenario: task detail without notes has no Agent notes section
    When I GET resolved "/tasks/@id(a1)"
    Then the response status is 200
    And the response body does not contain "Agent notes"

  Scenario: task detail of a closed task shows the close reason
    Given I run tk "close @id(a1) -r duplicate" in "A"
    When I GET resolved "/tasks/@id(a1)"
    Then the response status is 200
    And the response body contains "Close reason"
    And the response body contains "duplicate"

  Scenario: the epics list shows progress text
    Given I run tk "--json create 'Epic of A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Kid 1'  --parent @id(epic)" in "A" and save the id as "k1"
    And I run tk "--json create 'Kid 2' --parent @id(epic)" in "A"
    And I run tk "--json create 'Kid 3' --parent @id(epic)" in "A"
    And I run tk "close @id(k1)" in "A"
    When I GET resolved "/p/@project(A)/w/@workspace(A)/epics"
    Then the response status is 200
    And the response body contains "Epic of A"
    And the response body contains "1 of 3 done"

  Scenario: the create modal has a Workspace label
    When I GET "/tasks/new/modal"
    Then the response status is 200
    And the response body contains "Workspace"
