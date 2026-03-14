use crate::matcher::{AbbrScope, CompiledAbbr};
use regex::Regex;
use rustc_hash::FxHashMap;
use std::sync::Mutex;

/// Lazy regex cache that compiles patterns on first use.
/// Only the patterns actually needed for the current expansion are compiled,
/// avoiding the cost of eagerly compiling all patterns on every CLI invocation.
#[derive(Debug)]
pub struct RegexCache {
    cache: Mutex<FxHashMap<String, Regex>>,
}

impl RegexCache {
    /// Create an empty lazy regex cache.
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(FxHashMap::default()),
        }
    }

    /// Check if `pattern` matches `text`. Compiles and caches the regex on first use.
    /// Returns `None` if the regex pattern is invalid.
    pub fn is_match(&self, pattern: &str, text: &str) -> Option<bool> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(re) = cache.get(pattern) {
            return Some(re.is_match(text));
        }

        let re = Regex::new(pattern).ok()?;
        let result = re.is_match(text);
        cache.insert(pattern.to_string(), re);
        Some(result)
    }
}

impl Default for RegexCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Check context conditions using lazily-compiled regexes
/// Match against lbuffer/rbuffer regex patterns from AbbrScope::Contextual
pub fn matches_context(
    abbr: &CompiledAbbr,
    lbuffer: &str,
    rbuffer: &str,
    regex_cache: &RegexCache,
) -> bool {
    match &abbr.scope {
        AbbrScope::Contextual {
            lbuffer: lb_pat,
            rbuffer: rb_pat,
        } => {
            if let Some(ref pattern) = lb_pat {
                if regex_cache.is_match(pattern, lbuffer) != Some(true) {
                    return false;
                }
            }
            if let Some(ref pattern) = rb_pat {
                if regex_cache.is_match(pattern, rbuffer) != Some(true) {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}

/// Find matching contextual abbreviation from HashMap using pre-compiled regexes
pub fn find_contextual_match<'a>(
    contextual: &'a FxHashMap<String, Vec<CompiledAbbr>>,
    keyword: &str,
    lbuffer: &str,
    rbuffer: &str,
    regex_cache: &RegexCache,
) -> Option<&'a CompiledAbbr> {
    contextual
        .get(keyword)?
        .iter()
        .find(|abbr| matches_context(abbr, lbuffer, rbuffer, regex_cache))
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
        let cache = RegexCache::new();
        assert!(matches_context(&abbr, "git checkout ", "", &cache));
        assert!(matches_context(&abbr, "git switch ", "", &cache));
        assert!(!matches_context(&abbr, "git commit ", "", &cache));
        assert!(!matches_context(&abbr, "", "", &cache));
    }

    #[test]
    fn test_matches_rbuffer_pattern() {
        let abbr = make_contextual("--force", "--force-with-lease", None, Some("$"));
        let cache = RegexCache::new();
        assert!(matches_context(&abbr, "git push ", "", &cache));
        assert!(matches_context(&abbr, "", "", &cache));
    }

    #[test]
    fn test_matches_both_patterns() {
        let abbr = make_contextual(
            "main",
            "main --branch",
            Some("^git checkout"),
            Some("$"),
        );
        let cache = RegexCache::new();
        assert!(matches_context(&abbr, "git checkout ", "", &cache));
        assert!(!matches_context(&abbr, "echo ", "", &cache));
    }

    #[test]
    fn test_no_context_always_matches() {
        let abbr = CompiledAbbr {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            ..Default::default()
        };
        let cache = RegexCache::new();
        assert!(matches_context(&abbr, "anything", "anything", &cache));
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

        let cache = RegexCache::new();

        let result = find_contextual_match(&contextual, "main", "git checkout ", "", &cache);
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "main --branch");

        let result = find_contextual_match(&contextual, "main", "#include <stdio.h>\n", "", &cache);
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "int main()");

        let result = find_contextual_match(&contextual, "main", "echo ", "", &cache);
        assert!(result.is_none());

        let result = find_contextual_match(&contextual, "other", "git checkout ", "", &cache);
        assert!(result.is_none());
    }

    #[test]
    fn test_regex_cache_lazy_compilation() {
        let cache = RegexCache::new();
        // Pattern is compiled on first use
        assert_eq!(cache.is_match("^git checkout", "git checkout main"), Some(true));
        assert_eq!(cache.is_match("^git checkout", "echo hello"), Some(false));
        // Invalid pattern returns None
        assert_eq!(cache.is_match("[invalid", "anything"), None);
    }
}
