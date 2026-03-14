use crate::context::{self, RegexCache};
use crate::matcher::{self, CompiledAbbr, ExpandAction, Matcher};
use crate::output::{CandidateEntry, ExpandOutput};
use crate::placeholder;
use std::fmt;

/// Expansion input
pub struct ExpandInput {
    pub lbuffer: String,
    pub rbuffer: String,
}

/// Result of an expansion attempt, including matched abbreviation info for history recording
pub struct ExpandResult {
    /// The expansion output (for protocol display)
    pub output: ExpandOutput,
    /// The raw expansion text from the matched abbreviation (for history recording).
    /// None for NoMatch, Candidates, StaleCache.
    pub matched_expansion: Option<String>,
}

impl fmt::Display for ExpandResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.output.fmt(f)
    }
}

/// Extract keyword from lbuffer
/// Returns the trailing token (last word delimited by space) of lbuffer as the keyword
pub(crate) fn extract_keyword(lbuffer: &str) -> Option<(&str, &str)> {
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
    // Check if keyword follows a known prefix (with word boundary enforcement)
    prefixes.iter().any(|p| {
        if let Some(after) = trimmed.strip_prefix(p.as_str()) {
            // Ensure prefix is followed by whitespace (word boundary)
            after.starts_with(char::is_whitespace) && after.trim() == keyword
        } else {
            false
        }
    })
}

/// Perform expansion
pub fn expand(
    input: &ExpandInput,
    matcher_data: &Matcher,
    prefixes: &[String],
    regex_cache: &RegexCache,
) -> ExpandResult {
    let Some((prefix, keyword)) = extract_keyword(&input.lbuffer) else {
        return ExpandResult { output: ExpandOutput::NoMatch, matched_expansion: None };
    };

    // 1. Search contextual abbreviations (highest priority, skip if none registered)
    if !matcher_data.contextual.is_empty() {
        if let Some(abbr) = context::find_contextual_match(
            &matcher_data.contextual,
            keyword,
            prefix,
            &input.rbuffer,
            regex_cache,
        ) {
            return matched_result(prefix, abbr, keyword, &input.rbuffer);
        }
    }

    // Compute command position and current command for both exact and prefix matching
    let has_advanced_features = !matcher_data.command_scoped.is_empty()
        || !prefixes.is_empty()
        || !matcher_data.regex_abbrs.is_empty();

    let (in_command_position, current_command) = if has_advanced_features {
        let segment = last_command_segment(&input.lbuffer);
        let cmd = extract_command(segment).map(|s| s.to_string());
        let is_cmd_pos = is_command_or_prefix_position(segment, keyword, prefixes);

        // 2. Command-scoped: extract command from last segment
        if !matcher_data.command_scoped.is_empty() {
            if let Some(ref cmd_name) = cmd {
                if let Some(abbr) = matcher::lookup_command_scoped(matcher_data, cmd_name, keyword)
                {
                    return matched_result(prefix, abbr, keyword, &input.rbuffer);
                }
            }
        }

        // 3. If in command position (or after prefix), search regular abbreviations
        if is_cmd_pos {
            if let Some(abbr) = matcher::lookup_regular(matcher_data, keyword) {
                return matched_result(prefix, abbr, keyword, &input.rbuffer);
            }
        }

        (is_cmd_pos, cmd)
    } else {
        // Fast path: simple command position check (no segment parsing needed)
        let is_cmd_pos = prefix.trim().is_empty();
        if is_cmd_pos {
            if let Some(abbr) = matcher::lookup_regular(matcher_data, keyword) {
                return matched_result(prefix, abbr, keyword, &input.rbuffer);
            }
        }
        (is_cmd_pos, None)
    };

    // 4. Search global abbreviations (regardless of position)
    if let Some(abbr) = matcher::lookup_global(matcher_data, keyword) {
        return matched_result(prefix, abbr, keyword, &input.rbuffer);
    }

    // 5. Regex-keyword abbreviations (lazy-compiled regex lookup, linear scan)
    for abbr in &matcher_data.regex_abbrs {
        if regex_cache.is_match(&abbr.keyword, keyword) == Some(true) {
            return matched_result(prefix, abbr, keyword, &input.rbuffer);
        }
    }

    // 6. Prefix match fallback (O(1) lookup via prefix_index)
    let candidates = matcher::prefix_candidates(
        matcher_data,
        keyword,
        in_command_position,
        current_command.as_deref(),
    );
    if candidates.len() >= 1 {
        return ExpandResult {
            output: ExpandOutput::Candidates {
                candidates: candidates
                    .iter()
                    .map(|abbr| CandidateEntry {
                        keyword: abbr.keyword.clone(),
                        expansion: abbr.expansion.clone(),
                    })
                    .collect(),
            },
            matched_expansion: None,
        };
    }

    ExpandResult { output: ExpandOutput::NoMatch, matched_expansion: None }
}

