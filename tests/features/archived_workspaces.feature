Feature: Archived workspaces page in the web UI
  The sidebar does not list archived workspaces. When at least one workspace is
  archived it shows a single link "Archived workspaces (N)" to /archived. That page
  lists the archived workspaces of every project, grouped by project, each with a
  Restore button, and says "No archived workspaces." when there are none.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a git repository "other"
    And a worktree "C" of repository "other"
    And I run tk "--json create 'Alpha task'" in "A" and save the id as "alpha"
    And I run tk "--json create 'Beta task'" in "B" and save the id as "beta"
    And I run tk "--json create 'Gamma task'" in "C" and save the id as "gamma"
    And the web server is running

  Scenario: the archived page lists archived workspaces of different projects with their project names
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(C)/archive" with body "{}"
    And I GET "/archived"
    Then the response status is 200
    And the response body contains resolved 'data-ws-restore="@workspace(A)"'
    And the response body contains resolved 'data-ws-restore="@workspace(C)"'
    And the response body does not contain resolved 'data-ws-restore="@workspace(B)"'
    And the response body contains '<h3 class="archived-project">repo</h3>'
    And the response body contains '<h3 class="archived-project">other</h3>'
    And the response body does not contain "No archived workspaces."

  Scenario: the archived page shows an empty state when nothing is archived
    When I GET "/archived"
    Then the response status is 200
    And the response body contains "No archived workspaces."
    And the response body does not contain "data-ws-restore"

  Scenario: a restored workspace no longer shows on the archived page
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I GET "/archived"
    Then the response body contains resolved 'data-ws-restore="@workspace(A)"'
    When I POST resolved "/api/workspaces/@workspace(A)/restore" with body "{}"
    And I GET "/archived"
    Then the response status is 200
    And the response body does not contain resolved 'data-ws-restore="@workspace(A)"'
    And the response body contains "No archived workspaces."

  Scenario: the sidebar has no archived link when nothing is archived
    When I GET "/board"
    Then the response status is 200
    And the response body does not contain "Archived workspaces"

  Scenario: the sidebar shows the archived link once a workspace is archived
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I GET "/board"
    Then the response body contains "Archived workspaces (1)"
    And the response body contains 'href="/archived"'

  Scenario: the sidebar archived link counts every archived workspace
    When I POST resolved "/api/workspaces/@workspace(A)/archive" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(C)/archive" with body "{}"
    And I GET "/board"
    Then the response body contains "Archived workspaces (2)"
