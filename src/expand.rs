use crate::context;
use crate::matcher::{self, CompiledAbbr, ExpandAction, Matcher};
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

/// Extract the last command segment from lbuffer
/// Splits on pipe, semicolon, &&, || (outside quotes) and returns the last segment
fn last_command_segment(lbuffer: &str) -> &str {
    let mut last_start = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let bytes = lbuffer.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'|' if !in_single_quote && !in_double_quote => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    // || operator
                    last_start = i + 2;
                    i += 1;
                } else {
                    // pipe
                    last_start = i + 1;
                }
            }
            b';' if !in_single_quote && !in_double_quote => {
                last_start = i + 1;
            }
            b'&' if !in_single_quote && !in_double_quote => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                    last_start = i + 2;
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    &lbuffer[last_start..]
}

/// Extract the first command name from a segment
fn extract_command(segment: &str) -> Option<&str> {
    segment.trim().split_whitespace().next()
}

/// Check if the keyword is at command position or after a known prefix
fn is_command_or_prefix_position(segment: &str, keyword: &str, prefixes: &[String]) -> bool {
    let trimmed = segment.trim();
    // Segment is exactly the keyword → command position
    if trimmed == keyword {
        return true;
    }
    // Check if keyword follows a known prefix
    prefixes.iter().any(|p| {
        if let Some(after) = trimmed.strip_prefix(p.as_str()) {
            after.trim() == keyword
        } else {
            false
        }
    })
}

/// Perform expansion
pub fn expand(input: &ExpandInput, matcher_data: &Matcher, prefixes: &[String]) -> ExpandOutput {
    let Some((prefix, keyword)) = extract_keyword(&input.lbuffer) else {
        return ExpandOutput::NoMatch;
    };

    // 1. Search contextual abbreviations (highest priority, skip if none registered)
    if !matcher_data.contextual.is_empty() {
        if let Some(abbr) =
            context::find_contextual_match(&matcher_data.contextual, keyword, prefix, &input.rbuffer)
        {
            return build_output(prefix, abbr, keyword, &input.rbuffer);
        }
    }

    // Fast path: if no command-scoped, no prefixes, and no regex abbreviations,
    // use simple command position check (avoids last_command_segment parsing)
    let has_advanced_features = !matcher_data.command_scoped.is_empty()
        || !prefixes.is_empty()
        || !matcher_data.regex_abbrs.is_empty();

    if has_advanced_features {
        // 2. Command-scoped: extract command from last segment
        let segment = last_command_segment(&input.lbuffer);
        if !matcher_data.command_scoped.is_empty() {
            if let Some(cmd) = extract_command(segment) {
                if let Some(abbr) = matcher::lookup_command_scoped(matcher_data, cmd, keyword) {
                    return build_output(prefix, abbr, keyword, &input.rbuffer);
                }
            }
        }

        // 3. If in command position (or after prefix), search regular abbreviations
        if is_command_or_prefix_position(segment, keyword, prefixes) {
            if let Some(abbr) = matcher::lookup_regular(matcher_data, keyword) {
                return build_output(prefix, abbr, keyword, &input.rbuffer);
            }
        }
    } else {
        // Fast path: simple command position check (no segment parsing needed)
        if prefix.trim().is_empty() {
            if let Some(abbr) = matcher::lookup_regular(matcher_data, keyword) {
                return build_output(prefix, abbr, keyword, &input.rbuffer);
            }
        }
    }

    // 4. Search global abbreviations (regardless of position)
    if let Some(abbr) = matcher::lookup_global(matcher_data, keyword) {
        return build_output(prefix, abbr, keyword, &input.rbuffer);
    }

    // 5. Regex-keyword abbreviations (linear scan, only when not matched above)
    for abbr in &matcher_data.regex_abbrs {
        if let Ok(re) = regex::Regex::new(&abbr.keyword) {
            if re.is_match(keyword) {
                return build_output(prefix, abbr, keyword, &input.rbuffer);
            }
        }
    }

    ExpandOutput::NoMatch
}

