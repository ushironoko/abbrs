use crate::config::Abbreviation;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Compiled abbreviation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledAbbr {
    pub keyword: String,
    pub expansion: String,
    pub global: bool,
    pub evaluate: bool,
    pub lbuffer_pattern: Option<String>,
    pub rbuffer_pattern: Option<String>,
}

/// HashMap-based matching engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matcher {
    /// Command-position-only abbreviations (keyword -> compiled abbreviations)
    pub regular: FxHashMap<String, Vec<CompiledAbbr>>,
    /// Global abbreviations (keyword -> compiled abbreviations)
    pub global: FxHashMap<String, Vec<CompiledAbbr>>,
    /// Contextual abbreviations (require regex matching)
    pub contextual: Vec<CompiledAbbr>,
}

impl Matcher {
    pub fn new() -> Self {
        Self {
            regular: FxHashMap::default(),
            global: FxHashMap::default(),
            contextual: Vec::new(),
        }
    }
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Build Matcher from config
pub fn build(abbreviations: &[Abbreviation]) -> Matcher {
    let mut matcher = Matcher::new();

    for abbr in abbreviations {
        let compiled = CompiledAbbr {
            keyword: abbr.keyword.clone(),
            expansion: abbr.expansion.clone(),
            global: abbr.global,
            evaluate: abbr.evaluate,
            lbuffer_pattern: abbr.context.as_ref().and_then(|c| c.lbuffer.clone()),
            rbuffer_pattern: abbr.context.as_ref().and_then(|c| c.rbuffer.clone()),
        };

        if compiled.lbuffer_pattern.is_some() || compiled.rbuffer_pattern.is_some() {
            // Store contextual entries in separate list
            matcher.contextual.push(compiled);
        } else if abbr.global {
            matcher
                .global
                .entry(abbr.keyword.clone())
                .or_default()
                .push(compiled);
        } else {
            matcher
                .regular
                .entry(abbr.keyword.clone())
                .or_default()
                .push(compiled);
        }
    }

    matcher
}

/// Look up keyword in regular map (O(1))
pub fn lookup_regular<'a>(matcher: &'a Matcher, keyword: &str) -> Option<&'a CompiledAbbr> {
    matcher
        .regular
        .get(keyword)
        .and_then(|abbrs| abbrs.first())
}

/// Look up keyword in global map (O(1))
pub fn lookup_global<'a>(matcher: &'a Matcher, keyword: &str) -> Option<&'a CompiledAbbr> {
    matcher.global.get(keyword).and_then(|abbrs| abbrs.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Abbreviation, AbbreviationContext};

    fn make_abbr(keyword: &str, expansion: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            global: false,
            evaluate: false,
            allow_conflict: false,
            context: None,
        }
    }

    fn make_global_abbr(keyword: &str, expansion: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            global: true,
            evaluate: false,
            allow_conflict: false,
            context: None,
        }
    }

    fn make_contextual_abbr(keyword: &str, expansion: &str, lbuffer: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            global: false,
            evaluate: false,
            allow_conflict: false,
            context: Some(AbbreviationContext {
                lbuffer: Some(lbuffer.to_string()),
                rbuffer: None,
            }),
        }
    }

    #[test]
    fn test_build_regular() {
        let abbrs = vec![make_abbr("g", "git"), make_abbr("gc", "git commit")];
        let matcher = build(&abbrs);
        assert_eq!(matcher.regular.len(), 2);
        assert!(matcher.global.is_empty());
        assert!(matcher.contextual.is_empty());
    }

    #[test]
    fn test_build_global() {
        let abbrs = vec![make_global_abbr("NE", "2>/dev/null")];
        let matcher = build(&abbrs);
        assert!(matcher.regular.is_empty());
        assert_eq!(matcher.global.len(), 1);
        assert!(matcher.contextual.is_empty());
    }

    #[test]
    fn test_build_contextual() {
        let abbrs = vec![make_contextual_abbr(
            "main",
            "main --branch",
            "^git (checkout|switch)",
        )];
        let matcher = build(&abbrs);
        assert!(matcher.regular.is_empty());
        assert!(matcher.global.is_empty());
        assert_eq!(matcher.contextual.len(), 1);
    }

    #[test]
    fn test_build_mixed() {
        let abbrs = vec![
            make_abbr("g", "git"),
            make_global_abbr("NE", "2>/dev/null"),
            make_contextual_abbr("main", "main --branch", "^git checkout"),
        ];
        let matcher = build(&abbrs);
        assert_eq!(matcher.regular.len(), 1);
        assert_eq!(matcher.global.len(), 1);
        assert_eq!(matcher.contextual.len(), 1);
    }

    #[test]
    fn test_lookup_regular() {
        let abbrs = vec![make_abbr("g", "git"), make_abbr("gc", "git commit")];
        let matcher = build(&abbrs);

        let result = lookup_regular(&matcher, "g");
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "git");

        let result = lookup_regular(&matcher, "gc");
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "git commit");

        let result = lookup_regular(&matcher, "missing");
        assert!(result.is_none());
    }

    #[test]
    fn test_lookup_global() {
        let abbrs = vec![make_global_abbr("NE", "2>/dev/null")];
        let matcher = build(&abbrs);

        let result = lookup_global(&matcher, "NE");
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "2>/dev/null");

        let result = lookup_global(&matcher, "missing");
        assert!(result.is_none());
    }

    #[test]
    fn test_matcher_default() {
        let matcher = Matcher::default();
        assert!(matcher.regular.is_empty());
        assert!(matcher.global.is_empty());
        assert!(matcher.contextual.is_empty());
    }
}
