use crate::context;
use crate::matcher::{self, CompiledAbbr, Matcher};
use crate::output::ExpandOutput;
use crate::placeholder;

/// Expansion input
pub struct ExpandInput {
    pub lbuffer: String,
    pub rbuffer: String,
}

/// Extract keyword from lbuffer
/// Returns the trailing token (last word delimited by space) of lbuffer as the keyword
fn extract_keyword(lbuffer: &str) -> Option<(&str, &str)> {
    let trimmed = lbuffer.trim_end();
    if trimmed.is_empty() {
        return None;
    }

    // Get the trailing token
    if let Some(space_pos) = trimmed.rfind(' ') {
        let keyword = &trimmed[space_pos + 1..];
        let prefix = &trimmed[..space_pos + 1];
        if keyword.is_empty() {
            None
        } else {
            Some((prefix, keyword))
        }
    } else {
        // No space = entire lbuffer is the keyword
        Some(("", trimmed))
    }
}

/// Determine if position is command position
/// If lbuffer contains no spaces, it is a command position
fn is_command_position(prefix: &str) -> bool {
    prefix.trim().is_empty()
}

/// Perform expansion
pub fn expand(input: &ExpandInput, matcher_data: &Matcher) -> ExpandOutput {
    let Some((prefix, keyword)) = extract_keyword(&input.lbuffer) else {
        return ExpandOutput::NoMatch;
    };

    // 1. Search contextual abbreviations with highest priority
    // Use the part of lbuffer excluding the keyword as context
    if let Some(abbr) =
        context::find_contextual_match(&matcher_data.contextual, keyword, prefix, &input.rbuffer)
    {
        return build_output(prefix, abbr, &input.rbuffer);
    }

    // 2. If in command position, search regular abbreviations
    if is_command_position(prefix) {
        if let Some(abbr) = matcher::lookup_regular(matcher_data, keyword) {
            return build_output(prefix, abbr, &input.rbuffer);
        }
    }

    // 3. Search global abbreviations (regardless of position)
    if let Some(abbr) = matcher::lookup_global(matcher_data, keyword) {
        return build_output(prefix, abbr, &input.rbuffer);
    }

    ExpandOutput::NoMatch
}

fn build_output(prefix: &str, abbr: &CompiledAbbr, rbuffer: &str) -> ExpandOutput {
    if abbr.evaluate {
        return ExpandOutput::Evaluate {
            command: abbr.expansion.clone(),
            prefix: prefix.to_string(),
            rbuffer: rbuffer.to_string(),
        };
    }

    let expansion = &abbr.expansion;

    // Placeholder processing
    let new_lbuffer = format!("{}{}", prefix, expansion);
    let full_buffer = format!("{}{}", new_lbuffer, rbuffer);

    let placeholder_result =
        placeholder::apply_first_placeholder(&full_buffer, new_lbuffer.len());

    ExpandOutput::Success {
        buffer: placeholder_result.text,
        cursor: placeholder_result.cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Abbreviation, AbbreviationContext};

    fn build_test_matcher() -> Matcher {
        let abbrs = vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                global: false,
                evaluate: false,
                allow_conflict: false,
                context: None,
            },
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit -m '{{message}}'".to_string(),
                global: false,
                evaluate: false,
                allow_conflict: false,
                context: None,
            },
            Abbreviation {
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
                global: false,
                evaluate: false,
                allow_conflict: false,
                context: None,
            },
            Abbreviation {
                keyword: "NE".to_string(),
                expansion: "2>/dev/null".to_string(),
                global: true,
                evaluate: false,
                allow_conflict: false,
                context: None,
            },
            Abbreviation {
                keyword: "main".to_string(),
                expansion: "main --branch".to_string(),
                global: false,
                evaluate: false,
                allow_conflict: false,
                context: Some(AbbreviationContext {
                    lbuffer: Some("^git (checkout|switch) ".to_string()),
                    rbuffer: None,
                }),
            },
            Abbreviation {
                keyword: "TODAY".to_string(),
                expansion: "date +%Y-%m-%d".to_string(),
                global: true,
                evaluate: true,
                allow_conflict: false,
                context: None,
            },
        ];
        matcher::build(&abbrs)
    }

    #[test]
    fn test_extract_keyword_simple() {
        let (prefix, keyword) = extract_keyword("g").unwrap();
        assert_eq!(prefix, "");
        assert_eq!(keyword, "g");
    }

    #[test]
    fn test_extract_keyword_with_trailing_space() {
        // When trailing is only spaces, trim_end removes them and returns the last token
        let (prefix, keyword) = extract_keyword("git commit ").unwrap();
        assert_eq!(prefix, "git ");
        assert_eq!(keyword, "commit");
    }

    #[test]
    fn test_extract_keyword_with_args() {
        let (prefix, keyword) = extract_keyword("echo NE").unwrap();
        assert_eq!(prefix, "echo ");
        assert_eq!(keyword, "NE");
    }

    #[test]
    fn test_extract_keyword_empty() {
        assert!(extract_keyword("").is_none());
        assert!(extract_keyword("   ").is_none());
    }

    #[test]
    fn test_is_command_position() {
        assert!(is_command_position(""));
        assert!(is_command_position("  "));
        assert!(!is_command_position("echo "));
        assert!(!is_command_position("git commit "));
    }

    #[test]
    fn test_expand_regular_command_position() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "git");
                assert_eq!(cursor, 3);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_regular_not_command_position() {
        let matcher = build_test_matcher();
        // "g" is regular, so it only matches in command position
        let input = ExpandInput {
            lbuffer: "echo g".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::NoMatch => {}
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_global() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "echo hello NE".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "echo hello 2>/dev/null");
                assert_eq!(cursor, 22);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_with_placeholder() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "gc".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "git commit -m ''");
                assert_eq!(cursor, 15);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_contextual() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "git checkout main".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "git checkout main --branch");
                assert_eq!(cursor, 26);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_contextual_no_match() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "git commit main".to_string(),
            rbuffer: "".to_string(),
        };
        // "main" has context, but lbuffer is "git commit " so it doesn't match
        // "main" is also not in regular, so no match
        match expand(&input, &matcher) {
            ExpandOutput::NoMatch => {}
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_evaluate() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "echo TODAY".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::Evaluate {
                command,
                prefix,
                rbuffer,
            } => {
                assert_eq!(command, "date +%Y-%m-%d");
                assert_eq!(prefix, "echo ");
                assert_eq!(rbuffer, "");
            }
            other => panic!("Expected Evaluate, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_no_match() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "unknown_command".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::NoMatch => {}
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_empty_input() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::NoMatch => {}
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_with_rbuffer() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: " --help".to_string(),
        };
        match expand(&input, &matcher) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "git --help");
                assert_eq!(cursor, 3);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }
}
