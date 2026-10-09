use std::path::Path;

use crate::db::Database;
use crate::scope::{self, Scope};

/// Maximum length of a comment author label, in characters.
const MAX_AUTHOR_CHARS: usize = 64;

/// Validate and normalize an author label: trimmed, non-empty, at most 64 characters.
/// `None` defaults to `agent`.
fn validate_author(author: Option<&str>) -> Result<&str, String> {
    match author {
        Some(a) => {
            let a = a.trim();
            if a.is_empty() {
                return Err("author must not be empty".to_string());
            }
            if a.chars().count() > MAX_AUTHOR_CHARS {
                return Err(format!(
                    "author must be at most {MAX_AUTHOR_CHARS} characters"
                ));
            }
            Ok(a)
        }
        None => Ok("agent"),
    }
}

/// Add a comment to a task in scope.
///
/// The comment is attributed to `author` (default `agent`). The author `user` marks the
/// comment as user feedback, which agents see in `tk prime` / `tk show` until they reply.
/// The author is trimmed, must be non-empty and at most 64 characters. It is a label, not
/// authentication (see `docs/user-feedback.md`).
pub fn run(
    db_path: &Path,
    id: &str,
    body: &str,
    author: Option<&str>,
    scope: &Scope,
    json: bool,
) -> Result<(), String> {
    let author = validate_author(author)?;
    let db = Database::open(db_path)?;
    scope::ensure_id_in_scope(&db, id, scope)?;
    let comment = db.add_comment_by(id, body, Some(author))?;

    if json {
        let j = serde_json::to_string_pretty(&comment).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
    } else {
        println!("Added comment to {id}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_author_defaults_to_agent() {
        assert_eq!(validate_author(None).unwrap(), "agent");
    }

    #[test]
    fn test_author_is_trimmed() {
        assert_eq!(validate_author(Some("  user \n")).unwrap(), "user");
    }

    #[test]
    fn test_author_empty_is_error() {
        assert!(validate_author(Some("")).is_err());
        assert!(validate_author(Some("   ")).is_err());
    }

    #[test]
    fn test_author_length_limit() {
        assert!(validate_author(Some(&"a".repeat(64))).is_ok());
        assert!(validate_author(Some(&"a".repeat(65))).is_err());
        // whitespace padding does not count toward the limit
        assert!(validate_author(Some(&format!(" {} ", "a".repeat(64)))).is_ok());
    }
}