/// Build an ExpandResult for a matched abbreviation, capturing expansion info for history
fn matched_result(prefix: &str, abbr: &CompiledAbbr, keyword: &str, rbuffer: &str) -> ExpandResult {
    ExpandResult {
        matched_expansion: Some(abbr.expansion.clone()),
        output: build_output(prefix, abbr, keyword, rbuffer),
    }
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

            // Build full buffer in a single allocation
            let new_lbuffer_len = prefix.len() + expansion.len();
            let mut buffer = String::with_capacity(new_lbuffer_len + rbuffer.len());
            buffer.push_str(prefix);
            buffer.push_str(expansion);
            buffer.push_str(rbuffer);

            let placeholder_result =
                placeholder::apply_first_placeholder(buffer, new_lbuffer_len);

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
    use insta::assert_snapshot;

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

    fn cache_for(_matcher: &Matcher) -> RegexCache {
        RegexCache::new()
    }

    #[test]
    fn test_extract_keyword_simple() {
        let (prefix, keyword) = extract_keyword("g").unwrap();
        assert_eq!(prefix, "");
        assert_eq!(keyword, "g");
    }

    #[test]
    fn test_extract_keyword_with_trailing_space() {
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
    fn test_prefix_word_boundary() {
        let prefixes = vec!["sudo".to_string()];
        // "sudoku g" should NOT match prefix "sudo" (no word boundary)
        assert!(!is_command_or_prefix_position("sudoku g", "g", &prefixes));
        // "sudo g" should match
        assert!(is_command_or_prefix_position("sudo g", "g", &prefixes));
    }

    #[test]
    fn test_expand_regular_command_position() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_expand_regular_not_command_position() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "echo g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @"no_match");
    }

    #[test]
    fn test_expand_global() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "echo hello NE".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        echo hello 2>/dev/null
        22
        ");
    }

    #[test]
    fn test_expand_with_placeholder() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "gc".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git commit -m ''
        15
        ");
    }

    #[test]
    fn test_expand_contextual() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "git checkout main".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git checkout main --branch
        26
        ");
    }

    #[test]
    fn test_expand_contextual_no_match() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "git commit main".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @"no_match");
    }

    #[test]
    fn test_expand_evaluate() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "echo TODAY".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        evaluate
        date +%Y-%m-%d
        echo

        ");
    }

    #[test]
    fn test_expand_no_match() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "unknown_command".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @"no_match");
    }

    #[test]
    fn test_expand_empty_input() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @"no_match");
    }

    #[test]
    fn test_expand_with_rbuffer() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: " --help".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git --help
        3
        ");
    }

    #[test]
    fn test_expand_command_scoped() {
        let abbrs = vec![Abbreviation {
            keyword: "co".to_string(),
            expansion: "checkout".to_string(),
            command: Some("git".to_string()),
            ..Default::default()
        }];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "git co".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git checkout
        12
        ");
    }

    #[test]
    fn test_expand_command_scoped_wrong_command() {
        let abbrs = vec![Abbreviation {
            keyword: "co".to_string(),
            expansion: "checkout".to_string(),
            command: Some("git".to_string()),
            ..Default::default()
        }];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "npm co".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @"no_match");
    }

    #[test]
    fn test_expand_command_scoped_after_pipe() {
        let abbrs = vec![Abbreviation {
            keyword: "co".to_string(),
            expansion: "checkout".to_string(),
            command: Some("git".to_string()),
            ..Default::default()
        }];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "echo hello | git co".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        echo hello | git checkout
        25
        ");
    }

    #[test]
    fn test_expand_with_prefix_system() {
        let abbrs = vec![Abbreviation {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            ..Default::default()
        }];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let prefixes = vec!["sudo".to_string()];
        let input = ExpandInput {
            lbuffer: "sudo g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &prefixes, &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        sudo git
        8
        ");
    }

    #[test]
    fn test_expand_function() {
        let abbrs = vec![Abbreviation {
            keyword: "mf".to_string(),
            expansion: "my_func".to_string(),
            function: true,
            ..Default::default()
        }];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "mf".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        function
        my_func
        mf


        ");
    }

    // === Prefix match fallback tests ===

    #[test]
    fn test_expand_prefix_exact_match_takes_priority() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_expand_prefix_no_candidates() {
        let matcher = build_test_matcher();
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "xyz".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @"no_match");
    }

    #[test]
    fn test_expand_prefix_single_candidate_returns_candidates() {
        let abbrs = vec![Abbreviation {
            keyword: "gc".to_string(),
            expansion: "git commit".to_string(),
            ..Default::default()
        }];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        candidates
        1
        gc	git commit
        ");
    }

    #[test]
    fn test_expand_prefix_multiple_candidates() {
        let abbrs = vec![
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        candidates
        2
        gc	git commit
        gp	git push
        ");
    }

    #[test]
    fn test_expand_prefix_exact_match_with_prefix_candidates() {
        let abbrs = vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_expand_prefix_candidates_not_command_position_global_only() {
        // Non-command position: only global abbrs should appear as candidates
        let abbrs = vec![
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gx".to_string(),
                expansion: "global expand".to_string(),
                global: true,
                ..Default::default()
            },
            Abbreviation {
                keyword: "gy".to_string(),
                expansion: "global yank".to_string(),
                global: true,
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        // "echo g" → not command position
        let input = ExpandInput {
            lbuffer: "echo g".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        candidates
        2
        gx	global expand
        gy	global yank
        ");
    }

    #[test]
    fn test_expand_prefix_candidates_command_scoped() {
        // "git c" → command-scoped co, cm should appear as candidates
        let abbrs = vec![
            Abbreviation {
                keyword: "co".to_string(),
                expansion: "checkout".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
            Abbreviation {
                keyword: "cm".to_string(),
                expansion: "commit".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);
        let rc = cache_for(&matcher);
        let input = ExpandInput {
            lbuffer: "git c".to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand(&input, &matcher, &no_prefixes(), &rc);
        assert_snapshot!(result.to_string(), @r"
        candidates
        2
        cm	commit
        co	checkout
        ");
    }

    #[test]
    fn test_check_remind_exact_match() {
        let matcher = build_test_matcher();
        let result = check_remind("git push", &matcher);
        assert!(result.is_some());
        let (keyword, _) = result.unwrap();
        assert_eq!(keyword, "g");
    }

    #[test]
    fn test_check_remind_no_false_positive_on_prefix() {
        let matcher = build_test_matcher();
        let result = check_remind("gitlab push", &matcher);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_remind_exact_command() {
        let matcher = build_test_matcher();
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
