Feature: Agent hook for cross-workspace commands
  `tk hook pre-tool-use` is a hidden command used by the Claude Code plugin as a
  PreToolUse hook. It reads the hook JSON on stdin and asks the user to confirm any
  Bash command that runs tk in a way that reaches outside the current workspace
  (--scope project|all, --workspace, --move-to, --db, TACKS_WORKSPACE=, TACKS_DB=).
  Otherwise it prints nothing. It always exits 0.

  Scenario Outline: tk invocations that reach outside the workspace ask for confirmation
    When I run the pre-tool-use hook for the Bash command <command>
    Then the hook asks for confirmation naming "<trigger>"

    Examples:
      | command                               | trigger         |
      | tk list --scope all                   | --scope all     |
      | tk ready --scope=project              | --scope project |
      | TACKS_WORKSPACE=/x tk ready           | TACKS_WORKSPACE |
      | export TACKS_DB=/y; tk list           | TACKS_DB        |
      | cd a && tk update X --move-to ../b    | --move-to       |
      | /usr/local/bin/tk list --workspace /x | --workspace     |
      | tk --db /z list                       | --db            |

  Scenario Outline: commands that stay inside the workspace produce no output
    When I run the pre-tool-use hook for the Bash command <command>
    Then the hook prints nothing and exits 0

    Examples:
      | command                    |
      | tk list                    |
      | tk list --scope workspace  |
      | tk comment X "--scope all" |
      | git status                 |
      | echo tk --scope all        |

  Scenario: non-Bash tools are ignored
    When I run the pre-tool-use hook with this stdin:
      """
      {"tool_name":"Read","tool_input":{"command":"tk list --scope all"}}
      """
    Then the hook prints nothing and exits 0

  Scenario: a Bash payload given as raw JSON asks for confirmation
    When I run the pre-tool-use hook with this stdin:
      """
      {"tool_name":"Bash","tool_input":{"command":"tk list --scope all"}}
      """
    Then the hook asks for confirmation naming "--scope all"

  Scenario: invalid JSON on stdin is ignored
    When I run the pre-tool-use hook with this stdin:
      """
      this is { not json
      """
    Then the hook prints nothing and exits 0

  Scenario: the hook command is hidden from help
    Given a tacks database is initialized
    And a directory "plain" outside any git repository
    When I run tk "--help" in "plain"
    Then the tk command succeeds
    And the tk output does not contain "hook"
