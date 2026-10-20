Feature: Project backlog
  A project backlog holds tasks that belong to a project (a git repository) but to
  none of its workspaces (worktrees): workspace_id is null and project_id is set.
  Agents and users park future work there; a workspace later pulls a task out.
  "--scope backlog" selects the current project's backlog. "--to-backlog" on create
  and update puts a task there. Taking a task needs "--claim --scope backlog".
  Writes to backlog tasks are refused in the default scope.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And a worktree "B" of repository "repo"
    And a git repository "other"
    And a worktree "C" of repository "other"
    And a git repository "fresh"
    And a worktree "D" of repository "fresh"
    And a directory "nogit" outside any git repository

  # ---------------------------------------------------------------------------
  # Creating tasks in the backlog
  # ---------------------------------------------------------------------------

  Scenario: create --to-backlog puts the task in the project backlog
    Given I run tk "--json create 'Workspace task'" in "A"
    When I run tk "--json create 'Parked' --to-backlog" in "A"
    Then the tk command succeeds
    And the JSON output path "title" equals "Parked"
    And the JSON output path "workspace_id" equals "null"
    And the JSON output path "project_id" equals "@project(A)"

  Scenario: create --to-backlog registers a project that has no tasks yet
    When I run tk "--json create 'First ever' --to-backlog" in "D"
    Then the tk command succeeds
    And the JSON output path "workspace_id" equals "null"
    When I run tk "--json --scope backlog list" in "D"
    Then the JSON output lists exactly the titles "First ever"

  Scenario: create --to-backlog keeps the task out of the workspace list
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Workspace task"

  Scenario: create --to-backlog cannot be combined with --parent
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    When I run tk "create 'Child' --to-backlog --parent @id(parent)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "error: --to-backlog cannot be used with --parent; subtasks follow their parent"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Parent"

  Scenario: create --to-backlog outside a git repository is an error
    When I run tk "create 'Parked' --to-backlog" in "nogit"
    Then the tk command fails with exit code 1
    And the tk error contains "error: not in a git repository, so there is no project backlog; use --workspace <path>"
    When I run tk "--json list" in "nogit"
    Then the JSON output lists exactly the titles ""

  Scenario: create --to-backlog works outside git with --workspace
    When I run tk "--json --workspace @path(A) create 'Parked' --to-backlog" in "nogit"
    Then the tk command succeeds
    And the JSON output path "workspace_id" equals "null"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked"

  Scenario: a subtask of a backlog task stays in the backlog
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked parent' --to-backlog" in "A" and save the id as "parent"
    When I run tk "--json --scope backlog create 'Parked child' --parent @id(parent)" in "A" and save the id as "child"
    Then the tk command succeeds
    And the JSON output path "workspace_id" equals "null"
    And the JSON output path "project_id" equals "@project(A)"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Workspace task"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked parent,Parked child"

  Scenario: creating a subtask of a backlog task needs --scope backlog
    Given I run tk "--json create 'Parked parent' --to-backlog" in "A" and save the id as "parent"
    When I run tk "create 'Child' --parent @id(parent)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "is in the backlog of project repo, not the current workspace"

  # ---------------------------------------------------------------------------
  # Listing with --scope backlog
  # ---------------------------------------------------------------------------

  Scenario: --scope backlog lists only the backlog of the current project
    Given I run tk "--json create 'Workspace task A'" in "A"
    And I run tk "--json create 'Workspace task B'" in "B"
    And I run tk "--json create 'Backlog task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    And I run tk "--json create 'Parked in other project' --to-backlog" in "C"
    And I run tk "--json create 'Loose task'" in "nogit"
    When I run tk "--json --scope backlog list" in "B"
    Then the JSON output lists exactly the titles "Parked"
    When I run tk "--json --scope backlog list" in "C"
    Then the JSON output lists exactly the titles "Parked in other project"

  Scenario: --scope backlog in human output shows the backlog task
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked work' --to-backlog" in "A"
    When I run tk "--scope backlog list" in "B"
    Then the tk command succeeds
    And the tk output contains "Parked work"
    And the tk output does not contain "Workspace task"

  Scenario: --scope backlog for an unregistered project is empty
    When I run tk "--json --scope backlog list" in "D"
    Then the tk command succeeds
    And the JSON output lists exactly the titles ""

  Scenario: --scope backlog outside a git repository is an error
    When I run tk "--scope backlog list" in "nogit"
    Then the tk command fails with exit code 1
    And the tk error contains "error: not in a git repository, so there is no project backlog; use --workspace <path>"

  Scenario: --workspace selects the backlog of another repository
    Given I run tk "--json create 'Parked in other project' --to-backlog" in "C"
    And I run tk "--json create 'Parked here' --to-backlog" in "A"
    When I run tk "--json --workspace @path(C) --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked in other project"
    When I run tk "--json --workspace @path(A) --scope backlog list" in "nogit"
    Then the JSON output lists exactly the titles "Parked here"

  Scenario: --scope project includes the backlog
    Given I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    And I run tk "--json create 'Parked in other project' --to-backlog" in "C"
    When I run tk "--json --scope project list" in "A"
    Then the JSON output lists exactly the titles "Task in A,Task in B,Parked"

  Scenario: --scope all includes backlog tasks of every project
    Given I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    And I run tk "--json create 'Parked in other project' --to-backlog" in "C"
    And I run tk "--json create 'Loose task'" in "nogit"
    When I run tk "--json --scope all list" in "A"
    Then the JSON output lists exactly the titles "Task in A,Parked,Parked in other project,Loose task"

  Scenario: the default scope excludes backlog tasks
    Given I run tk "--json create 'Task in A'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Task in A"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles ""

  Scenario: the unscoped bucket outside git excludes backlog tasks
    Given I run tk "--json create 'Parked' --to-backlog" in "A"
    And I run tk "--json create 'Loose task'" in "nogit"
    When I run tk "--json list" in "nogit"
    Then the JSON output lists exactly the titles "Loose task"

  Scenario: ready lists backlog tasks with --scope backlog
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "--json --scope backlog ready" in "B"
    Then the JSON output lists exactly the titles "Parked"
    When I run tk "--json ready" in "A"
    Then the JSON output lists exactly the titles "Workspace task"

  Scenario: blocked lists backlog tasks with --scope backlog
    Given I run tk "--json create 'Blocker' --to-backlog" in "A" and save the id as "blocker"
    And I run tk "--json create 'Waiting' --to-backlog" in "A" and save the id as "waiting"
    And I run tk "--scope backlog dep add @id(waiting) @id(blocker)" in "A"
    When I run tk "--json --scope backlog blocked" in "B"
    Then the JSON output lists exactly the titles "Waiting"
    When I run tk "--json blocked" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: epic lists backlog epics with --scope backlog
    Given I run tk "--json create 'Parked epic' --to-backlog" in "A" and save the id as "epic"
    And I run tk "--json --scope backlog create 'Parked child' --parent @id(epic)" in "A"
    When I run tk "--json --scope backlog epic" in "B"
    Then the JSON output lists exactly the titles "Parked epic"
    When I run tk "--json epic" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: stats count only backlog tasks with --scope backlog
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked one' --to-backlog" in "A"
    And I run tk "--json create 'Parked two' --to-backlog" in "A"
    When I run tk "--json --scope backlog stats" in "B"
    Then the stats JSON counts 2 tasks in total
    When I run tk "--json stats" in "A"
    Then the stats JSON counts 1 tasks in total

  # ---------------------------------------------------------------------------
  # Moving tasks into the backlog
  # ---------------------------------------------------------------------------

  Scenario: update --to-backlog moves a task and its subtasks to the backlog
    Given I run tk "--json create 'Parent'" in "A" and save the id as "parent"
    And I run tk "--json create 'Child' --parent @id(parent)" in "A" and save the id as "child"
    And I run tk "--json create 'Stays'" in "A" and save the id as "stays"
    When I run tk "update @id(parent) --to-backlog" in "A"
    Then the tk command succeeds
    And the task "parent" has workspace_id "null"
    And the task "child" has workspace_id "null"
    And the task "stays" has workspace_id "@workspace(A)"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles "Stays"
    When I run tk "--json --scope backlog list" in "B"
    Then the JSON output lists exactly the titles "Parent,Child"

  Scenario: update --to-backlog keeps the project of the task
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "update @id(ta) --to-backlog" in "A"
    And I run tk "--json show @id(ta)" in "A"
    Then the JSON output path "workspace_id" equals "null"
    And the JSON output path "project_id" equals "@project(A)"
    And the JSON output path "project.name" equals "repo"

  Scenario: update --to-backlog sends an unscoped task to the current project backlog
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "--scope all update @id(loose) --to-backlog" in "A"
    Then the tk command succeeds
    When I run tk "--json --scope backlog list" in "B"
    Then the JSON output lists exactly the titles "Loose task"

  Scenario: update --to-backlog cannot be combined with --move-to
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "update @id(ta) --to-backlog --move-to @path(B)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "error: --to-backlog cannot be used with --move-to"
    And the task "ta" has workspace_id "@workspace(A)"

  Scenario: update --to-backlog cannot be combined with --claim
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "update @id(ta) --to-backlog --claim" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "error: --to-backlog cannot be used with --claim; claiming moves the task into the current workspace"
    And the task "ta" has workspace_id "@workspace(A)"

  Scenario: update --to-backlog on a task of another workspace is refused
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "update @id(ta) --to-backlog" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A (project repo)"
    And the task "ta" has workspace_id "@workspace(A)"

  # ---------------------------------------------------------------------------
  # Write guard
  # ---------------------------------------------------------------------------

  Scenario Outline: <command> is refused for a backlog task in the default scope
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    And I remember the updated_at of task "parked"
    When I run tk "<command>" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains resolved "@id(parked) is in the backlog of project repo, not the current workspace. rerun with --scope backlog"
    And the tk error does not contain "unless the user"
    When I run tk "--json show @id(parked)" in "A"
    Then the JSON output path "title" equals "Parked"
    And the JSON output path "status" equals "open"
    And the JSON output path "assignee" equals "null"
    And the JSON output path "notes" equals "null"
    And the JSON output path "workspace_id" equals "null"
    And the JSON output path "comments" equals "[]"
    And the updated_at of task "parked" is unchanged

    Examples:
      | command                               |
      | update @id(parked) --title Renamed    |
      | update @id(parked) --notes secret     |
      | update @id(parked) --claim            |
      | update @id(parked) --move-to @path(A) |
      | close @id(parked)                     |
      | comment @id(parked) hello             |

  Scenario: the backlog guard also applies from another workspace of the project
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "update @id(parked) --title Renamed" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "is in the backlog of project repo, not the current workspace"
    And the tk error contains "--scope backlog"

  Scenario: writing to the backlog of another project is refused and names --scope all
    Given I run tk "--json create 'Parked elsewhere' --to-backlog" in "C" and save the id as "parked"
    When I run tk "update @id(parked) --title Renamed" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "is in the backlog of project other, not the current workspace"
    And the tk error contains "do not act on it unless the user explicitly asked you to; if they did, rerun with --scope all"
    When I run tk "--json show @id(parked)" in "C"
    Then the JSON output path "title" equals "Parked elsewhere"

  Scenario: --scope backlog allows writes to a backlog task of the current project
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "--scope backlog update @id(parked) --title Renamed --notes later" in "B"
    Then the tk command succeeds
    When I run tk "--scope backlog comment @id(parked) hello" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(parked)" in "B"
    Then the JSON output path "title" equals "Renamed"
    And the JSON output path "notes" equals "later"
    And the JSON output path "workspace_id" equals "null"
    When I run tk "--scope backlog close @id(parked)" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(parked)" in "B"
    Then the JSON output path "status" equals "done"

  Scenario: --scope project allows writes to a backlog task of the current project
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "--scope project update @id(parked) --title Renamed" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(parked)" in "B"
    Then the JSON output path "title" equals "Renamed"
    And the JSON output path "workspace_id" equals "null"

  Scenario: --scope backlog refuses a task of a workspace
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--scope backlog update @id(ta) --title Renamed" in "B"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A (project repo), not the current project's backlog"
    And the tk error contains "do not act on it unless the user explicitly asked you to; if they did, rerun with --scope project"
    And the task "ta" has title "Task in A"

  Scenario: --scope backlog refuses a task of the current workspace
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--scope backlog close @id(ta)" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "belongs to workspace A (project repo), not the current project's backlog"
    When I run tk "--json show @id(ta)" in "A"
    Then the JSON output path "status" equals "open"

  # ---------------------------------------------------------------------------
  # Pulling a backlog task into a workspace
  # ---------------------------------------------------------------------------

  Scenario: claiming with --scope backlog moves the task into the current workspace
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "update @id(parked) --claim --scope backlog" in "B"
    Then the tk command succeeds
    When I run tk "--json show @id(parked)" in "B"
    Then the JSON output path "status" equals "in_progress"
    And the JSON output path "assignee" equals "agent"
    And the JSON output path "workspace_id" equals "@workspace(B)"
    And the JSON output path "project_id" equals "@project(B)"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles "Parked"
    When I run tk "--json --scope backlog list" in "B"
    Then the JSON output lists exactly the titles ""
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: claiming a backlog task moves its subtasks too
    Given I run tk "--json create 'Parked parent' --to-backlog" in "A" and save the id as "parent"
    And I run tk "--json --scope backlog create 'Parked child' --parent @id(parent)" in "A" and save the id as "child"
    When I run tk "update @id(parent) --claim --scope backlog" in "B"
    Then the tk command succeeds
    And the task "parent" has workspace_id "@workspace(B)"
    And the task "child" has workspace_id "@workspace(B)"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles "Parked parent,Parked child"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: claiming a backlog task works with --scope project
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "update @id(parked) --claim --scope project" in "B"
    Then the tk command succeeds
    And the task "parked" has workspace_id "@workspace(B)"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles "Parked"

  Scenario: claiming a backlog task into a workspace that has no tasks yet registers it
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "update @id(parked) --claim --scope backlog" in "B"
    Then the tk command succeeds
    And the workspaces JSON row for "B" has "open" equal to "0"
    And the workspaces JSON row for "B" has "in_progress" equal to "1"

  Scenario: a backlog task of another project cannot be pulled with --scope backlog
    Given I run tk "--json create 'Parked elsewhere' --to-backlog" in "C" and save the id as "parked"
    When I run tk "update @id(parked) --claim --scope backlog" in "A"
    Then the tk command fails with exit code 1
    And the task "parked" has workspace_id "null"
    When I run tk "--json --scope backlog list" in "C"
    Then the JSON output lists exactly the titles "Parked elsewhere"

  # ---------------------------------------------------------------------------
  # show
  # ---------------------------------------------------------------------------

  Scenario: show prints the project of a backlog task
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "show @id(parked)" in "B"
    Then the tk command succeeds
    And the tk output contains "Workspace:   none (backlog of project repo)"

  Scenario: show of an unscoped task still prints no project
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "show @id(loose)" in "A"
    Then the tk command succeeds
    And the tk output contains "Workspace:   none"
    And the tk output does not contain "backlog of project"

  Scenario: show --json has a project object for a backlog task
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "--json show @id(parked)" in "B"
    Then the JSON output path "workspace" equals "null"
    And the JSON output path "project.id" equals "@project(A)"
    And the JSON output path "project.name" equals "repo"
    And the JSON output path "project.path" equals "@path(repo)"

  Scenario: show --json has a project object for a workspace task
    Given I run tk "--json create 'Task in A'" in "A" and save the id as "ta"
    When I run tk "--json show @id(ta)" in "B"
    Then the JSON output path "project.id" equals "@project(A)"
    And the JSON output path "project.name" equals "repo"
    And the JSON output path "workspace.name" equals "A"

  Scenario: show --json has a null project for unscoped tasks
    Given I run tk "--json create 'Loose task'" in "nogit" and save the id as "loose"
    When I run tk "--json show @id(loose)" in "A"
    Then the JSON output path "project" equals "null"
    And the JSON output path "project_id" equals "null"

  # ---------------------------------------------------------------------------
  # Moving a backlog task elsewhere
  # ---------------------------------------------------------------------------

  Scenario: --move-to a path puts a backlog task in that workspace
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "--scope backlog update @id(parked) --move-to @path(B)" in "A"
    Then the tk command succeeds
    And the task "parked" has workspace_id "@workspace(B)"
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles "Parked"

  Scenario: --move-to none makes a backlog task unscoped
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "--scope backlog update @id(parked) --move-to none" in "A"
    Then the tk command succeeds
    And the task "parked" has workspace_id "null"
    When I run tk "--json show @id(parked)" in "A"
    Then the JSON output path "project_id" equals "null"
    And the JSON output path "project" equals "null"
    When I run tk "--json list" in "nogit"
    Then the JSON output lists exactly the titles "Parked"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles ""

  Scenario: --move-to none on a backlog task needs --scope backlog
    Given I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    When I run tk "update @id(parked) --move-to none" in "A"
    Then the tk command fails with exit code 1
    And the tk error contains "is in the backlog of project repo, not the current workspace"
    When I run tk "--json --scope backlog list" in "A"
    Then the JSON output lists exactly the titles "Parked"

  Scenario: a workspace task can go to the backlog and come back
    Given I run tk "--json create 'Round trip'" in "A" and save the id as "task"
    When I run tk "update @id(task) --to-backlog" in "A"
    And I run tk "update @id(task) --claim --scope backlog" in "B"
    Then the tk command succeeds
    When I run tk "--json list" in "B"
    Then the JSON output lists exactly the titles "Round trip"
    When I run tk "--json list" in "A"
    Then the JSON output lists exactly the titles ""

  # ---------------------------------------------------------------------------
  # prime
  # ---------------------------------------------------------------------------

  Scenario: prime shows the backlog count of the project in the stats section
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked one' --to-backlog" in "A"
    And I run tk "--json create 'Parked two' --to-backlog" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    When I run tk "prime" in "B"
    Then the tk command succeeds
    And the tk output has the line "Project backlog: 2 open tasks (tk list --scope backlog; claim one with tk update <id> --claim --scope backlog)" directly under the stats line

  Scenario: prime uses the singular for a backlog with one task
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "prime" in "A"
    Then the tk command succeeds
    And the tk output has the line "Project backlog: 1 open task (tk list" directly under the stats line

  Scenario: prime does not count done backlog tasks
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked done' --to-backlog" in "A" and save the id as "done"
    And I run tk "--json create 'Parked open' --to-backlog" in "A"
    And I run tk "--scope backlog close @id(done)" in "A"
    When I run tk "prime" in "A"
    Then the tk output has the line "Project backlog: 1 open task (" directly under the stats line

  Scenario: prime counts in_progress and blocked backlog tasks
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked open' --to-backlog" in "A"
    And I run tk "--json create 'Parked started' --to-backlog" in "A" and save the id as "started"
    And I run tk "--json create 'Parked blocked' --to-backlog" in "A" and save the id as "blocked"
    And I run tk "--scope backlog update @id(started) -s in_progress" in "A"
    And I run tk "--scope backlog update @id(blocked) -s blocked" in "A"
    When I run tk "prime" in "A"
    Then the tk output has the line "Project backlog: 3 open tasks (" directly under the stats line

  Scenario: prime hides the backlog line when the backlog only has done tasks
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A" and save the id as "parked"
    And I run tk "--scope backlog close @id(parked)" in "A"
    When I run tk "prime" in "A"
    Then the tk command succeeds
    And the tk output contains "## Stats"
    And the tk output does not contain "Project backlog"

  Scenario: prime hides the backlog line when the backlog is empty
    Given I run tk "--json create 'Workspace task'" in "A"
    When I run tk "prime" in "A"
    Then the tk command succeeds
    And the tk output contains "## Stats"
    And the tk output does not contain "Project backlog"

  Scenario: prime does not show the backlog of another project
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked elsewhere' --to-backlog" in "C"
    When I run tk "prime" in "A"
    Then the tk command succeeds
    And the tk output does not contain "Project backlog"

  Scenario Outline: prime hides the backlog line with --scope <scope>
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "--scope <scope> prime" in "A"
    Then the tk command succeeds
    And the tk output contains "## Stats"
    And the tk output does not contain "Project backlog"

    Examples:
      | scope   |
      | backlog |
      | project |
      | all     |

  Scenario: prime --json reports the backlog counts of the project
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked open' --to-backlog" in "A"
    And I run tk "--json create 'Parked started' --to-backlog" in "A" and save the id as "started"
    And I run tk "--json create 'Parked blocked' --to-backlog" in "A" and save the id as "blocked"
    And I run tk "--json create 'Parked done' --to-backlog" in "A" and save the id as "done"
    And I run tk "--scope backlog update @id(started) -s in_progress" in "A"
    And I run tk "--scope backlog update @id(blocked) -s blocked" in "A"
    And I run tk "--scope backlog close @id(done)" in "A"
    And I run tk "--json create 'Task in B'" in "B"
    When I run tk "--json prime" in "B"
    Then the tk command succeeds
    And the JSON output path "backlog.project_id" equals "@project(A)"
    And the JSON output path "backlog.open" equals "1"
    And the JSON output path "backlog.in_progress" equals "1"
    And the JSON output path "backlog.blocked" equals "1"
    And the JSON output path "stats.open" equals "1"
    And the JSON output path "ready.0.title" equals "Task in B"

  Scenario: prime --json reports zero backlog counts for a project with an empty backlog
    Given I run tk "--json create 'Workspace task'" in "A"
    When I run tk "--json prime" in "A"
    Then the tk command succeeds
    And the JSON output path "backlog.project_id" equals "@project(A)"
    And the JSON output path "backlog.open" equals "0"
    And the JSON output path "backlog.in_progress" equals "0"
    And the JSON output path "backlog.blocked" equals "0"

  Scenario: prime --json reports the backlog also with --scope project
    Given I run tk "--json create 'Workspace task'" in "A"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "--json --scope project prime" in "A"
    Then the JSON output path "backlog.project_id" equals "@project(A)"
    And the JSON output path "backlog.open" equals "1"

  Scenario: prime --json has a null backlog outside a git repository
    Given I run tk "--json create 'Loose task'" in "nogit"
    And I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "--json prime" in "nogit"
    Then the tk command succeeds
    And the JSON output path "backlog" equals "null"
    And the JSON output path "stats.open" equals "1"

  Scenario: prime --json keeps its existing keys
    Given I run tk "--json create 'Workspace task'" in "A"
    When I run tk "--json prime" in "A"
    Then the JSON output path "stats.open" equals "1"
    And the JSON output path "in_progress" equals "[]"
    And the JSON output path "user_feedback" equals "[]"
    And the JSON output path "ready.0.title" equals "Workspace task"

  Scenario: prime stays silent in a workspace that never used tacks
    Given I run tk "--json create 'Parked' --to-backlog" in "A"
    When I run tk "prime" in "D"
    Then the tk output is empty
    When I run tk "--json prime" in "D"
    Then the tk output is empty

  # ---------------------------------------------------------------------------
  # Hook
  # ---------------------------------------------------------------------------

  Scenario Outline: tk invocations that read or take from the backlog do not ask for confirmation
    When I run the pre-tool-use hook for the Bash command <command>
    Then the hook prints nothing and exits 0

    Examples:
      | command                                   |
      | tk list --scope backlog                   |
      | tk list --scope=backlog                   |
      | tk update X --claim --scope backlog       |
      | bash -c 'tk ready --scope backlog'        |

  Scenario: a command that mixes --scope backlog with a wider scope still asks for confirmation
    When I run the pre-tool-use hook for the Bash command tk list --scope backlog --scope all
    Then the hook asks for confirmation naming "--scope all"

  Scenario Outline: moving into the backlog does not ask for confirmation
    When I run the pre-tool-use hook for the Bash command <command>
    Then the hook prints nothing and exits 0

    Examples:
      | command                      |
      | tk update X --to-backlog     |
      | tk create "X" --to-backlog   |