fn build_output(prefix: &str, abbr: &CompiledAbbr, matched_keyword: &str, rbuffer: &str) -> ExpandOutput {
    match &abbr.action {
        ExpandAction::Evaluate => ExpandOutput::Evaluate {
            command: abbr.expansion.clone(),
            prefix: prefix.to_string(),
            rbuffer: rbuffer.to_string(),
        },
        ExpandAction::Function => ExpandOutput::Function {
            function_name: abbr.expansion.clone(),
            matched_token: matched_keyword.to_string(),
            prefix: prefix.to_string(),
            rbuffer: rbuffer.to_string(),
        },
        ExpandAction::Replace => {
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
    }
}

/// Check if the buffer starts with the expansion followed by a word boundary (space or end)
fn starts_with_at_boundary(buffer: &str, expansion: &str) -> bool {
    if !buffer.starts_with(expansion) {
        return false;
    }
    // After the expansion, must be end-of-string or whitespace
    buffer[expansion.len()..]
        .chars()
        .next()
        .map_or(true, |c| c.is_whitespace())
}

/// Check if the buffer contains a command that could have been abbreviated
/// Returns the first reminder found (keyword that could have been used)
pub fn check_remind(buffer: &str, matcher_data: &Matcher) -> Option<(String, String)> {
    // Extract the first word of the buffer (the command)
    let trimmed = buffer.trim();
    let first_word = trimmed.split_whitespace().next()?;

    // Check if this command matches any expansion's first word
    if let Some(keywords) = matcher_data.remind_index.get(first_word) {
        // Check if the full expansion matches at a word boundary
        for keyword in keywords {
            // Look up the abbreviation to get its full expansion
            if let Some(abbr) = matcher::lookup_regular(matcher_data, keyword) {
                if starts_with_at_boundary(trimmed, &abbr.expansion) {
                    return Some((keyword.clone(), abbr.expansion.clone()));
                }
            }
            if let Some(abbr) = matcher::lookup_global(matcher_data, keyword) {
                if starts_with_at_boundary(trimmed, &abbr.expansion) {
                    return Some((keyword.clone(), abbr.expansion.clone()));
                }
            }
        }
    }

    None
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
                ..Default::default()
            },
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit -m '{{message}}'".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "NE".to_string(),
                expansion: "2>/dev/null".to_string(),
                global: true,
                ..Default::default()
            },
            Abbreviation {
                keyword: "main".to_string(),
                expansion: "main --branch".to_string(),
                context: Some(AbbreviationContext {
                    lbuffer: Some("^git (checkout|switch) ".to_string()),
                    rbuffer: None,
                }),
                ..Default::default()
            },
            Abbreviation {
                keyword: "TODAY".to_string(),
                expansion: "date +%Y-%m-%d".to_string(),
                global: true,
                evaluate: true,
                ..Default::default()
            },
        ];
        matcher::build(&abbrs)
    }

    fn no_prefixes() -> Vec<String> {
        vec![]
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
    fn test_last_command_segment_simple() {
        assert_eq!(last_command_segment("echo hello").trim(), "echo hello");
    }

    #[test]
    fn test_last_command_segment_pipe() {
        assert_eq!(last_command_segment("echo hello | grep co").trim(), "grep co");
    }

    #[test]
    fn test_last_command_segment_semicolon() {
        assert_eq!(last_command_segment("cd /tmp; ls").trim(), "ls");
    }

    #[test]
    fn test_last_command_segment_and() {
        assert_eq!(last_command_segment("make && make install").trim(), "make install");
    }

    #[test]
    fn test_last_command_segment_or() {
        assert_eq!(last_command_segment("test -f foo || echo no").trim(), "echo no");
    }

    #[test]
    fn test_last_command_segment_quoted() {
        // Pipe inside quotes should not split
        assert_eq!(
            last_command_segment("echo \"hello | world\"").trim(),
            "echo \"hello | world\""
        );
    }

    #[test]
    fn test_is_command_or_prefix_position() {
        let prefixes = vec!["sudo".to_string(), "doas".to_string()];
        assert!(is_command_or_prefix_position("g", "g", &prefixes));
        assert!(is_command_or_prefix_position("sudo g", "g", &prefixes));
        assert!(is_command_or_prefix_position("doas g", "g", &prefixes));
        assert!(!is_command_or_prefix_position("echo g", "g", &prefixes));
    }

    #[test]
    fn test_expand_regular_command_position() {
        let matcher = build_test_matcher();
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
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
        match expand(&input, &matcher, &no_prefixes()) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "git --help");
                assert_eq!(cursor, 3);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_command_scoped() {
        let abbrs = vec![
            Abbreviation {
                keyword: "co".to_string(),
                expansion: "checkout".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let input = ExpandInput {
            lbuffer: "git co".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher, &no_prefixes()) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "git checkout");
                assert_eq!(cursor, 12);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_command_scoped_wrong_command() {
        let abbrs = vec![
            Abbreviation {
                keyword: "co".to_string(),
                expansion: "checkout".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let input = ExpandInput {
            lbuffer: "npm co".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher, &no_prefixes()) {
            ExpandOutput::NoMatch => {}
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_command_scoped_after_pipe() {
        let abbrs = vec![
            Abbreviation {
                keyword: "co".to_string(),
                expansion: "checkout".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let input = ExpandInput {
            lbuffer: "echo hello | git co".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher, &no_prefixes()) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "echo hello | git checkout");
                assert_eq!(cursor, 25);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_with_prefix_system() {
        let abbrs = vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let prefixes = vec!["sudo".to_string()];
        let input = ExpandInput {
            lbuffer: "sudo g".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher, &prefixes) {
            ExpandOutput::Success { buffer, cursor } => {
                assert_eq!(buffer, "sudo git");
                assert_eq!(cursor, 8);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_expand_function() {
        let abbrs = vec![
            Abbreviation {
                keyword: "mf".to_string(),
                expansion: "my_func".to_string(),
                function: true,
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let input = ExpandInput {
            lbuffer: "mf".to_string(),
            rbuffer: "".to_string(),
        };
        match expand(&input, &matcher, &no_prefixes()) {
            ExpandOutput::Function {
                function_name,
                matched_token,
                prefix,
                rbuffer,
            } => {
                assert_eq!(function_name, "my_func");
                assert_eq!(matched_token, "mf");
                assert_eq!(prefix, "");
                assert_eq!(rbuffer, "");
            }
            other => panic!("Expected Function, got {:?}", other),
        }
    }

    #[test]
    fn test_check_remind_exact_match() {
        let matcher = build_test_matcher();
        // "git push" starts with "git" followed by a space → should remind about "g"
        let result = check_remind("git push", &matcher);
        assert!(result.is_some());
        let (keyword, _) = result.unwrap();
        assert_eq!(keyword, "g");
    }

    #[test]
    fn test_check_remind_no_false_positive_on_prefix() {
        let matcher = build_test_matcher();
        // "gitlab" starts with "git" but NOT at a word boundary → should NOT remind
        let result = check_remind("gitlab push", &matcher);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_remind_exact_command() {
        let matcher = build_test_matcher();
        // "git" alone (exact match) → should remind
        let result = check_remind("git", &matcher);
        assert!(result.is_some());
    }

    #[test]
    fn test_starts_with_at_boundary() {
        assert!(starts_with_at_boundary("git push", "git"));
        assert!(starts_with_at_boundary("git", "git"));
        assert!(!starts_with_at_boundary("gitlab", "git"));
        assert!(!starts_with_at_boundary("gitk", "git"));
        assert!(starts_with_at_boundary("git\tpush", "git"));
    }
}
