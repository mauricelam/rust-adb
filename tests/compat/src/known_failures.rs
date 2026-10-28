//! Parser for `rust-client-known-failures.txt`.

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct KnownFailureEntry {
    pub test_name: String,
    pub reason: String,
}

/// Parses the contents of `rust-client-known-failures.txt`.
///
/// Ignores blank lines and `#` comment lines. Captures `#` comments directly preceding
/// a test line as the reason comment for that entry.
pub fn parse_known_failures(content: &str) -> Vec<KnownFailureEntry> {
    let mut entries = Vec::new();
    let mut current_reason = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            current_reason.clear();
        } else if let Some(comment) = trimmed.strip_prefix('#') {
            let comment_text = comment.trim();
            if !comment_text.is_empty() {
                current_reason.push(comment_text.to_string());
            }
        } else {
            let reason = if current_reason.is_empty() {
                "No reason provided".to_string()
            } else {
                current_reason.join("; ")
            };
            entries.push(KnownFailureEntry {
                test_name: trimmed.to_string(),
                reason,
            });
            current_reason.clear();
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_known_failures_basic() {
        let fixture = r#"
# Issue 41
test_help

# Issue 42: multi-word comment
# Second comment line
test_version
"#;

        let parsed = parse_known_failures(fixture);
        assert_eq!(
            parsed,
            vec![
                KnownFailureEntry {
                    test_name: "test_help".to_string(),
                    reason: "Issue 41".to_string(),
                },
                KnownFailureEntry {
                    test_name: "test_version".to_string(),
                    reason: "Issue 42: multi-word comment; Second comment line".to_string(),
                },
            ]
        );
    }

    #[test]
    fn test_parse_known_failures_empty_and_no_comments() {
        let fixture = r#"
# General file comment that gets cleared by empty line

test_no_comment

# Issue 43
test_with_comment
"#;

        let parsed = parse_known_failures(fixture);
        assert_eq!(
            parsed,
            vec![
                KnownFailureEntry {
                    test_name: "test_no_comment".to_string(),
                    reason: "No reason provided".to_string(),
                },
                KnownFailureEntry {
                    test_name: "test_with_comment".to_string(),
                    reason: "Issue 43".to_string(),
                },
            ]
        );
    }
}
