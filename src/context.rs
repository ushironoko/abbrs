use crate::matcher::{AbbrScope, CompiledAbbr, Matcher};
use regex::Regex;
use rustc_hash::FxHashMap;

/// Pre-compiled regex cache for O(1) pattern lookup at expand time.
/// Built once after cache deserialization to avoid repeated Regex::new() calls.
#[derive(Debug)]
pub struct RegexCache {
    cache: FxHashMap<String, Regex>,
}

impl RegexCache {
    /// Build a RegexCache from all regex patterns in the Matcher.
    /// Compiles contextual lbuffer/rbuffer patterns and regex-keyword patterns.
    pub fn from_matcher(matcher: &Matcher) -> Self {
        let mut cache = FxHashMap::default();
        for abbrs in matcher.contextual.values() {
            for abbr in abbrs {
                if let AbbrScope::Contextual {
                    lbuffer,
                    rbuffer,
                } = &abbr.scope
                {
                    if let Some(pat) = lbuffer {
                        if !cache.contains_key(pat) {
                            if let Ok(re) = Regex::new(pat) {
                                cache.insert(pat.clone(), re);
                            }
                        }
                    }
                    if let Some(pat) = rbuffer {
                        if !cache.contains_key(pat) {
                            if let Ok(re) = Regex::new(pat) {
                                cache.insert(pat.clone(), re);
                            }
                        }
                    }
                }
            }
        }
        for abbr in &matcher.regex_abbrs {
            if !cache.contains_key(&abbr.keyword) {
                if let Ok(re) = Regex::new(&abbr.keyword) {
                    cache.insert(abbr.keyword.clone(), re);
                }
            }
        }
        Self { cache }
    }

    /// Look up a pre-compiled regex by its pattern string.
    pub fn get(&self, pattern: &str) -> Option<&Regex> {
        self.cache.get(pattern)
    }
}

/// Check context conditions using pre-compiled regexes
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
                match regex_cache.get(pattern) {
                    Some(re) => {
                        if !re.is_match(lbuffer) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            if let Some(ref pattern) = rb_pat {
                match regex_cache.get(pattern) {
                    Some(re) => {
                        if !re.is_match(rbuffer) {
                            return false;
                        }
                    }
                    None => return false,
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
    use crate::matcher::{AbbrScope, CompiledAbbr, Matcher};

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

    fn build_regex_cache_from_contextual(
        contextual: &FxHashMap<String, Vec<CompiledAbbr>>,
    ) -> RegexCache {
        let mut matcher = Matcher::new();
        matcher.contextual = contextual.clone();
        RegexCache::from_matcher(&matcher)
    }

    fn regex_cache_for_abbr(abbr: &CompiledAbbr) -> RegexCache {
        let mut contextual: FxHashMap<String, Vec<CompiledAbbr>> = FxHashMap::default();
        contextual
            .entry(abbr.keyword.clone())
            .or_default()
            .push(abbr.clone());
        build_regex_cache_from_contextual(&contextual)
    }

    #[test]
    fn test_matches_lbuffer_pattern() {
        let abbr = make_contextual("main", "main --branch", Some("^git (checkout|switch)"), None);
        let cache = regex_cache_for_abbr(&abbr);
        assert!(matches_context(&abbr, "git checkout ", "", &cache));
        assert!(matches_context(&abbr, "git switch ", "", &cache));
        assert!(!matches_context(&abbr, "git commit ", "", &cache));
        assert!(!matches_context(&abbr, "", "", &cache));
    }

    #[test]
    fn test_matches_rbuffer_pattern() {
        let abbr = make_contextual("--force", "--force-with-lease", None, Some("$"));
        let cache = regex_cache_for_abbr(&abbr);
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
        let cache = regex_cache_for_abbr(&abbr);
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
        let cache = RegexCache::from_matcher(&Matcher::new());
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

        let cache = build_regex_cache_from_contextual(&contextual);

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
    fn test_regex_cache_from_matcher() {
        let mut matcher = Matcher::new();
        matcher.contextual.entry("main".to_string()).or_default().push(
            make_contextual("main", "main --branch", Some("^git checkout"), None),
        );
        matcher.regex_abbrs.push(CompiledAbbr {
            keyword: r"^[A-Z]+$".to_string(),
            expansion: "uppercase_match".to_string(),
            scope: AbbrScope::RegexKeyword,
            ..Default::default()
        });

        let cache = RegexCache::from_matcher(&matcher);
        assert!(cache.get("^git checkout").is_some());
        assert!(cache.get(r"^[A-Z]+$").is_some());
        assert!(cache.get("nonexistent").is_none());
    }
}
