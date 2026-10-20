Feature: Project backlog in the web UI and API
  A project backlog holds the tasks of a project that belong to none of its workspaces
  (workspace_id is null, project_id is set). The web UI shows it as a "Backlog" scope
  under /p/{project}/backlog, the JSON API selects it with project=<id>&backlog=1, and
  tasks move in and out of it with PATCH /api/tasks/{id} or the "move open tasks to
  backlog" action of a missing workspace. See project_backlog.feature for the CLI side.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a git repository "other"
    And a worktree "C" of repository "other"
    And I run tk "--json create 'Alpha task' -t wstag" in "A" and save the id as "alpha"
    And I run tk "--json create 'Parked task' -t bltag --to-backlog" in "A" and save the id as "parked"
    And I run tk "--json create 'Gamma task'" in "C" and save the id as "gamma"
    And the web server is running

  # ---------------------------------------------------------------------------
  # Backlog scope pages
  # ---------------------------------------------------------------------------

  Scenario Outline: the backlog <view> page lists only the backlog of that project
    Given I run tk "--json create 'Parked in other' --to-backlog" in "C"
    When I GET resolved "/p/@project(A)/backlog/<view>"
    Then the response status is 200
    And the response body contains "Parked task"
    And the response body does not contain "Alpha task"
    And the response body does not contain "Gamma task"
    And the response body does not contain "Parked in other"

    Examples:
      | view  |
      | board |
      | tasks |

  Scenario: the backlog of another project lists its own tasks
    Given I run tk "--json create 'Parked in other' --to-backlog" in "C"
    When I GET resolved "/p/@project(C)/backlog/tasks"
    Then the response status is 200
    And the response body contains "Parked in other"
    And the response body does not contain "Parked task"

  Scenario: the backlog epics page lists backlog epics and the epic detail renders
    Given I run tk "--json create 'Parked epic' --to-backlog" in "A" and save the id as "epic"
    And I run tk "--json --scope backlog create 'Parked child' --parent @id(epic)" in "A"
    And I run tk "--json create 'Alpha epic'" in "A" and save the id as "alpha_epic"
    And I run tk "--json create 'Alpha child' --parent @id(alpha_epic)" in "A"
    When I GET resolved "/p/@project(A)/backlog/epics"
    Then the response status is 200
    And the response body contains "Parked epic"
    And the response body does not contain "Alpha epic"
    And the response body contains resolved 'href="/p/@project(A)/backlog/epics/@id(epic)"'
    When I GET resolved "/p/@project(A)/backlog/epics/@id(epic)"
    Then the response status is 200
    And the response body contains "Parked epic"

  Scenario Outline: the backlog <page> page renders
    When I GET resolved "/p/@project(A)/backlog/<page>"
    Then the response status is 200

    Examples:
      | page            |
      | epics           |
      | tasks/new       |
      | tasks/new/modal |

  Scenario Outline: the backlog <page> page of an unknown project is 404
    When I GET resolved "/p/999999/backlog/<page>"
    Then the response status is 404

    Examples:
      | page            |
      | board           |
      | tasks           |
      | epics           |
      | epics/tk-none   |
      | tasks/new       |
      | tasks/new/modal |

  Scenario: a backlog page with a non-numeric project id is 404
    When I GET resolved "/p/abc/backlog/board"
    Then the response status is 404

  Scenario: links and polling in the backlog scope keep the backlog prefix
    When I GET resolved "/p/@project(A)/backlog/tasks"
    Then the response status is 200
    And the response body contains resolved 'href="/p/@project(A)/backlog/tasks"'
    And the response body contains resolved 'href="/p/@project(A)/backlog/board"'
    And the response body contains resolved 'href="/p/@project(A)/backlog/epics"'
    And the response body contains resolved 'hx-get="/p/@project(A)/backlog/tasks/new/modal"'
    And the response body contains resolved 'hx-get="/p/@project(A)/backlog/tasks'

  Scenario: an HTMX request to a backlog page returns a fragment without the sidebar
    When I GET resolved "/p/@project(A)/backlog/tasks"
    Then the response body contains "scope-sidebar"
    When I HTMX GET resolved "/p/@project(A)/backlog/tasks"
    Then the response status is 200
    And the response body does not contain "scope-sidebar"
    And the response body contains "Parked task"
    And the response body does not contain "Alpha task"

  Scenario: the backlog page shows the backlog in the scope header
    When I GET resolved "/p/@project(A)/backlog/board"
    Then the response status is 200
    And the response body contains resolved '<strong class="scope-crumb-ws">Backlog</strong>'

  # ---------------------------------------------------------------------------
  # Creating tasks from the HTML forms
  # ---------------------------------------------------------------------------

  Scenario: posting the backlog form creates a backlog task and redirects to the backlog list
    When I POST the form "title=From+the+backlog+form" to resolved "/p/@project(A)/backlog/tasks" without following redirects it answers 303 with location "/p/@project(A)/backlog/tasks"
    And I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked task,From the backlog form"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Alpha task"

  Scenario: posting the backlog form of an unknown project is 404 and creates nothing
    When I POST the form "title=Nowhere" to resolved "/p/999999/backlog/tasks"
    Then the response status is 404
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked task"

  Scenario: a form that picks the backlog option creates a backlog task
    When I POST the form "title=Picked+backlog&workspace_id=backlog:@project(A)" to resolved "/p/@project(A)/tasks" without following redirects it answers 303 with location "/p/@project(A)/tasks"
    And I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked task,Picked backlog"

  # ---------------------------------------------------------------------------
  # Sidebar
  # ---------------------------------------------------------------------------

  Scenario: every project has a Backlog entry with its not-done count, even when empty
    When I GET resolved "/board"
    Then the response status is 200
    And the sidebar link "/p/@project(A)/backlog" has count 1
    And the sidebar link "/p/@project(C)/backlog" has count 0

  Scenario: the Backlog entry comes first in the list of its project
    When I GET resolved "/board"
    Then the sidebar link "/p/@project(A)/backlog" comes before the sidebar link "/p/@project(A)/w/@workspace(A)"
    And the sidebar link "/p/@project(C)/backlog" comes before the sidebar link "/p/@project(C)/w/@workspace(C)"

  Scenario: the Backlog entry counts in progress and blocked tasks but not done ones
    Given I run tk "--json create 'Parked wip' --to-backlog" in "A" and save the id as "wip"
    And I run tk "--json create 'Parked blocked' --to-backlog" in "A" and save the id as "blocked"
    And I run tk "--json create 'Parked done' --to-backlog" in "A" and save the id as "done"
    And I run tk "--scope backlog update @id(wip) -s in_progress" in "A"
    And I run tk "--scope backlog update @id(blocked) -s blocked" in "A"
    And I run tk "--scope backlog close @id(done)" in "A"
    When I GET resolved "/board"
    Then the sidebar link "/p/@project(A)/backlog" has count 3

  Scenario: the Backlog entry is highlighted only in the backlog scope
    When I GET resolved "/p/@project(A)/backlog/board"
    Then the response status is 200
    And the sidebar link "/p/@project(A)/backlog" is active
    And the sidebar link "/p/@project(A)" is not active
    And the sidebar link "/p/@project(A)/w/@workspace(A)" is not active
    And the sidebar link "" is not active
    When I GET resolved "/p/@project(A)/board"
    Then the sidebar link "/p/@project(A)/backlog" is not active
    And the sidebar link "/p/@project(A)" is active

  Scenario: the Backlog entry of another project is not highlighted
    When I GET resolved "/p/@project(A)/backlog/board"
    Then the sidebar link "/p/@project(C)/backlog" is not active

  Scenario: a project with only backlog tasks appears in the sidebar
    Given a git repository "fresh"
    And a worktree "D" of repository "fresh"
    And I run tk "--json create 'Only parked' --to-backlog" in "D" and save the id as "only"
    When I GET resolved "/board"
    Then the sidebar has the link "/p/@taskproject(only)"
    And the sidebar link "/p/@taskproject(only)" has count 1
    And the sidebar link "/p/@taskproject(only)/backlog" has count 1
    When I GET resolved "/p/@taskproject(only)/backlog/tasks"
    Then the response status is 200
    And the response body contains "Only parked"

  Scenario: the project and All counts include the backlog
    When I GET resolved "/board"
    Then the sidebar link "/p/@project(A)" has count 2
    And the sidebar link "/p/@project(A)/w/@workspace(A)" has count 1
    And the sidebar link "/p/@project(C)" has count 1
    And the sidebar link "" has count 3

  Scenario: closing a backlog task lowers the project and All counts
    Given I run tk "--scope backlog close @id(parked)" in "A"
    When I GET resolved "/board"
    Then the sidebar link "/p/@project(A)" has count 1
    And the sidebar link "/p/@project(A)/backlog" has count 0
    And the sidebar link "" has count 2

  # ---------------------------------------------------------------------------
  # Backlog badge
  # ---------------------------------------------------------------------------

  Scenario Outline: backlog tasks carry a backlog badge in <view>
    When I GET resolved "<view>"
    Then the response status is 200
    And the response body contains "Parked task"
    And the response body contains 'title="Backlog of project repo"'

    Examples:
      | view                 |
      | /tasks               |
      | /board               |
      | /p/@project(A)/tasks |
      | /p/@project(A)/board |

  Scenario: a backlog epic carries a backlog badge in the epics list of the project scope
    Given I run tk "--json create 'Parked epic' --to-backlog" in "A" and save the id as "epic"
    And I run tk "--json --scope backlog create 'Parked child' --parent @id(epic)" in "A"
    When I GET resolved "/p/@project(A)/epics"
    Then the response status is 200
    And the response body contains "Parked epic"
    And the response body contains 'title="Backlog of project repo"'

  Scenario Outline: the badge is not shown in the <scope> scope
    When I GET resolved "<view>"
    Then the response status is 200
    And the response body does not contain 'title="Backlog of project repo"'

    Examples:
      | scope           | view                                    |
      | backlog board   | /p/@project(A)/backlog/board            |
      | backlog tasks   | /p/@project(A)/backlog/tasks            |
      | workspace board | /p/@project(A)/w/@workspace(A)/board    |
      | workspace tasks | /p/@project(A)/w/@workspace(A)/tasks    |

  Scenario: workspace tasks do not get a backlog badge
    When I GET resolved "/p/@project(C)/tasks"
    Then the response status is 200
    And the response body contains "Gamma task"
    And the response body does not contain 'title="Backlog of project other"'

  # ---------------------------------------------------------------------------
  # Task detail and the move selector
  # ---------------------------------------------------------------------------

  Scenario: task detail names the project of a backlog task
    When I GET resolved "/tasks/@id(parked)"
    Then the response status is 200
    And the response body contains "none (backlog of project repo)"

  Scenario: task detail of a workspace task does not mention the backlog placement
    When I GET resolved "/tasks/@id(alpha)"
    Then the response status is 200
    And the response body does not contain "none (backlog of project"

  Scenario: the move selector of a backlog task has a Backlog option per project with its own selected
    When I GET resolved "/tasks/@id(parked)"
    Then the response body contains resolved '<option value="backlog:@project(A)" selected>Backlog</option>'
    And the response body contains resolved '<option value="backlog:@project(C)">Backlog</option>'
    And the response body contains '<option value="">'
    And the response body does not contain '<option value="" selected>'

  Scenario: the move selector of a workspace task offers Backlog but keeps the workspace selected
    When I GET resolved "/tasks/@id(alpha)"
    Then the response body contains resolved '<option value="backlog:@project(A)">Backlog</option>'
    And the response body contains resolved '<option value="@workspace(A)" selected>'

  Scenario: the move selector of an unscoped task keeps the unscoped option selected
    Given a directory "nogit" outside any git repository
    And I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I GET resolved "/tasks/@id(loose)"
    Then the response body contains '<option value="" selected>'
    And the response body contains resolved '<option value="backlog:@project(A)">Backlog</option>'

  Scenario: the HTMX task detail fragment also shows the backlog placement
    When I HTMX GET the task "parked"
    Then the response status is 200
    And the response body contains "none (backlog of project repo)"
    And the response body contains resolved '<option value="backlog:@project(A)" selected>Backlog</option>'

  # ---------------------------------------------------------------------------
  # Create forms
  # ---------------------------------------------------------------------------

  Scenario: the new task form in project scope defaults to the project backlog
    When I GET resolved "/p/@project(A)/tasks/new"
    Then the response status is 200
    And the response body contains resolved '<option value="backlog:@project(A)" selected>Backlog</option>'
    And the response body does not contain '<option value="" selected>'

  Scenario: the create modal in project scope defaults to the project backlog
    When I GET resolved "/p/@project(A)/tasks/new/modal"
    Then the response status is 200
    And the response body contains resolved '<option value="backlog:@project(A)" selected>Backlog</option>'
    And the response body does not contain resolved '<option value="backlog:@project(C)" selected>'

  Scenario: the create modal in backlog scope has a hidden backlog field and no selector
    When I GET resolved "/p/@project(A)/backlog/tasks/new/modal"
    Then the response status is 200
    And the response body contains resolved '<input type="hidden" name="workspace_id" value="backlog:@project(A)">'
    And the response body does not contain 'id="create-workspace"'

  Scenario: the new task form in backlog scope defaults to the project backlog
    When I GET resolved "/p/@project(A)/backlog/tasks/new"
    Then the response status is 200
    And the response body contains resolved '<option value="backlog:@project(A)" selected>Backlog</option>'

  Scenario: the create modal in All scope offers Backlog options without selecting one
    When I GET "/tasks/new/modal"
    Then the response status is 200
    And the response body contains resolved '<option value="backlog:@project(A)">Backlog</option>'
    And the response body contains resolved '<option value="backlog:@project(C)">Backlog</option>'

  # ---------------------------------------------------------------------------
  # Missing workspace: move open tasks to the backlog
  # ---------------------------------------------------------------------------

  Scenario: task detail of a task in a missing workspace offers to move open tasks to the backlog
    Given I run tk "--json create 'Beta one'" in "B" and save the id as "b1"
    And I remove the directory "B"
    When I GET resolved "/tasks/@id(b1)"
    Then the response status is 200
    And the response body contains "Move open tasks to backlog"
    And the response body contains resolved 'data-ws-action="move-open-to-backlog" data-ws-id="@workspace(B)"'

  Scenario: task detail of a task in an existing workspace has no move-to-backlog button
    When I GET resolved "/tasks/@id(alpha)"
    Then the response status is 200
    And the response body does not contain "Move open tasks to backlog"

  Scenario: moving the open tasks of a workspace to the backlog keeps done tasks in place
    Given I run tk "--json create 'Beta open'" in "B" and save the id as "bopen"
    And I run tk "--json create 'Beta wip'" in "B" and save the id as "bwip"
    And I run tk "--json create 'Beta done'" in "B" and save the id as "bdone"
    And I run tk "update @id(bwip) --claim" in "B"
    And I run tk "close @id(bdone)" in "B"
    And I remove the directory "B"
    When I POST resolved "/api/workspaces/@workspace(B)/move-open-to-backlog" with body "{}"
    Then the response status is 200
    And the response JSON path "moved" equals "2"
    And the response JSON path "project_id" equals "@project(A)"
    And the task "bopen" has workspace_id "null"
    And the task "bwip" has workspace_id "null"
    And the task "bdone" has workspace_id "@workspace(B)"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked task,Beta open,Beta wip"
    When I run tk "--json show @id(bwip)" in "A"
    Then the JSON output path "status" equals "in_progress"
    And the JSON output path "project_id" equals "@project(A)"

  Scenario: moving the open tasks also moves their subtasks, done ones included
    Given I run tk "--json create 'Beta parent'" in "B" and save the id as "bparent"
    And I run tk "--json create 'Beta open child' --parent @id(bparent)" in "B" and save the id as "bkid"
    And I run tk "--json create 'Beta done child' --parent @id(bparent)" in "B" and save the id as "bdonekid"
    And I run tk "close @id(bdonekid)" in "B"
    When I POST resolved "/api/workspaces/@workspace(B)/move-open-to-backlog" with body "{}"
    Then the response status is 200
    And the response JSON path "moved" equals "3"
    And the task "bparent" has workspace_id "null"
    And the task "bkid" has workspace_id "null"
    And the task "bdonekid" has workspace_id "null"

  Scenario: moving the open tasks leaves other workspaces alone
    Given I run tk "--json create 'Beta open'" in "B" and save the id as "bopen"
    When I POST resolved "/api/workspaces/@workspace(B)/move-open-to-backlog" with body "{}"
    Then the task "alpha" has workspace_id "@workspace(A)"
    And the task "gamma" has workspace_id "@workspace(C)"

  Scenario: repeating the move reports zero tasks moved
    Given I run tk "--json create 'Beta open'" in "B"
    When I POST resolved "/api/workspaces/@workspace(B)/move-open-to-backlog" with body "{}"
    And I POST resolved "/api/workspaces/@workspace(B)/move-open-to-backlog" with body "{}"
    Then the response status is 200
    And the response JSON path "moved" equals "0"
    And the response JSON path "project_id" equals "@project(A)"

  Scenario: moving from a workspace with only done tasks moves nothing
    Given I run tk "--json create 'Beta done'" in "B" and save the id as "bdone"
    And I run tk "close @id(bdone)" in "B"
    When I POST resolved "/api/workspaces/@workspace(B)/move-open-to-backlog" with body "{}"
    Then the response status is 200
    And the response JSON path "moved" equals "0"
    And the task "bdone" has workspace_id "@workspace(B)"

  Scenario: moving open tasks to the backlog of an unknown workspace is 404
    When I POST resolved "/api/workspaces/999999/move-open-to-backlog" with body "{}"
    Then the response status is 404

  Scenario: moving open tasks to the backlog of a non-numeric workspace id is 404
    When I POST resolved "/api/workspaces/abc/move-open-to-backlog" with body "{}"
    Then the response status is 404

  Scenario: the move-open-to-backlog endpoint rejects GET
    When I GET resolved "/api/workspaces/@workspace(A)/move-open-to-backlog"
    Then the response status is 405

  Scenario: a cross-origin move-open-to-backlog is rejected and moves nothing
    When I POST resolved "/api/workspaces/@workspace(A)/move-open-to-backlog" with body "{}" from a foreign origin
    Then the response status is 403
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: a cross-site move-open-to-backlog is rejected and moves nothing
    When I POST "/api/workspaces/@workspace(A)/move-open-to-backlog" with body "{}" and header "Sec-Fetch-Site" set to "cross-site"
    Then the response status is 403
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: a move-open-to-backlog with a foreign Host header is rejected and moves nothing
    When I POST "/api/workspaces/@workspace(A)/move-open-to-backlog" with body "{}" and header "Host" set to "evil.example"
    Then the response status is 403
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: a same-origin move-open-to-backlog is accepted
    When I POST "/api/workspaces/@workspace(A)/move-open-to-backlog" with body "{}" and header "Origin" set to "http://127.0.0.1:@port"
    Then the response status is 200
    And the response JSON path "moved" equals "1"

  # ---------------------------------------------------------------------------
  # GET /api/backlogs and the workspaces overview
  # ---------------------------------------------------------------------------

  Scenario: GET /api/backlogs has one row per project with a backlog task
    When I GET resolved "/api/backlogs"
    Then the response status is 200
    And the response JSON array has length 1
    And the backlogs response row for project "@project(A)" has "project_name" equal to "repo"
    And the backlogs response row for project "@project(A)" has "open" equal to "1"
    And the backlogs response rows have exactly the keys "project_id,project_name,open,in_progress,blocked,done"

  Scenario: GET /api/backlogs counts each status
    Given I run tk "--json create 'Parked wip' --to-backlog" in "A" and save the id as "wip"
    And I run tk "--json create 'Parked blocked' --to-backlog" in "A" and save the id as "blocked"
    And I run tk "--json create 'Parked done' --to-backlog" in "A" and save the id as "done"
    And I run tk "--scope backlog update @id(wip) -s in_progress" in "A"
    And I run tk "--scope backlog update @id(blocked) -s blocked" in "A"
    And I run tk "--scope backlog close @id(done)" in "A"
    When I GET resolved "/api/backlogs"
    Then the backlogs response row for project "@project(A)" has "open" equal to "1"
    And the backlogs response row for project "@project(A)" has "in_progress" equal to "1"
    And the backlogs response row for project "@project(A)" has "blocked" equal to "1"
    And the backlogs response row for project "@project(A)" has "done" equal to "1"

  Scenario: GET /api/backlogs lists every project that has backlog tasks
    Given I run tk "--json create 'Parked in other' --to-backlog" in "C"
    When I GET resolved "/api/backlogs"
    Then the response JSON array has length 2
    And the backlogs response row for project "@project(A)" has "project_name" equal to "repo"
    And the backlogs response row for project "@project(C)" has "project_name" equal to "other"
    And the backlogs response row for project "@project(C)" has "open" equal to "1"

  Scenario: GET /api/backlogs keeps a project whose backlog only has done tasks
    Given I run tk "--scope backlog close @id(parked)" in "A"
    When I GET resolved "/api/backlogs"
    Then the response JSON array has length 1
    And the backlogs response row for project "@project(A)" has "open" equal to "0"
    And the backlogs response row for project "@project(A)" has "done" equal to "1"

  Scenario: GET /api/backlogs is empty when no project has backlog tasks
    Given I run tk "--scope backlog update @id(parked) --move-to none" in "A"
    When I GET resolved "/api/backlogs"
    Then the response status is 200
    And the response JSON is an empty array

  Scenario: GET /api/workspaces does not list the backlog
    When I GET resolved "/api/workspaces"
    Then the response status is 200
    And the response JSON array has length 2
    And the workspaces response row for "A" has "open" equal to "1"
    And the workspaces response row for "A" has "workspace_id" equal to "@workspace(A)"

  Scenario: tk workspaces --json does not list the backlog
    When I run tk "--json workspaces" in "A"
    Then the tk command succeeds
    And the workspaces output has 2 workspaces
    And the workspaces JSON row for "A" has "open" equal to "1"

  Scenario: tk workspaces prints a backlog line before the workspaces of its project
    When I run tk "workspaces" in "A"
    Then the tk command succeeds
    And the tk output has 1 lines containing "(backlog)"
    And the tk output line containing "(backlog)" comes before the line containing "@path(A)"

  Scenario: tk workspaces prints a backlog line for a project without workspaces
    Given a git repository "fresh"
    And a worktree "D" of repository "fresh"
    And I run tk "--json create 'Only parked' --to-backlog" in "D"
    When I run tk "workspaces" in "A"
    Then the tk command succeeds
    And the tk output has 2 lines containing "(backlog)"
    And the tk output has 1 lines containing "fresh"

  Scenario: tk workspaces prints no backlog line when no backlog has tasks
    Given I run tk "--scope backlog update @id(parked) --move-to none" in "A"
    When I run tk "workspaces" in "A"
    Then the tk command succeeds
    And the tk output has 0 lines containing "(backlog)"

  # ---------------------------------------------------------------------------
  # POST /api/tasks with project_id
  # ---------------------------------------------------------------------------

  Scenario: POST /api/tasks with project_id and no workspace_id creates a backlog task
    When I POST resolved "/api/tasks" with body '{"title":"Parked via API","project_id":@project(A)}'
    Then the response status is 201
    And the response JSON path "workspace_id" equals "null"
    And the response JSON path "project_id" equals "@project(A)"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked task,Parked via API"

  Scenario: a backlog task created through the API stays out of the workspace list
    When I POST resolved "/api/tasks" with body '{"title":"Parked via API","project_id":@project(A)}'
    And I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Alpha task"

  Scenario: POST /api/tasks with project_id creates the task in the backlog of another project
    When I POST resolved "/api/tasks" with body '{"title":"Parked for other","project_id":@project(C)}'
    Then the response status is 201
    And the response JSON path "project_id" equals "@project(C)"
    When I run tk "--json --scope backlog list" in "C"
    Then the JSON output lists exactly the titles "Parked for other"

  Scenario: POST /api/tasks with project_id of an unknown project is 404
    When I POST resolved "/api/tasks" with body '{"title":"Nowhere","project_id":999999}'
    Then the response status is 404
    When I GET resolved "/api/tasks"
    Then the response body does not contain "Nowhere"

  Scenario: POST /api/tasks with a workspace of another project is 400
    When I POST resolved "/api/tasks" with body '{"title":"Mismatch","workspace_id":@workspace(C),"project_id":@project(A)}'
    Then the response status is 400
    When I GET resolved "/api/tasks"
    Then the response body does not contain "Mismatch"

  Scenario: POST /api/tasks with a workspace and its own project creates a workspace task
    When I POST resolved "/api/tasks" with body '{"title":"Matching","workspace_id":@workspace(A),"project_id":@project(A)}'
    Then the response status is 201
    And the response JSON path "workspace_id" equals "@workspace(A)"
    And the response JSON path "project_id" equals "@project(A)"

  Scenario: POST /api/tasks with a workspace_id still reports the project of the workspace
    When I POST resolved "/api/tasks" with body '{"title":"In A","workspace_id":@workspace(A)}'
    Then the response status is 201
    And the response JSON path "project_id" equals "@project(A)"

  Scenario: POST /api/tasks without project_id or workspace_id creates an unscoped task
    When I POST resolved "/api/tasks" with body '{"title":"Loose via API"}'
    Then the response status is 201
    And the response JSON path "workspace_id" equals "null"
    And the response JSON path "project_id" equals "null"

  Scenario: a subtask created through the API follows a backlog parent
    When I POST resolved "/api/tasks" with body '{"title":"Parked child","parent_id":"@id(parked)"}'
    Then the response status is 201
    And the response JSON path "workspace_id" equals "null"
    And the response JSON path "project_id" equals "@project(A)"

  # ---------------------------------------------------------------------------
  # PATCH /api/tasks/{id} with project_id
  # ---------------------------------------------------------------------------

  Scenario: PATCH with workspace_id null and project_id moves a task and its subtasks to the backlog
    Given I run tk "--json create 'Alpha child' --parent @id(alpha)" in "A" and save the id as "child"
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":null,"project_id":@project(A)}'
    Then the response status is 200
    And the response JSON path "workspace_id" equals "null"
    And the response JSON path "project_id" equals "@project(A)"
    And the task "alpha" has workspace_id "null"
    And the task "child" has workspace_id "null"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked task,Alpha task,Alpha child"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: PATCH with workspace_id null and the project of another repository moves the task to that backlog
    When I PATCH resolved "/api/tasks/@id(parked)" with body '{"workspace_id":null,"project_id":@project(C)}'
    Then the response status is 200
    And the response JSON path "project_id" equals "@project(C)"
    When I run tk "--json --scope backlog list" in "C"
    Then the JSON output lists exactly the titles "Parked task"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: PATCH can move a backlog task into a workspace of its project
    When I PATCH resolved "/api/tasks/@id(parked)" with body '{"workspace_id":@workspace(A),"project_id":@project(A)}'
    Then the response status is 200
    And the task "parked" has workspace_id "@workspace(A)"

  Scenario: PATCH can move a backlog task into a workspace without project_id
    When I PATCH resolved "/api/tasks/@id(parked)" with body '{"workspace_id":@workspace(A)}'
    Then the response status is 200
    And the task "parked" has workspace_id "@workspace(A)"

  Scenario: PATCH with an unknown project_id is 404 and changes nothing
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":null,"project_id":999999}'
    Then the response status is 404
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: PATCH with a workspace and a project that does not match is 400 and changes nothing
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":@workspace(C),"project_id":@project(A)}'
    Then the response status is 400
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: PATCH with project_id and no workspace_id is 400 and changes nothing
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"project_id":@project(A)}'
    Then the response status is 400
    And the task "alpha" has workspace_id "@workspace(A)"

  Scenario: PATCH with project_id and no workspace_id does not apply other fields
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"title":"Renamed","project_id":@project(A)}'
    Then the response status is 400
    When I run tk "--json show @id(alpha)" in "A"
    Then the JSON output path "title" equals "Alpha task"

  Scenario: PATCH with a non-integer project_id is rejected
    When I PATCH resolved "/api/tasks/@id(alpha)" with body '{"workspace_id":null,"project_id":"x"}'
    Then the response status is an error
    And the task "alpha" has workspace_id "@workspace(A)"

  # ---------------------------------------------------------------------------
  # List endpoints with backlog=1
  # ---------------------------------------------------------------------------

  Scenario Outline: GET <path> with backlog=1 returns only the backlog of the project
    Given I run tk "--json create 'Parked waiting' --to-backlog" in "A" and save the id as "pwait"
    And I run tk "--scope backlog dep add @id(pwait) @id(parked)" in "A"
    And I run tk "--json create 'Alpha waiting'" in "A" and save the id as "await"
    And I run tk "dep add @id(await) @id(alpha)" in "A"
    And I run tk "--json create 'Parked epic' --to-backlog" in "A" and save the id as "pepic"
    And I run tk "--json --scope backlog create 'Parked epic child' --parent @id(pepic)" in "A"
    And I run tk "--json create 'Alpha epic'" in "A" and save the id as "aepic"
    And I run tk "--json create 'Alpha epic child' --parent @id(aepic)" in "A"
    When I GET resolved "<path>?project=@project(A)&backlog=1"
    Then the response status is 200
    And the response body contains "<included>"
    And the response body does not contain "<excluded>"

    Examples:
      | path               | included       | excluded      |
      | /api/tasks         | Parked task    | Alpha task    |
      | /api/tasks/ready   | Parked task    | Alpha task    |
      | /api/tasks/blocked | Parked waiting | Alpha waiting |
      | /api/epics         | Parked epic    | Alpha epic    |
      | /api/stats         | bltag          | wstag         |
      | /api/tags          | bltag          | wstag         |
      | /api/prime         | Parked task    | Alpha task    |

  Scenario Outline: GET /api/tasks with backlog=<value> selects the backlog
    When I GET resolved "/api/tasks?project=@project(A)&backlog=<value>"
    Then the response status is 200
    And the response JSON array lists exactly the titles "Parked task"

    Examples:
      | value |
      | 1     |
      | true  |

  Scenario Outline: GET /api/tasks with backlog=<value> keeps the whole project
    When I GET resolved "/api/tasks?project=@project(A)&backlog=<value>"
    Then the response status is 200
    And the response JSON array lists exactly the titles "Alpha task,Parked task"

    Examples:
      | value |
      | 0     |
      | false |

  Scenario: GET /api/tasks with project and no backlog flag includes the backlog
    When I GET resolved "/api/tasks?project=@project(A)"
    Then the response JSON array lists exactly the titles "Alpha task,Parked task"

  Scenario: GET /api/tasks?backlog=1 of another project does not leak tasks
    When I GET resolved "/api/tasks?project=@project(C)&backlog=1"
    Then the response status is 200
    And the response JSON is an empty array

  Scenario: GET /api/stats with backlog=1 counts only backlog tasks
    When I GET resolved "/api/stats?project=@project(A)&backlog=1"
    Then the response status is 200
    And the response JSON path "by_status.open" equals "1"

  Scenario: GET /api/tasks?backlog=1 does not include the unscoped bucket
    Given a directory "nogit" outside any git repository
    And I run tk "--json create 'Loose task'" in "nogit"
    When I GET resolved "/api/tasks?project=@project(A)&backlog=1"
    Then the response JSON array lists exactly the titles "Parked task"

  Scenario Outline: GET <path>?<query> is rejected with <status>
    When I GET resolved "<path>?<query>"
    Then the response status is <status>

    Examples:
      | path               | query                                                  | status |
      | /api/tasks         | backlog=1                                              | 400    |
      | /api/tasks/ready   | backlog=1                                              | 400    |
      | /api/tasks/blocked | backlog=1                                              | 400    |
      | /api/epics         | backlog=1                                              | 400    |
      | /api/stats         | backlog=1                                              | 400    |
      | /api/tags          | backlog=1                                              | 400    |
      | /api/prime         | backlog=1                                              | 400    |
      | /api/tasks         | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/tasks/ready   | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/tasks/blocked | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/epics         | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/stats         | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/tags          | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/prime         | workspace=@workspace(A)&backlog=1                      | 400    |
      | /api/tasks         | project=@project(A)&workspace=@workspace(A)&backlog=1  | 400    |
      | /api/tasks         | project=@project(A)&backlog=yes                        | 400    |
      | /api/tasks/ready   | project=@project(A)&backlog=yes                        | 400    |
      | /api/tasks/blocked | project=@project(A)&backlog=yes                        | 400    |
      | /api/epics         | project=@project(A)&backlog=yes                        | 400    |
      | /api/stats         | project=@project(A)&backlog=yes                        | 400    |
      | /api/tags          | project=@project(A)&backlog=yes                        | 400    |
      | /api/prime         | project=@project(A)&backlog=yes                        | 400    |
      | /api/tasks         | project=@project(A)&backlog=                           | 400    |
      | /api/tasks         | project=@project(A)&backlog=TRUE                       | 400    |
      | /api/tasks         | project=999999&backlog=1                               | 404    |
      | /api/tasks/ready   | project=999999&backlog=1                               | 404    |
      | /api/tasks/blocked | project=999999&backlog=1                               | 404    |
      | /api/epics         | project=999999&backlog=1                               | 404    |
      | /api/stats         | project=999999&backlog=1                               | 404    |
      | /api/tags          | project=999999&backlog=1                               | 404    |
      | /api/prime         | project=999999&backlog=1                               | 404    |

  Scenario: backlog=0 without a project is accepted and lists every task
    When I GET resolved "/api/tasks?backlog=0"
    Then the response status is 200
    And the response JSON array lists exactly the titles "Alpha task,Parked task,Gamma task"
