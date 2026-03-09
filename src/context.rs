use crate::matcher::{AbbrScope, CompiledAbbr};
use regex::Regex;
use rustc_hash::FxHashMap;

/// Check context conditions
/// Match against lbuffer/rbuffer regex patterns from AbbrScope::Contextual
pub fn matches_context(abbr: &CompiledAbbr, lbuffer: &str, rbuffer: &str) -> bool {
    match &abbr.scope {
        AbbrScope::Contextual {
            lbuffer: lb_pat,
            rbuffer: rb_pat,
        } => {
            if let Some(ref pattern) = lb_pat {
                match Regex::new(pattern) {
                    Ok(re) => {
                        if !re.is_match(lbuffer) {
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
            if let Some(ref pattern) = rb_pat {
                match Regex::new(pattern) {
                    Ok(re) => {
                        if !re.is_match(rbuffer) {
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
            true
        }
        _ => true,
    }
}

/// Find matching contextual abbreviation from HashMap
pub fn find_contextual_match<'a>(
    contextual: &'a FxHashMap<String, Vec<CompiledAbbr>>,
    keyword: &str,
    lbuffer: &str,
    rbuffer: &str,
) -> Option<&'a CompiledAbbr> {
    contextual
        .get(keyword)?
        .iter()
        .find(|abbr| matches_context(abbr, lbuffer, rbuffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::{AbbrScope, CompiledAbbr};

    fn make_contextual(
        keyword: &str,
        expansion: &str,
        lbuffer: Option<&str>,
        rbuffer: Option<&str>,
    ) -> CompiledAbbr {
        CompiledAbbr {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            scope: AbbrScope::Contextual {
                lbuffer: lbuffer.map(|s| s.to_string()),
                rbuffer: rbuffer.map(|s| s.to_string()),
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_matches_lbuffer_pattern() {
        let abbr = make_contextual("main", "main --branch", Some("^git (checkout|switch)"), None);
        assert!(matches_context(&abbr, "git checkout ", ""));
        assert!(matches_context(&abbr, "git switch ", ""));
        assert!(!matches_context(&abbr, "git commit ", ""));
        assert!(!matches_context(&abbr, "", ""));
    }

    #[test]
    fn test_matches_rbuffer_pattern() {
        let abbr = make_contextual("--force", "--force-with-lease", None, Some("$"));
        assert!(matches_context(&abbr, "git push ", ""));
        assert!(matches_context(&abbr, "", ""));
    }

    #[test]
    fn test_matches_both_patterns() {
        let abbr = make_contextual(
            "main",
            "main --branch",
            Some("^git checkout"),
            Some("$"),
        );
        assert!(matches_context(&abbr, "git checkout ", ""));
        assert!(!matches_context(&abbr, "echo ", ""));
    }

    #[test]
    fn test_no_context_always_matches() {
        let abbr = CompiledAbbr {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            ..Default::default()
        };
        assert!(matches_context(&abbr, "anything", "anything"));
    }

    #[test]
    fn test_find_contextual_match() {
        let mut contextual: FxHashMap<String, Vec<CompiledAbbr>> = FxHashMap::default();
        contextual.entry("main".to_string()).or_default().push(
            make_contextual("main", "main --branch", Some("^git (checkout|switch)"), None),
        );
        contextual.entry("main".to_string()).or_default().push(
            make_contextual("main", "int main()", Some("^#include"), None),
        );

        let result = find_contextual_match(&contextual, "main", "git checkout ", "");
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "main --branch");

        let result = find_contextual_match(&contextual, "main", "#include <stdio.h>\n", "");
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "int main()");

        let result = find_contextual_match(&contextual, "main", "echo ", "");
        assert!(result.is_none());

        let result = find_contextual_match(&contextual, "other", "git checkout ", "");
        assert!(result.is_none());
    }
}
