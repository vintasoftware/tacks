Feature: Workspace scoping in the web UI and API
  One tk serve instance shows every project and workspace. The scope lives in the
  URL path (/p/{project}/w/{workspace}/...) and in optional query parameters on
  the JSON API.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a git repository "other"
    And a worktree "C" of repository "other"
    And a directory "nogit" outside any git repository
    And I run tk "--json create 'Alpha task'" in "A" and save the id as "alpha"
    And I run tk "--json create 'Beta task'" in "B" and save the id as "beta"
    And I run tk "--json create 'Gamma task'" in "C" and save the id as "gamma"
    And I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    And the web server is running

  # ---------------------------------------------------------------------------
  # Scoped pages
  # ---------------------------------------------------------------------------

  Scenario Outline: the unscoped <view> page shows every task
    When I GET resolved "/<view>"
    Then the response status is 200
    And the response body contains "Alpha task"
    And the response body contains "Beta task"
    And the response body contains "Gamma task"
    And the response body contains "Loose task"

    Examples:
      | view  |
      | board |
      | tasks |

  Scenario Outline: a workspace-scoped <view> page only shows that workspace's tasks
    When I GET resolved "/p/@project(A)/w/@workspace(A)/<view>"
    Then the response status is 200
    And the response body contains "Alpha task"
    And the response body does not contain "Beta task"
    And the response body does not contain "Gamma task"
    And the response body does not contain "Loose task"

    Examples:
      | view  |
      | board |
      | tasks |

  Scenario Outline: a project-scoped <view> page shows every workspace of the project
    When I GET resolved "/p/@project(A)/<view>"
    Then the response status is 200
    And the response body contains "Alpha task"
    And the response body contains "Beta task"
    And the response body does not contain "Gamma task"
    And the response body does not contain "Loose task"

    Examples:
      | view  |
      | board |
      | tasks |

  Scenario: the epics page is scoped to the workspace
    Given I run tk "--json create 'Epic of A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Child of A' --parent @id(epic)" in "A"
    When I GET resolved "/p/@project(A)/w/@workspace(A)/epics"
    Then the response status is 200
    And the response body contains "Epic of A"
    When I GET resolved "/p/@project(A)/w/@workspace(B)/epics"
    Then the response status is 200
    And the response body does not contain "Epic of A"
    When I GET resolved "/p/@project(C)/epics"
    Then the response status is 200
    And the response body does not contain "Epic of A"

  Scenario: the epic badge in a scoped task list keeps the scope prefix
    Given I run tk "--json create 'Epic of A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Child of A' --parent @id(epic)" in "A"
    When I GET resolved "/p/@project(A)/w/@workspace(A)/tasks"
    Then the response status is 200
    And the response body contains resolved 'href="/p/@project(A)/w/@workspace(A)/epics/@id(epic)"'

  # ---------------------------------------------------------------------------
  # 404s
  # ---------------------------------------------------------------------------

  Scenario: an unknown project id returns 404
    When I GET resolved "/p/999999/board"
    Then the response status is 404

  Scenario: an unknown workspace id returns 404
    When I GET resolved "/p/@project(A)/w/999999/tasks"
    Then the response status is 404

  Scenario: non-numeric ids return 404
    When I GET resolved "/p/abc/board"
    Then the response status is 404
    When I GET resolved "/p/@project(A)/w/abc/board"
    Then the response status is 404

  Scenario: a workspace from another project returns 404
    When I GET resolved "/p/@project(C)/w/@workspace(A)/tasks"
    Then the response status is 404

  # ---------------------------------------------------------------------------
  # Sidebar and links
  # ---------------------------------------------------------------------------

  Scenario: the sidebar lists projects and workspaces
    When I GET resolved "/board"
    Then the response status is 200
    And the sidebar lists project "A"
    And the sidebar lists project "C"
    And the sidebar lists workspace "A"
    And the sidebar lists workspace "B"
    And the sidebar lists workspace "C"

  Scenario: the sidebar marks a missing workspace
    Given I remove the directory "B"
    When I GET resolved "/tasks"
    Then the sidebar marks workspace "B" as missing
    And the sidebar does not mark workspace "A" as missing

  Scenario: tasks of a missing workspace still show up in the project scope
    Given I remove the directory "B"
    When I GET resolved "/p/@project(A)/tasks"
    Then the response status is 200
    And the response body contains "Beta task"

  Scenario: the scoped poll URL keeps the scope prefix
    When I GET resolved "/p/@project(A)/w/@workspace(A)/tasks"
    Then the response body contains resolved 'hx-get="/p/@project(A)/w/@workspace(A)/tasks'
    When I GET resolved "/p/@project(A)/epics"
    Then the response body contains resolved 'hx-get="/p/@project(A)/epics'

  Scenario: HTMX partial responses have no sidebar
    When I GET resolved "/p/@project(A)/w/@workspace(A)/tasks"
    Then the response body contains "scope-sidebar"
    When I HTMX GET resolved "/p/@project(A)/w/@workspace(A)/tasks"
    Then the response status is 200
    And the response body does not contain "scope-sidebar"
    And the response body contains "Alpha task"
    And the response body does not contain "Beta task"

  # ---------------------------------------------------------------------------
  # JSON API filters
  # ---------------------------------------------------------------------------

  Scenario: GET /api/tasks without a scope returns every task
    When I GET resolved "/api/tasks"
    Then the response status is 200
    And the response JSON array lists exactly the titles "Alpha task,Beta task,Gamma task,Loose task"

  Scenario: GET /api/tasks?workspace= filters to one workspace
    When I GET resolved "/api/tasks?workspace=@workspace(A)"
    Then the response status is 200
    And the response JSON array lists exactly the titles "Alpha task"

  Scenario: GET /api/tasks?project= filters to one project
    When I GET resolved "/api/tasks?project=@project(A)"
    Then the response JSON array lists exactly the titles "Alpha task,Beta task"

  Scenario: the workspace parameter wins over the project parameter
    When I GET resolved "/api/tasks?project=@project(C)&workspace=@workspace(A)"
    Then the response JSON array lists exactly the titles "Alpha task"

  Scenario: GET /api/tasks/ready honors the workspace filter
    When I GET resolved "/api/tasks/ready?workspace=@workspace(B)"
    Then the response JSON array lists exactly the titles "Beta task"

  Scenario: GET /api/stats honors the project filter
    When I GET resolved "/api/stats?project=@project(A)"
    Then the response status is 200
    And the response JSON path "by_status.open" equals "2"

  Scenario: a non-integer workspace parameter returns 400
    When I GET resolved "/api/tasks?workspace=abc"
    Then the response status is 400

  Scenario: a non-integer project parameter returns 400
    When I GET resolved "/api/tasks?project=abc"
    Then the response status is 400

  Scenario: an empty workspace parameter returns 400
    When I GET resolved "/api/tasks?workspace="
    Then the response status is 400

  Scenario: an empty project parameter returns 400
    When I GET resolved "/api/tasks?project="
    Then the response status is 400

  Scenario: an unknown workspace parameter returns 404
    When I GET resolved "/api/tasks?workspace=999999"
    Then the response status is 404

  Scenario: an unknown project parameter returns 404
    When I GET resolved "/api/tasks?project=999999"
    Then the response status is 404

  Scenario: GET /api/workspaces lists workspaces with project and missing flag
    Given I remove the directory "B"
    When I GET resolved "/api/workspaces"
    Then the response status is 200
    And the workspaces response row for "A" has "workspace_id" equal to "@workspace(A)"
    And the workspaces response row for "A" has "project_id" equal to "@project(A)"
    And the workspaces response row for "A" has "project_name" equal to "repo"
    And the workspaces response row for "A" has "name" equal to "A"
    And the workspaces response row for "A" has "missing" equal to "false"
    And the workspaces response row for "A" has "current" equal to "false"
    And the workspaces response row for "A" has "open" equal to "1"
    And the workspaces response row for "B" has "missing" equal to "true"
    And the workspaces response row for "C" has "project_name" equal to "other"

  # ---------------------------------------------------------------------------
  # JSON API writes
  # ---------------------------------------------------------------------------

  Scenario: PATCH with workspace_id moves a task and its children
    Given I run tk "--json create 'Child of alpha' --parent @id(alpha)" in "A" and save the id as "child"
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":@workspace(B)}'
    Then the response status is 200
    And the task "alpha" has workspace_id "@workspace(B)"
    And the task "child" has workspace_id "@workspace(B)"

  Scenario: PATCH with workspace_id null unscopes the task
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":null}'
    Then the response status is 200
    And the task "alpha" has workspace_id "null"

  Scenario: PATCH without workspace_id leaves the workspace unchanged
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"priority":1}'
    Then the response status is 200
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: PATCH with an unknown workspace_id returns 404
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":999999}'
    Then the response status is 404
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: PATCH with parent_id moves the task and its children to the parent's workspace
    Given I run tk "--json create 'Child of beta' --parent @id(beta)" in "B" and save the id as "child"
    When I PATCH resolved "/api/tasks/@id(beta)" with body '{"parent_id":"@id(alpha)"}'
    Then the response status is 200
    And the task "beta" has workspace_id "@workspace(A)"
    And the task "child" has workspace_id "@workspace(A)"

  Scenario: PATCH with a failing field leaves the workspace unchanged
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":@workspace(B),"status":"bogus"}'
    Then the response status is 400
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: POST /api/tasks with workspace_id creates the task in that workspace
    When I POST resolved "/api/tasks" with body '{"title":"Created via API","workspace_id":@workspace(B)}'
    Then the response status is 201
    And the response JSON path "workspace_id" equals "@workspace(B)"
    When I GET resolved "/api/tasks?workspace=@workspace(B)"
    Then the response JSON array lists exactly the titles "Beta task,Created via API"

  Scenario: POST /api/tasks without workspace_id creates an unscoped task
    When I POST resolved "/api/tasks" with body '{"title":"Unscoped via API"}'
    Then the response status is 201
    And the response JSON path "workspace_id" equals "null"

  Scenario: POST /api/tasks with an unknown workspace_id returns 404
    When I POST resolved "/api/tasks" with body '{"title":"Nowhere","workspace_id":999999}'
    Then the response status is 404
