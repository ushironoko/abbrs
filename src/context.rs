use crate::matcher::CompiledAbbr;
use regex::Regex;

/// Check context conditions
/// Match against lbuffer/rbuffer regex patterns
pub fn matches_context(abbr: &CompiledAbbr, lbuffer: &str, rbuffer: &str) -> bool {
    if let Some(ref pattern) = abbr.lbuffer_pattern {
        match Regex::new(pattern) {
            Ok(re) => {
                if !re.is_match(lbuffer) {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }

    if let Some(ref pattern) = abbr.rbuffer_pattern {
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

/// Find matching contextual abbreviation
pub fn find_contextual_match<'a>(
    contextual: &'a [CompiledAbbr],
    keyword: &str,
    lbuffer: &str,
    rbuffer: &str,
) -> Option<&'a CompiledAbbr> {
    contextual.iter().find(|abbr| {
        abbr.keyword == keyword && matches_context(abbr, lbuffer, rbuffer)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::CompiledAbbr;

    fn make_contextual(
        keyword: &str,
        expansion: &str,
        lbuffer: Option<&str>,
        rbuffer: Option<&str>,
    ) -> CompiledAbbr {
        CompiledAbbr {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            global: false,
            evaluate: false,
            lbuffer_pattern: lbuffer.map(|s| s.to_string()),
            rbuffer_pattern: rbuffer.map(|s| s.to_string()),
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
        let abbr = make_contextual("g", "git", None, None);
        assert!(matches_context(&abbr, "anything", "anything"));
    }

    #[test]
    fn test_find_contextual_match() {
        let contextual = vec![
            make_contextual("main", "main --branch", Some("^git (checkout|switch)"), None),
            make_contextual("main", "int main()", Some("^#include"), None),
        ];

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
