use std::fmt;

/// A single candidate for prefix match display
#[derive(Debug, Clone)]
pub struct CandidateEntry {
    pub keyword: String,
    pub expansion: String,
}

/// Expansion result
#[derive(Debug)]
pub enum ExpandOutput {
    /// Expansion succeeded
    Success {
        /// New buffer contents
        buffer: String,
        /// New cursor position
        cursor: usize,
    },
    /// No match
    NoMatch,
    /// Command evaluation required
    Evaluate {
        command: String,
        prefix: String,
        rbuffer: String,
    },
    /// Shell function call required
    Function {
        function_name: String,
        matched_token: String,
        prefix: String,
        rbuffer: String,
    },
    /// Cache is stale
    StaleCache,
    /// Multiple prefix-match candidates found
    Candidates {
        candidates: Vec<CandidateEntry>,
    },
}

impl fmt::Display for ExpandOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpandOutput::Success { buffer, cursor } => {
                write!(f, "success\n{}\n{}", buffer, cursor)
            }
            ExpandOutput::NoMatch => {
                write!(f, "no_match")
            }
            ExpandOutput::Evaluate {
                command,
                prefix,
                rbuffer,
            } => {
                write!(f, "evaluate\n{}\n{}\n{}", command, prefix, rbuffer)
            }
            ExpandOutput::Function {
                function_name,
                matched_token,
                prefix,
                rbuffer,
            } => {
                write!(
                    f,
                    "function\n{}\n{}\n{}\n{}",
                    function_name, matched_token, prefix, rbuffer
                )
            }
            ExpandOutput::StaleCache => {
                write!(f, "stale_cache")
            }
            ExpandOutput::Candidates { candidates } => {
                writeln!(f, "candidates")?;
                writeln!(f, "{}", candidates.len())?;
                for (i, c) in candidates.iter().enumerate() {
                    let escaped = c
                        .expansion
                        .replace('\n', "\\n")
                        .replace('\t', "\\t");
                    if i < candidates.len() - 1 {
                        writeln!(f, "{}\t{}", c.keyword, escaped)?;
                    } else {
                        write!(f, "{}\t{}", c.keyword, escaped)?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Placeholder jump result
#[derive(Debug)]
pub enum PlaceholderOutput {
    /// Jump succeeded
    Success {
        buffer: String,
        cursor: usize,
    },
    /// No placeholder found
    NoPlaceholder,
}

impl fmt::Display for PlaceholderOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceholderOutput::Success { buffer, cursor } => {
                writeln!(f, "success")?;
                writeln!(f, "{}", buffer)?;
                write!(f, "{}", cursor)
            }
            PlaceholderOutput::NoPlaceholder => {
                write!(f, "no_placeholder")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_output_success_display() {
        let output = ExpandOutput::Success {
            buffer: "git commit".to_string(),
            cursor: 10,
        };
        let formatted = output.to_string();
        assert_eq!(formatted, "success\ngit commit\n10");
    }

    #[test]
    fn test_expand_output_no_match_display() {
        let output = ExpandOutput::NoMatch;
        assert_eq!(output.to_string(), "no_match");
    }

    #[test]
    fn test_expand_output_evaluate_display() {
        let output = ExpandOutput::Evaluate {
            command: "date +%Y-%m-%d".to_string(),
            prefix: "echo ".to_string(),
            rbuffer: "".to_string(),
        };
        let formatted = output.to_string();
        assert_eq!(formatted, "evaluate\ndate +%Y-%m-%d\necho \n");
    }

    #[test]
    fn test_expand_output_function_display() {
        let output = ExpandOutput::Function {
            function_name: "my_func".to_string(),
            matched_token: "mf".to_string(),
            prefix: "echo ".to_string(),
            rbuffer: "".to_string(),
        };
        let formatted = output.to_string();
        assert_eq!(formatted, "function\nmy_func\nmf\necho \n");
    }

    #[test]
    fn test_expand_output_stale_cache_display() {
        let output = ExpandOutput::StaleCache;
        assert_eq!(output.to_string(), "stale_cache");
    }

    #[test]
    fn test_placeholder_output_success_display() {
        let output = PlaceholderOutput::Success {
            buffer: "git commit -m ''".to_string(),
            cursor: 15,
        };
        let formatted = output.to_string();
        assert_eq!(formatted, "success\ngit commit -m ''\n15");
    }

    #[test]
    fn test_placeholder_output_no_placeholder_display() {
        let output = PlaceholderOutput::NoPlaceholder;
        assert_eq!(output.to_string(), "no_placeholder");
    }

    #[test]
    fn test_expand_output_candidates_display() {
        let output = ExpandOutput::Candidates {
            candidates: vec![
                CandidateEntry {
                    keyword: "gc".to_string(),
                    expansion: "git commit -m '{{message}}'".to_string(),
                },
                CandidateEntry {
                    keyword: "gp".to_string(),
                    expansion: "git push".to_string(),
                },
                CandidateEntry {
                    keyword: "gd".to_string(),
                    expansion: "git diff".to_string(),
                },
            ],
        };
        let formatted = output.to_string();
        assert_eq!(
            formatted,
            "candidates\n3\ngc\tgit commit -m '{{message}}'\ngp\tgit push\ngd\tgit diff"
        );
    }

    #[test]
    fn test_expand_output_candidates_escape_newline_tab() {
        let output = ExpandOutput::Candidates {
            candidates: vec![
                CandidateEntry {
                    keyword: "a".to_string(),
                    expansion: "line1\nline2".to_string(),
                },
                CandidateEntry {
                    keyword: "b".to_string(),
                    expansion: "col1\tcol2".to_string(),
                },
            ],
        };
        let formatted = output.to_string();
        assert_eq!(
            formatted,
            "candidates\n2\na\tline1\\nline2\nb\tcol1\\tcol2"
        );
    }
}
