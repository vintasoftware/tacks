Feature: Web UI feedback and security guards
  The web UI posts comments as the user, shows pending feedback, sanitizes rendered
  markdown, and rejects cross-origin writes and foreign Host headers.
  See docs/web-security.md and docs/user-feedback.md.

  Background:
    Given a tacks database is initialized
    And a git repository "repo"
    And a worktree "A" of repository "repo"
    And I run tk "--json create 'Web task'" in "A" and save the id as "t"
    And the web server is running

  # ---------------------------------------------------------------------------
  # Authorship and feedback display
  # ---------------------------------------------------------------------------

  Scenario: a web comment is stored with author user
    When I POST resolved "/api/tasks/@id(t)/comments" with body '{"body":"x"}'
    Then the response status is 201
    And the response JSON field "author" equals "user"

  Scenario: the comments endpoint accepts a custom author
    When I POST resolved "/api/tasks/@id(t)/comments" with body '{"body":"x","author":"alice"}'
    Then the response status is 201
    And the response JSON field "author" equals "alice"

  Scenario: an author longer than 64 characters is rejected
    When I POST resolved "/api/tasks/@id(t)/comments" with body '{"body":"x","author":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
    Then the response status is 422

  Scenario: closing from the web stores the comment as the user and ignores pending feedback
    Given I run tk "comment @id(t) 'please wait' --author user" in "A"
    When I POST resolved "/api/tasks/@id(t)/close" with body '{"comment":"closing from web"}'
    Then the response status is 200
    When I run tk "--json show @id(t)" in "A"
    Then the JSON output path "status" equals "done"
    And the JSON output path "comments.1.body" equals "closing from web"
    And the JSON output path "comments.1.author" equals "user"

  Scenario: the task detail page has a comment form and the awaiting marker
    Given I run tk "comment @id(t) 'please wait' --author user" in "A"
    When I GET resolved "/tasks/@id(t)"
    Then the response status is 200
    And the response body contains "comment-form"
    And the response body contains "awaiting agent reply"

  Scenario: the comments fragment shows the awaiting marker until an agent replies
    Given I run tk "comment @id(t) 'please wait' --author user" in "A"
    When I GET resolved "/tasks/@id(t)/comments"
    Then the response status is 200
    And the response body contains "please wait"
    And the response body contains "awaiting agent reply"
    Given I run tk "comment @id(t) 'on it'" in "A"
    When I GET resolved "/tasks/@id(t)/comments"
    Then the response body does not contain "awaiting agent reply"

  Scenario Outline: the <view> page shows the pending count tooltip
    Given I run tk "comment @id(t) 'please wait' --author user" in "A"
    When I GET resolved "<view>"
    Then the response status is 200
    And the response body contains "1 user comment awaiting agent reply"

    Examples:
      | view   |
      | /board |
      | /tasks |

  # ---------------------------------------------------------------------------
  # Markdown sanitization
  # ---------------------------------------------------------------------------

  Scenario Outline: dangerous markup <payload> in a comment is removed from the task detail page
    Given I posted a comment "<payload>" on API task "t"
    When I GET the HTML task "t"
    Then the response status is 200
    And the response body does not contain "alert(1)"
    And the response body does not contain "onerror"
    And the response body does not contain "javascript:"

    Examples:
      | payload                      |
      | <script>alert(1)</script>    |
      | <img src=x onerror=alert(1)> |
      | [x](javascript:alert(1))     |

  Scenario Outline: dangerous markup <payload> in a description is removed from the task detail page
    Given I created a task via API with title "Evil" and description "<payload>" as "evil"
    When I GET the HTML task "evil"
    Then the response status is 200
    And the response body does not contain "alert(1)"
    And the response body does not contain "onerror"
    And the response body does not contain "javascript:"

    Examples:
      | payload                      |
      | <script>alert(1)</script>    |
      | <img src=x onerror=alert(1)> |
      | [x](javascript:alert(1))     |

  Scenario: normal markdown still renders after sanitization
    Given I posted a comment "this is **bold** text" on API task "t"
    When I GET the HTML task "t"
    Then the response body contains "<strong>bold</strong>"

  # ---------------------------------------------------------------------------
  # Cross-origin and Host guards
  # ---------------------------------------------------------------------------

  Scenario: a POST with a foreign Origin is rejected
    When I POST "/api/tasks" with body '{"title":"Evil"}' and header "Origin" set to "http://evil.example"
    Then the response status is 403
    When I GET "/api/tasks"
    Then the response body does not contain "Evil"

  Scenario: a POST with Sec-Fetch-Site cross-site is rejected
    When I POST "/api/tasks" with body '{"title":"Evil"}' and header "Sec-Fetch-Site" set to "cross-site"
    Then the response status is 403

  Scenario: a POST with a same-origin Origin succeeds
    When I POST "/api/tasks" with body '{"title":"Same origin"}' and header "Origin" set to "http://127.0.0.1:@port"
    Then the response status is 201

  Scenario: a POST without an Origin header succeeds
    When I POST "/api/tasks" with body '{"title":"No origin"}'
    Then the response status is 201

  Scenario: a GET with a foreign Host header is rejected
    When I GET "/api/tasks" with header "Host" set to "evil.example"
    Then the response status is 403

  Scenario: a GET with a localhost Host header succeeds
    When I GET "/api/tasks" with header "Host" set to "localhost:@port"
    Then the response status is 200
