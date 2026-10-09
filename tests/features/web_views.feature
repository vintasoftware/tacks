Feature: Web view pages
  As a developer using the browser UI
  I want HTML views for tasks, board, epics, and forms
  So that I can manage my backlog without using the CLI

  Background:
    Given a tacks database is initialized
    And the web server is running

  # ---------------------------------------------------------------------------
  # Task list page — GET /tasks
  # ---------------------------------------------------------------------------

  Scenario: Task list page returns HTML
    When I GET "/tasks"
    Then the response status is 200
    And the response content type is "text/html"
    And the response body contains "<html"

  Scenario: Task list page contains a task table
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains "<table"

  Scenario: Task list page shows created task titles
    Given I created a task via API with title "Write documentation" as "doc-task"
    And I created a task via API with title "Fix login bug" as "bug-task"
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains "Write documentation"
    And the response body contains "Fix login bug"

  Scenario: Task list page filters by status=open
    Given I created a task via API with title "Open task" as "open-t"
    And I created a task via API with title "Done task" as "done-t"
    And I closed the API task "done-t"
    When I GET "/tasks?status=open"
    Then the response status is 200
    And the response body contains "Open task"

  Scenario: Task list page filters by priority
    Given I created a task via API with title "High priority task" and priority 1 as "p1-view"
    And I created a task via API with title "Low priority task" and priority 3 as "p3-view"
    When I GET "/tasks?priority=1"
    Then the response status is 200
    And the response body contains "High priority task"

  # ---------------------------------------------------------------------------
  # Task detail page — GET /tasks/:id
  # ---------------------------------------------------------------------------

  Scenario: Task detail page returns HTML with task title
    Given I created a task via API with title "Detail view task" as "detail-task"
    When I GET the HTML task "detail-task"
    Then the response status is 200
    And the response content type is "text/html"
    And the response body contains "Detail view task"

  Scenario: Task detail page shows task status and priority
    Given I created a task via API with title "Status check task" and priority 2 as "status-view"
    When I GET the HTML task "status-view"
    Then the response status is 200
    And the response body contains "open"
    And the response body contains "2"

  Scenario: Task detail page shows task description
    Given I created a task via API with title "Described task" and description "This task has a detailed description" as "desc-task"
    When I GET the HTML task "desc-task"
    Then the response status is 200
    And the response body contains "This task has a detailed description"

  Scenario: Task detail page returns 404 for unknown id
    When I GET "/tasks/tk-0000"
    Then the response status is 404

  # ---------------------------------------------------------------------------
  # Board view — GET /board
  # ---------------------------------------------------------------------------

  Scenario: Board page returns HTML
    When I GET "/board"
    Then the response status is 200
    And the response content type is "text/html"
    And the response body contains "<html"

  Scenario: Board page has four status column headers
    When I GET "/board"
    Then the response status is 200
    And the response body contains "Open"
    And the response body contains "In Progress"
    And the response body contains "Blocked"
    And the response body contains "Done"

  Scenario: Board page shows tasks in their correct columns
    Given I created a task via API with title "Open board task" as "board-open"
    When I GET "/board"
    Then the response status is 200
    And the response body contains "Open board task"

  # ---------------------------------------------------------------------------
  # Epic view — GET /epics
  # ---------------------------------------------------------------------------

  Scenario: Epics page returns HTML
    When I GET "/epics"
    Then the response status is 200
    And the response content type is "text/html"
    And the response body contains "<html"

  Scenario: Epics page shows epic progress table
    When I GET "/epics"
    Then the response status is 200
    And the response body contains "<table"

  Scenario: Epics page lists epics with subtasks
    Given I created a task via API with title "Main epic" as "epic-parent"
    And I created a subtask via API with title "Epic subtask" under "epic-parent" as "epic-child"
    When I GET "/epics"
    Then the response status is 200
    And the response body contains "Main epic"

  # ---------------------------------------------------------------------------
  # Create task form — GET /tasks/new
  # ---------------------------------------------------------------------------

  Scenario: Create task form returns HTML
    When I GET "/tasks/new"
    Then the response status is 200
    And the response content type is "text/html"
    And the response body contains "<html"

  Scenario: Create task form contains a form element
    When I GET "/tasks/new"
    Then the response status is 200
    And the response body contains "<form"

  Scenario: Create task form has title input field
    When I GET "/tasks/new"
    Then the response status is 200
    And the response body contains "title"

  Scenario: Create task form has priority input field
    When I GET "/tasks/new"
    Then the response status is 200
    And the response body contains "priority"

  Scenario: Create task form has description input field
    When I GET "/tasks/new"
    Then the response status is 200
    And the response body contains "description"

  # ---------------------------------------------------------------------------
  # Task detail modal fragment — HTMX GET /tasks/:id
  # ---------------------------------------------------------------------------

  Scenario: HTMX request for task detail returns fragment without full page wrapper
    Given I created a task via API with title "Modal task" as "modal-task"
    When I HTMX GET the task "modal-task"
    Then the response status is 200
    And the response body contains "Modal task"
    And the response body contains "<article"
    And the response body does not contain "<!DOCTYPE"
    And the response body does not contain "<html"

  Scenario: HTMX fragment includes status badge
    Given I created a task via API with title "Badge task" as "badge-task"
    When I HTMX GET the task "badge-task"
    Then the response status is 200
    And the response body contains "badge status-"

  Scenario: HTMX fragment includes close button
    Given I created a task via API with title "Closable task" as "close-task"
    When I HTMX GET the task "close-task"
    Then the response status is 200
    And the response body contains "aria-label="

  Scenario: Direct request for task detail returns full page
    Given I created a task via API with title "Full page task" as "full-task"
    When I GET the HTML task "full-task"
    Then the response status is 200
    And the response body contains "<!DOCTYPE html"
    And the response body contains "Full page task"

  # ---------------------------------------------------------------------------
  # Navigation — all pages include nav links
  # ---------------------------------------------------------------------------

  Scenario: Task list page includes navigation
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains "<nav"

  Scenario: Board page includes navigation
    When I GET "/board"
    Then the response status is 200
    And the response body contains "<nav"

  Scenario: Epics page includes navigation
    When I GET "/epics"
    Then the response status is 200
    And the response body contains "<nav"

  # ---------------------------------------------------------------------------
  # Full page base template — direct navigation returns complete HTML
  # ---------------------------------------------------------------------------

  Scenario: Task list page includes full HTML document structure
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains "<!DOCTYPE html"
    And the response body contains "<head"
    And the response body contains "<body"
    And the response body contains "</html>"

  Scenario: Board page includes full HTML document structure
    When I GET "/board"
    Then the response status is 200
    And the response body contains "<!DOCTYPE html"
    And the response body contains "</html>"

  Scenario: Task list page has link to board view
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains "/board"

  Scenario: Task list page has link to epics view
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains "/epics"

  # ---------------------------------------------------------------------------
  # Priority badges — P0 must not render as P4
  # The filter dropdowns on /board and /tasks always contain one badge per
  # priority, so a rendered task badge raises the count from 1 to 2.
  # ---------------------------------------------------------------------------

  Scenario: Board renders a P0 task with a P0 badge
    Given I created a task via API with title "Urgent board task" and priority 0 as "p0-board"
    When I GET "/board"
    Then the response status is 200
    And the response body contains '<span class="badge priority-0">▲▲ P0</span>' exactly 2 times
    And the response body contains '<span class="badge priority-4">· P4</span>' exactly 1 times

  Scenario: Board renders a P4 task with a P4 badge
    Given I created a task via API with title "Backlog board task" and priority 4 as "p4-board"
    When I GET "/board"
    Then the response status is 200
    And the response body contains '<span class="badge priority-4">· P4</span>' exactly 2 times
    And the response body contains '<span class="badge priority-0">▲▲ P0</span>' exactly 1 times

  Scenario: Board priority filter offers P0
    When I GET "/board"
    Then the response status is 200
    And the response body contains 'data-value="0" data-label="▲▲ P0"'

  Scenario: Task list renders a P0 task with a P0 badge
    Given I created a task via API with title "Urgent list task" and priority 0 as "p0-list"
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains '<span class="badge priority-0">▲▲ P0</span>' exactly 2 times
    And the response body contains '<span class="badge priority-4">· P4</span>' exactly 1 times

  Scenario: Task list renders a P4 task with a P4 badge
    Given I created a task via API with title "Backlog list task" and priority 4 as "p4-list"
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains '<span class="badge priority-4">· P4</span>' exactly 2 times
    And the response body contains '<span class="badge priority-0">▲▲ P0</span>' exactly 1 times

  Scenario: Task list priority filter offers P0
    When I GET "/tasks"
    Then the response status is 200
    And the response body contains 'data-value="0" data-label="▲▲ P0"'

  Scenario: Task list filtered to P0 shows only the P0 task
    Given I created a task via API with title "Urgent filtered task" and priority 0 as "p0-filter"
    And I created a task via API with title "Normal filtered task" and priority 2 as "p2-filter"
    When I GET "/tasks?priority=0"
    Then the response status is 200
    And the response body contains "Urgent filtered task"
    And the response body does not contain "Normal filtered task"

  Scenario: Task detail renders a P0 task with a P0 badge
    Given I created a task via API with title "Urgent detail task" and priority 0 as "p0-detail"
    When I GET the HTML task "p0-detail"
    Then the response status is 200
    And the response body contains '<span class="badge priority-0">▲▲ P0</span>' exactly 1 times
    And the response body contains '<span class="badge priority-4">· P4</span>' exactly 0 times

  Scenario: Task detail renders a P4 task with a P4 badge
    Given I created a task via API with title "Backlog detail task" and priority 4 as "p4-detail"
    When I GET the HTML task "p4-detail"
    Then the response status is 200
    And the response body contains '<span class="badge priority-4">· P4</span>' exactly 1 times
    And the response body contains '<span class="badge priority-0">▲▲ P0</span>' exactly 0 times

  Scenario: Epic detail renders a P0 child with a P0 badge
    Given I created a task via API with title "Priority epic" as "p0-epic"
    When I POST resolved "/api/tasks" with body '{"title":"Urgent child","priority":0,"parent_id":"@id(p0-epic)"}'
    Then the response status is 201
    When I GET the HTML epic "p0-epic"
    Then the response status is 200
    And the response body contains "Urgent child"
    And the response body does not contain "· P4"
    And the response body contains "▲▲ P0"
