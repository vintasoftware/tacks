Feature: Workspace scoping
  Tasks belong to the git worktree ("workspace") they were created in. Workspaces
  are grouped by repository ("project"). List-type commands only show the current
  workspace unless widened with --scope. ID-based commands work across workspaces.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a directory "nogit" outside any git repository

  # ---------------------------------------------------------------------------
  # Scoped list-type commands
  # ---------------------------------------------------------------------------

  Scenario: list only shows tasks of the current workspace
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    And I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Task in A"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles "Task in B"

  Scenario: list in a workspace that never created a task is empty
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles ""

  Scenario: ready only shows tasks of the current workspace
    When I run tk "--json create 'Ready in A'" in "A"
    And I run tk "--json create 'Ready in B'" in "B"
    And I run tk "--json ready" in "A"
    Then the JSON output lists exactly the titles "Ready in A"

  Scenario: blocked only shows tasks of the current workspace
    Given I run tk "--json create 'Blocker'" in "A" and save the id as "blocker"
    And I run tk "--json create 'Waiting'" in "A" and save the id as "waiting"
    And I run tk "dep add @id(waiting) @id(blocker)" in "A"
    When I run tk "--json blocked" in "A"
    Then the JSON output lists exactly the titles "Waiting"
    When I run tk "--json blocked" in "B"
    Then the JSON output lists exactly the titles ""

  Scenario: stats only count tasks of the current workspace
    When I run tk "--json create 'One'" in "A"
    And I run tk "--json create 'Two'" in "A"
    And I run tk "--json create 'Three'" in "B"
    And I run tk "--json stats" in "A"
    Then the stats JSON counts 2 tasks in total
    When I run tk "--json stats" in "B"
    Then the stats JSON counts 1 tasks in total

  Scenario: epic only shows epics of the current workspace
    Given I run tk "--json create 'Epic in A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Child' --parent @id(epic)" in "A"
    When I run tk "--json epic" in "A"
    Then the JSON output lists exactly the titles "Epic in A"
    When I run tk "--json epic" in "B"
    Then the JSON output lists exactly the titles ""

  # ---------------------------------------------------------------------------
  # --scope widening
  # ---------------------------------------------------------------------------

  Scenario: --scope project shows every workspace of the project
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    And I run tk "--json create 'Task outside git'" in "nogit"
    And I run tk "--json --scope project list" in "A"
    Then the JSON output lists exactly the titles "Task in A,Task in B"

  Scenario: --scope all also shows unscoped tasks
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    And I run tk "--json create 'Task outside git'" in "nogit"
    And I run tk "--json --scope all list" in "A"
    Then the JSON output lists exactly the titles "Task in A,Task in B,Task outside git"

  Scenario: --scope project widens ready and stats
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    And I run tk "--json --scope project ready" in "A"
    Then the JSON output lists exactly the titles "Task in A,Task in B"
    When I run tk "--json --scope project stats" in "A"
    Then the stats JSON counts 2 tasks in total

  Scenario: tasks created outside any git repository are unscoped
    When I run tk "--json create 'Loose task'" in "nogit"
    Then the JSON output path "workspace_id" equals "null"
    When I run tk "--json list" in "nogit"
    Then the JSON output lists exactly the titles "Loose task"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles ""

  # ---------------------------------------------------------------------------
  # Scope resolution
  # ---------------------------------------------------------------------------

  Scenario: running from a subdirectory resolves to the same workspace
    Given a subdirectory "src/deep" inside "A"
    When I run tk "--json create 'From the root'" in "A"
    And I run tk "--json create 'From a subdirectory'" in "A/src/deep"
    And I run tk "--json list" in "A/src/deep"
    Then the JSON output lists exactly the titles "From the root,From a subdirectory"
    And the workspaces JSON row for "A" has "open" equal to "2"

  Scenario: TACKS_WORKSPACE overrides the current directory
    Given the environment variable "TACKS_WORKSPACE" is set to "@path(B)"
    When I run tk "--json create 'Created via env'" in "A"
    Then the workspaces JSON row for "B" has "open" equal to "1"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Created via env"

  Scenario: --workspace flag overrides the current directory
    When I run tk "--json --workspace @path(B) create 'Created via flag'" in "A"
    And I run tk "--json --workspace @path(B) list" in "A"
    Then the JSON output lists exactly the titles "Created via flag"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: --db is still honored
    Given I run tk "--json create 'Default db task'" in "A"
    When I run tk "--db @path(nogit)/other.db --json list" in "A"
    Then the JSON output lists exactly the titles ""

  # ---------------------------------------------------------------------------
  # Cross-workspace ID commands
  # ---------------------------------------------------------------------------

  Scenario: a subtask inherits its parent's workspace even from another worktree
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    When I run tk "--json --scope project create 'Child' --parent @id(parent)" in "B" and save the id as "child"
    Then the JSON output path "workspace_id" equals "@workspace(A)"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles ""

  Scenario: show works for a task of another workspace
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--json show @id(ta)" in "B"
    Then the tk command succeeds
    And the JSON output path "title" equals "Task in A"
    And the JSON output path "workspace.name" equals "A"

  Scenario: an agent in another workspace can claim a task by ID with --scope project
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--scope project update @id(ta) --claim" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "status" equals "in_progress"
    And the JSON output path "workspace_id" equals "@workspace(A)"

  # ---------------------------------------------------------------------------
  # Moving tasks
  # ---------------------------------------------------------------------------

  Scenario: update --move-to moves a task and its subtasks
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    And I run tk "--json create 'Child' --parent @id(parent)" in "A" and save the id as "child"
    And I run tk "--json create 'Stays'" in "A" and save the id as "stays"
    When I run tk "update @id(parent) --move-to @path(B)" in "A"
    Then the tk command succeeds
    And the task "parent" has workspace_id "@workspace(B)"
    And the task "child" has workspace_id "@workspace(B)"
    And the task "stays" has workspace_id "@workspace(A)"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Stays"

  Scenario: update --move-to none moves a task and its subtasks to the unscoped bucket
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    And I run tk "--json create 'Child' --parent @id(parent)" in "A" and save the id as "child"
    When I run tk "update @id(parent) --move-to none" in "A"
    Then the tk command succeeds
    And the task "parent" has workspace_id "null"
    And the task "child" has workspace_id "null"
    When I run tk "--json list" in "nogit"
    Then the JSON output lists exactly the titles "Parent,Child"

  Scenario: reparenting under an epic of another workspace moves the task and its subtree
    Given I run tk "--json create 'Epic in A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Mover'" in "B" and save the id as "mover"
    And I run tk "--json create 'Mover child' --parent @id(mover)" in "B" and save the id as "mchild"
    When I run tk "--scope project update @id(mover) --parent @id(epic)" in "B"
    Then the tk command succeeds
    And the task "mover" has workspace_id "@workspace(A)"
    And the task "mchild" has workspace_id "@workspace(A)"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Epic in A,Mover,Mover child"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles ""

  Scenario: promoting a subtask with --parent none keeps its workspace
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    And I run tk "--json create 'Child' --parent @id(parent)" in "A" and save the id as "child"
    When I run tk "--scope project update @id(child) --parent none" in "B"
    Then the tk command succeeds
    And the task "child" has workspace_id "@workspace(A)"

  Scenario: --move-to wins over a simultaneous --parent
    Given a git repository "other"
    And a worktree "C" of repository "other"
    And I run tk "--json create 'Epic in A'" in "A" and save the id as "epic"
    And I run tk "--json create 'Mover'" in "B" and save the id as "mover"
    And I run tk "--json create 'Mover child' --parent @id(mover)" in "B" and save the id as "mchild"
    When I run tk "--scope project update @id(mover) --parent @id(epic) --move-to @path(C)" in "B"
    Then the tk command succeeds
    And the task "mover" has workspace_id "@workspace(C)"
    And the task "mchild" has workspace_id "@workspace(C)"

  Scenario: moving an epic keeps updated_at of done subtasks and bumps open ones
    Given I run tk "--json create 'Epic'" in "A" and save the id as "epic"
    And I run tk "--json create 'Done child' --parent @id(epic)" in "A" and save the id as "done"
    And I run tk "--json create 'Open child' --parent @id(epic)" in "A" and save the id as "open"
    And I run tk "close @id(done)" in "A"
    And I remember the updated_at of task "done"
    And I remember the updated_at of task "open"
    When I run tk "update @id(epic) --move-to @path(B)" in "A"
    Then the tk command succeeds
    And the task "done" has workspace_id "@workspace(B)"
    And the task "open" has workspace_id "@workspace(B)"
    And the updated_at of task "done" is unchanged
    And the updated_at of task "open" has changed

  Scenario: a failed update does not partially apply
    Given I run tk "--json create 'Old title'" in "A" and save the id as "task"
    When I run tk "update @id(task) --title New --move-to /nonexistent/path" in "A"
    Then the tk command fails with exit code 1
    And the task "task" has title "Old title"
    And the task "task" has workspace_id "@workspace(A)"

  # ---------------------------------------------------------------------------
  # Workspace metadata and missing workspaces
  # ---------------------------------------------------------------------------

  Scenario: show --json has a workspace object
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--json show @id(ta)" in "A"
    Then the JSON output path "workspace_id" equals "@workspace(A)"
    And the JSON output path "workspace.id" equals "@workspace(A)"
    And the JSON output path "workspace.name" equals "A"
    And the JSON output path "workspace.path" equals "@path(A)"
    And the JSON output path "workspace.project_id" equals "@project(A)"
    And the JSON output path "workspace.project_name" equals "repo"
    And the JSON output path "workspace.missing" equals "false"

  Scenario: show --json has a null workspace for unscoped tasks
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "--json show @id(loose)" in "A"
    Then the JSON output path "workspace" equals "null"

  Scenario: worktrees of one repository share a project
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    Then the workspaces JSON row for "A" has "project_id" equal to "@project(B)"
    And the workspaces JSON row for "A" has "project_name" equal to "repo"

  Scenario: tk workspaces human output has a BLOCKED column
    Given I run tk "--json create 'Task in A'" in "A"
    When I run tk "workspaces" in "A"
    Then the tk command succeeds
    And the tk output contains "BLOCKED"

  Scenario: tk workspaces reports task counts and the current workspace
    When I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    And I run tk "--json workspaces" in "A"
    Then the workspaces output has 2 workspaces
    And the JSON output path "0.current" equals "true"
    And the JSON output path "1.current" equals "false"

  Scenario: a workspace becomes missing after its directory is removed
    Given I run tk "--json create 'Orphaned task'" in "B" and save the id as "orphan"
    When I run tk "--json show @id(orphan)" in "A"
    Then the JSON output path "workspace.missing" equals "false"
    When I remove the directory "B"
    Then the workspaces JSON row for "B" has "missing" equal to "true"
    When I run tk "--json show @id(orphan)" in "A"
    Then the JSON output path "workspace.missing" equals "true"
    And the JSON output path "title" equals "Orphaned task"

  Scenario: tasks of a missing workspace can be claimed or moved from elsewhere
    Given I run tk "--json create 'Orphaned task'" in "B" and save the id as "orphan"
    And I remove the directory "B"
    When I run tk "--scope project update @id(orphan) --move-to @path(A)" in "A"
    Then the tk command succeeds
    And the task "orphan" has workspace_id "@workspace(A)"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Orphaned task"

  # ---------------------------------------------------------------------------
  # prime
  # ---------------------------------------------------------------------------

  Scenario: prime is silent for an unregistered workspace
    Given I run tk "--json create 'Task in A'" in "A"
    When I run tk "prime" in "B"
    Then the tk output is empty

  Scenario: prime prints for a registered workspace
    Given I run tk "--json create 'Task in A'" in "A"
    When I run tk "--json prime" in "A"
    Then the tk command succeeds
    And the JSON output path "ready.0.title" equals "Task in A"

  Scenario: prime is silent when TACKS_WORKSPACE cannot be resolved
    Given I run tk "--json create 'Task in A'" in "A"
    And the environment variable "TACKS_WORKSPACE" is set to "/nonexistent"
    When I run tk "prime" in "A"
    Then the tk output is empty
