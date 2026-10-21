Feature: Board Blocked column holds only human-review tasks
  As a user reading the kanban board
  I want the Blocked column to list only tasks with status blocked
  So that it means "needs my review", while tasks waiting on a dependency
  stay in Open with a visible "blocked by" marker

  Background:
    Given a tacks database is initialized
    And the web server is running

  Scenario: A dependency-blocked open task stays in the Open column
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker"
    When I GET "/board"
    Then the response status is 200
    And the board column "open" contains the task "waiter"
    And the board column "blocked" does not contain the task "waiter"

  Scenario: A dependency-blocked open task shows a blocked by marker with the blocker id
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker"
    When I GET "/board"
    Then the board card of "waiter" shows it is blocked by "blocker"
    And the board card of "waiter" marker tooltip names "blocker" as "Prerequisite"

  Scenario: The blocker itself has no marker
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker"
    When I GET "/board"
    Then the board column "open" contains the task "blocker"
    And the board card of "blocker" shows no blocked by marker

  Scenario: The Blocked column contains only tasks with status blocked
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I created a task via API with title "Needs a decision" as "review"
    And I added API dependency so "waiter" is blocked by "blocker"
    And I set the API task "review" to status "blocked"
    When I GET "/board"
    Then the board column "blocked" contains the task "review"
    And the board column "blocked" does not contain the task "waiter"
    And the board column "blocked" has 1 cards

  Scenario: A status blocked task without dependencies shows no marker
    Given I created a task via API with title "Needs a decision" as "review"
    And I set the API task "review" to status "blocked"
    When I GET "/board"
    Then the board column "blocked" contains the task "review"
    And the board card of "review" shows no blocked by marker

  Scenario: A status blocked task with an open blocker shows the marker in the Blocked column
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Needs a decision" as "review"
    And I added API dependency so "review" is blocked by "blocker"
    And I set the API task "review" to status "blocked"
    When I GET "/board"
    Then the board column "blocked" contains the task "review"
    And the board card of "review" shows it is blocked by "blocker"

  Scenario: An in progress task with an open blocker shows the marker
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Started early" as "early"
    And I added API dependency so "early" is blocked by "blocker"
    And I set the API task "early" to status "in_progress"
    When I GET "/board"
    Then the board column "in_progress" contains the task "early"
    And the board card of "early" shows it is blocked by "blocker"

  Scenario: Dependency-blocked open tasks come after ready open tasks
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I created a task via API with title "Ready task" as "ready"
    And I added API dependency so "waiter" is blocked by "blocker"
    When I GET "/board"
    Then in the board column "open" the task "ready" comes before the task "waiter"
    And in the board column "open" the task "blocker" comes before the task "waiter"

  Scenario: Existing order is kept inside the ready and the waiting groups
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting first" as "wait-1"
    And I created a task via API with title "Ready first" as "ready-1"
    And I created a task via API with title "Waiting second" as "wait-2"
    And I created a task via API with title "Ready second" as "ready-2"
    And I added API dependency so "wait-1" is blocked by "blocker"
    And I added API dependency so "wait-2" is blocked by "blocker"
    When I GET "/board"
    Then in the board column "open" the task "ready-1" comes before the task "ready-2"
    And in the board column "open" the task "ready-2" comes before the task "wait-1"
    And in the board column "open" the task "wait-1" comes before the task "wait-2"

  Scenario: A task with two open blockers lists both ids in blocker creation order
    Given I created a task via API with title "First prerequisite" as "blocker-1"
    And I created a task via API with title "Second prerequisite" as "blocker-2"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker-2"
    And I added API dependency so "waiter" is blocked by "blocker-1"
    When I GET "/board"
    Then the board column "open" contains the task "waiter"
    And the board card of "waiter" shows it is blocked by "blocker-1,blocker-2"
    And the board card of "waiter" marker tooltip names "blocker-1" as "First prerequisite"
    And the board card of "waiter" marker tooltip names "blocker-2" as "Second prerequisite"

  Scenario: A done blocker is dropped from the marker
    Given I created a task via API with title "First prerequisite" as "blocker-1"
    And I created a task via API with title "Second prerequisite" as "blocker-2"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker-1"
    And I added API dependency so "waiter" is blocked by "blocker-2"
    And I closed the API task "blocker-1"
    When I GET "/board"
    Then the board card of "waiter" shows it is blocked by "blocker-2"

  Scenario: Closing the only blocker removes the marker
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker"
    And I closed the API task "blocker"
    When I GET "/board"
    Then the board column "open" contains the task "waiter"
    And the board card of "waiter" shows no blocked by marker

  Scenario: A done blocker no longer pushes the waiting task behind ready tasks
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Formerly waiting" as "waiter"
    And I created a task via API with title "Ready task" as "ready"
    And I added API dependency so "waiter" is blocked by "blocker"
    And I closed the API task "blocker"
    When I GET "/board"
    Then in the board column "open" the task "waiter" comes before the task "ready"

  Scenario: A done task gets no marker even when its blocker is still open
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Finished anyway" as "finished"
    And I added API dependency so "finished" is blocked by "blocker"
    And I closed the API task "finished"
    When I GET "/board"
    Then the board column "done" contains the task "finished"
    And the board card of "finished" shows no blocked by marker

  Scenario: Column header counts follow the split between Open and Blocked
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I created a task via API with title "Ready task" as "ready"
    And I created a task via API with title "Needs a decision" as "review"
    And I added API dependency so "waiter" is blocked by "blocker"
    And I set the API task "review" to status "blocked"
    When I GET "/board"
    Then the board column "open" header count is 3
    And the board column "open" has 3 cards
    And the board column "blocked" header count is 1
    And the board column "blocked" has 1 cards

  Scenario: The Blocked column is empty when only dependency waits exist
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker"
    When I GET "/board"
    Then the board column "blocked" header count is 0
    And the board column "blocked" has 0 cards
    And the board column "open" header count is 2

  Scenario: The priority filter applies to dependency-blocked open tasks
    Given I created a task via API with title "Prerequisite" and priority 1 as "blocker"
    And I created a task via API with title "Urgent waiter" and priority 0 as "urgent-waiter"
    And I created a task via API with title "Low waiter" and priority 3 as "low-waiter"
    And I added API dependency so "urgent-waiter" is blocked by "blocker"
    And I added API dependency so "low-waiter" is blocked by "blocker"
    When I GET "/board?priority=0"
    Then the board column "open" contains the task "urgent-waiter"
    And the board column "open" does not contain the task "low-waiter"
    And the board column "open" header count is 1
    And the board column "blocked" header count is 0

  Scenario: The epic filter applies to dependency-blocked open tasks
    Given I created a task via API with title "Epic one" as "epic-1"
    And I created a task via API with title "Epic two" as "epic-2"
    And I created a task via API with title "Prerequisite" as "blocker"
    And I created a subtask via API with title "Waiting in epic one" under "epic-1" as "waiter-1"
    And I created a subtask via API with title "Waiting in epic two" under "epic-2" as "waiter-2"
    And I added API dependency so "waiter-1" is blocked by "blocker"
    And I added API dependency so "waiter-2" is blocked by "blocker"
    When I GET the board filtered by epic "epic-1"
    Then the board column "open" contains the task "waiter-1"
    And the board column "open" does not contain the task "waiter-2"
    And the board column "open" header count is 1
    And the board column "blocked" header count is 0

  Scenario: The API blocked list still includes dependency-blocked open tasks
    Given I created a task via API with title "Prerequisite" as "blocker"
    And I created a task via API with title "Waiting task" as "waiter"
    And I added API dependency so "waiter" is blocked by "blocker"
    When I GET "/api/tasks/blocked"
    Then the response status is 200
    And the response JSON array contains a task with title "Waiting task"
