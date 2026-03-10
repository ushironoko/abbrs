use crate::config::Abbreviation;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Expansion method
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum ExpandAction {
    #[default]
    Replace,
    Evaluate,
    Function,
}

/// Abbreviation scope
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum AbbrScope {
    #[default]
    Regular,
    Global,
    CommandScoped {
        command: String,
    },
    Contextual {
        lbuffer: Option<String>,
        rbuffer: Option<String>,
    },
    RegexKeyword,
}

/// Compiled abbreviation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompiledAbbr {
    pub keyword: String,
    pub expansion: String,
    pub scope: AbbrScope,
    pub action: ExpandAction,
}

/// HashMap-based matching engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matcher {
    /// Command-position-only abbreviations (keyword -> compiled abbreviations)
    pub regular: FxHashMap<String, Vec<CompiledAbbr>>,
    /// Global abbreviations (keyword -> compiled abbreviations)
    pub global: FxHashMap<String, Vec<CompiledAbbr>>,
    /// Command-scoped abbreviations (command -> keyword -> compiled abbreviations)
    pub command_scoped: FxHashMap<String, FxHashMap<String, Vec<CompiledAbbr>>>,
    /// Contextual abbreviations (keyword -> compiled abbreviations, require regex matching)
    pub contextual: FxHashMap<String, Vec<CompiledAbbr>>,
    /// Regex-keyword abbreviations (keyword is a regex pattern)
    pub regex_abbrs: Vec<CompiledAbbr>,
    /// Reverse index for remind feature (expansion first word -> keyword)
    pub remind_index: FxHashMap<String, Vec<String>>,
    /// Prefix index: maps a typed prefix to candidate keywords (O(1) lookup)
    /// Built at compile time. Only stores proper prefixes (excludes exact matches).
    pub prefix_index: FxHashMap<String, Vec<String>>,
}

impl Matcher {
    pub fn new() -> Self {
        Self {
            regular: FxHashMap::default(),
            global: FxHashMap::default(),
            command_scoped: FxHashMap::default(),
            contextual: FxHashMap::default(),
            regex_abbrs: Vec::new(),
            remind_index: FxHashMap::default(),
            prefix_index: FxHashMap::default(),
        }
    }
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Build Matcher from config abbreviations
pub fn build(abbreviations: &[Abbreviation]) -> Matcher {
    let mut matcher = Matcher::new();

    for abbr in abbreviations {
        let scope = if let Some(ref ctx) = abbr.context {
            AbbrScope::Contextual {
                lbuffer: ctx.lbuffer.clone(),
                rbuffer: ctx.rbuffer.clone(),
            }
        } else if abbr.command.is_some() {
            AbbrScope::CommandScoped {
                command: abbr.command.clone().unwrap(),
            }
        } else if abbr.regex {
            AbbrScope::RegexKeyword
        } else if abbr.global {
            AbbrScope::Global
        } else {
            AbbrScope::Regular
        };

        let action = if abbr.function {
            ExpandAction::Function
        } else if abbr.evaluate {
            ExpandAction::Evaluate
        } else {
            ExpandAction::Replace
        };

        let compiled = CompiledAbbr {
            keyword: abbr.keyword.clone(),
            expansion: abbr.expansion.clone(),
            scope,
            action,
        };

        // Build remind index: map first word of expansion -> keyword
        if let Some(first_word) = compiled.expansion.split_whitespace().next() {
            matcher
                .remind_index
                .entry(first_word.to_string())
                .or_default()
                .push(compiled.keyword.clone());
        }

        match &compiled.scope {
            AbbrScope::RegexKeyword => matcher.regex_abbrs.push(compiled),
            AbbrScope::Contextual { .. } => {
                matcher
                    .contextual
                    .entry(abbr.keyword.clone())
                    .or_default()
                    .push(compiled);
            }
            AbbrScope::CommandScoped { command } => {
                matcher
                    .command_scoped
                    .entry(command.clone())
                    .or_default()
                    .entry(abbr.keyword.clone())
                    .or_default()
                    .push(compiled);
            }
            AbbrScope::Global => {
                matcher
                    .global
                    .entry(abbr.keyword.clone())
                    .or_default()
                    .push(compiled);
            }
            AbbrScope::Regular => {
                matcher
                    .regular
                    .entry(abbr.keyword.clone())
                    .or_default()
                    .push(compiled);
            }
        }
    }

    // Build prefix index for regular, global, and command_scoped keywords
    // Contextual and regex_abbrs are excluded (handled by existing priority chain)
    let mut all_keywords: Vec<&str> = Vec::new();
    for key in matcher.regular.keys() {
        all_keywords.push(key);
    }
    for key in matcher.global.keys() {
        if !all_keywords.contains(&key.as_str()) {
            all_keywords.push(key);
        }
    }
    for cmd_map in matcher.command_scoped.values() {
        for key in cmd_map.keys() {
            if !all_keywords.contains(&key.as_str()) {
                all_keywords.push(key);
            }
        }
    }

    for keyword in &all_keywords {
        // Generate all proper prefixes using char boundaries (safe for multibyte UTF-8)
        let char_boundaries: Vec<usize> = keyword.char_indices().map(|(i, _)| i).collect();
        // Skip the last boundary (full keyword = exact match, not a prefix)
        for &byte_pos in &char_boundaries[1..] {
            let prefix = &keyword[..byte_pos];
            matcher
                .prefix_index
                .entry(prefix.to_string())
                .or_default()
                .push(keyword.to_string());
        }
    }

    // Sort each prefix's candidates for stable output, then deduplicate
    for candidates in matcher.prefix_index.values_mut() {
        candidates.sort();
        candidates.dedup();
    }

    matcher
}

/// Get prefix-match candidates from the prefix index (O(1) lookup)
/// Returns compiled abbreviations matching the given prefix, filtered by scope.
pub fn prefix_candidates<'a>(
    matcher: &'a Matcher,
    prefix: &str,
    is_command_position: bool,
    current_command: Option<&str>,
) -> Vec<&'a CompiledAbbr> {
    let Some(keywords) = matcher.prefix_index.get(prefix) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for keyword in keywords {
        // Command-scoped lookup
        if let Some(cmd) = current_command {
            if let Some(abbr) = lookup_command_scoped(matcher, cmd, keyword) {
                result.push(abbr);
                continue;
            }
        }
        // Regular (only in command position)
        if is_command_position {
            if let Some(abbr) = lookup_regular(matcher, keyword) {
                result.push(abbr);
                continue;
            }
        }
        // Global (any position)
        if let Some(abbr) = lookup_global(matcher, keyword) {
            result.push(abbr);
        }
    }

    result
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

/// Look up keyword in command-scoped map (O(1))
pub fn lookup_command_scoped<'a>(
    matcher: &'a Matcher,
    command: &str,
    keyword: &str,
) -> Option<&'a CompiledAbbr> {
    matcher
        .command_scoped
        .get(command)
        .and_then(|kw_map| kw_map.get(keyword))
        .and_then(|abbrs| abbrs.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Abbreviation, AbbreviationContext};

    fn make_abbr(keyword: &str, expansion: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            ..Default::default()
        }
    }

    fn make_global_abbr(keyword: &str, expansion: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            global: true,
            ..Default::default()
        }
    }

    fn make_contextual_abbr(keyword: &str, expansion: &str, lbuffer: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            context: Some(AbbreviationContext {
                lbuffer: Some(lbuffer.to_string()),
                rbuffer: None,
            }),
            ..Default::default()
        }
    }

    fn make_command_scoped_abbr(
        keyword: &str,
        expansion: &str,
        command: &str,
    ) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: expansion.to_string(),
            command: Some(command.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_build_regular() {
        let abbrs = vec![make_abbr("g", "git"), make_abbr("gc", "git commit")];
        let matcher = build(&abbrs);
        assert_eq!(matcher.regular.len(), 2);
        assert!(matcher.global.is_empty());
        assert!(matcher.contextual.is_empty());
        assert!(matcher.command_scoped.is_empty());
        assert!(matcher.regex_abbrs.is_empty());
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
    fn test_build_command_scoped() {
        let abbrs = vec![make_command_scoped_abbr("co", "checkout", "git")];
        let matcher = build(&abbrs);
        assert!(matcher.regular.is_empty());
        assert!(matcher.global.is_empty());
        assert_eq!(matcher.command_scoped.len(), 1);
        assert!(matcher.command_scoped.contains_key("git"));
    }

    #[test]
    fn test_build_mixed() {
        let abbrs = vec![
            make_abbr("g", "git"),
            make_global_abbr("NE", "2>/dev/null"),
            make_contextual_abbr("main", "main --branch", "^git checkout"),
            make_command_scoped_abbr("co", "checkout", "git"),
        ];
        let matcher = build(&abbrs);
        assert_eq!(matcher.regular.len(), 1);
        assert_eq!(matcher.global.len(), 1);
        assert_eq!(matcher.contextual.len(), 1);
        assert_eq!(matcher.command_scoped.len(), 1);
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
    fn test_lookup_command_scoped() {
        let abbrs = vec![make_command_scoped_abbr("co", "checkout", "git")];
        let matcher = build(&abbrs);

        let result = lookup_command_scoped(&matcher, "git", "co");
        assert!(result.is_some());
        assert_eq!(result.unwrap().expansion, "checkout");

        let result = lookup_command_scoped(&matcher, "git", "missing");
        assert!(result.is_none());

        let result = lookup_command_scoped(&matcher, "npm", "co");
        assert!(result.is_none());
    }

    #[test]
    fn test_expand_action_conversion() {
        let abbr_eval = Abbreviation {
            keyword: "T".to_string(),
            expansion: "date".to_string(),
            evaluate: true,
            ..Default::default()
        };
        let abbr_func = Abbreviation {
            keyword: "F".to_string(),
            expansion: "my_func".to_string(),
            function: true,
            ..Default::default()
        };
        let abbr_replace = Abbreviation {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            ..Default::default()
        };

        let matcher = build(&[abbr_eval, abbr_func, abbr_replace]);

        let t = lookup_regular(&matcher, "T").unwrap();
        assert_eq!(t.action, ExpandAction::Evaluate);

        let f = lookup_regular(&matcher, "F").unwrap();
        assert_eq!(f.action, ExpandAction::Function);

        let g = lookup_regular(&matcher, "g").unwrap();
        assert_eq!(g.action, ExpandAction::Replace);
    }

    #[test]
    fn test_abbr_scope_conversion() {
        let abbrs = vec![
            make_abbr("g", "git"),
            make_global_abbr("NE", "2>/dev/null"),
            make_contextual_abbr("main", "main --branch", "^git checkout"),
            make_command_scoped_abbr("co", "checkout", "git"),
        ];
        let matcher = build(&abbrs);

        let g = lookup_regular(&matcher, "g").unwrap();
        assert_eq!(g.scope, AbbrScope::Regular);

        let ne = lookup_global(&matcher, "NE").unwrap();
        assert_eq!(ne.scope, AbbrScope::Global);

        let main_ctx = matcher.contextual.get("main").unwrap().first().unwrap();
        assert!(matches!(main_ctx.scope, AbbrScope::Contextual { .. }));

        let co = lookup_command_scoped(&matcher, "git", "co").unwrap();
        assert!(matches!(co.scope, AbbrScope::CommandScoped { .. }));
    }

    #[test]
    fn test_matcher_default() {
        let matcher = Matcher::default();
        assert!(matcher.regular.is_empty());
        assert!(matcher.global.is_empty());
        assert!(matcher.contextual.is_empty());
        assert!(matcher.command_scoped.is_empty());
        assert!(matcher.regex_abbrs.is_empty());
        assert!(matcher.remind_index.is_empty());
    }

    #[test]
    fn test_prefix_index_built() {
        let abbrs = vec![
            make_abbr("g", "git"),
            make_abbr("gc", "git commit"),
            make_abbr("gp", "git push"),
            make_abbr("gd", "git diff"),
        ];
        let matcher = build(&abbrs);

        // "g" is an exact match for "g", so prefix_index["g"] should contain gc, gd, gp
        let g_candidates = matcher.prefix_index.get("g").unwrap();
        assert_eq!(g_candidates, &vec!["gc", "gd", "gp"]);

        // No prefix index entry for "gc" since only "gp" would not match
        assert!(matcher.prefix_index.get("gc").is_none());
        assert!(matcher.prefix_index.get("gp").is_none());
    }

    #[test]
    fn test_prefix_index_sorted() {
        let abbrs = vec![
            make_abbr("ls", "ls --color"),
            make_abbr("la", "ls -A"),
            make_abbr("ll", "ls -alF"),
            make_abbr("lg", "ls -G"),
        ];
        let matcher = build(&abbrs);

        let l_candidates = matcher.prefix_index.get("l").unwrap();
        assert_eq!(l_candidates, &vec!["la", "lg", "ll", "ls"]);
    }

    #[test]
    fn test_prefix_candidates_command_position() {
        let abbrs = vec![
            make_abbr("gc", "git commit"),
            make_abbr("gp", "git push"),
            make_abbr("gd", "git diff"),
        ];
        let matcher = build(&abbrs);

        let candidates = prefix_candidates(&matcher, "g", true, None);
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn test_prefix_candidates_not_command_position() {
        let abbrs = vec![
            make_abbr("gc", "git commit"),
            make_global_abbr("gx", "global x"),
        ];
        let matcher = build(&abbrs);

        // Not in command position: only global should match
        let candidates = prefix_candidates(&matcher, "g", false, None);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].keyword, "gx");
    }

    #[test]
    fn test_prefix_candidates_command_scoped() {
        let abbrs = vec![
            make_command_scoped_abbr("co", "checkout", "git"),
            make_command_scoped_abbr("cm", "commit", "git"),
        ];
        let matcher = build(&abbrs);

        let candidates = prefix_candidates(&matcher, "c", false, Some("git"));
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn test_prefix_candidates_empty() {
        let abbrs = vec![make_abbr("g", "git")];
        let matcher = build(&abbrs);

        let candidates = prefix_candidates(&matcher, "x", true, None);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_prefix_index_excludes_contextual_and_regex() {
        let abbrs = vec![
            make_contextual_abbr("main", "main --branch", "^git checkout"),
            Abbreviation {
                keyword: "^b".to_string(),
                expansion: "branch".to_string(),
                regex: true,
                ..Default::default()
            },
        ];
        let matcher = build(&abbrs);

        // Contextual and regex keywords should not be in prefix_index
        assert!(matcher.prefix_index.is_empty());
    }

    #[test]
    fn test_remind_index() {
        let abbrs = vec![
            make_abbr("g", "git"),
            make_abbr("gc", "git commit"),
            make_global_abbr("NE", "2>/dev/null"),
        ];
        let matcher = build(&abbrs);

        // "git" should map to both "g" and "gc"
        let git_entries = matcher.remind_index.get("git").unwrap();
        assert!(git_entries.contains(&"g".to_string()));
        assert!(git_entries.contains(&"gc".to_string()));

        // "2>/dev/null" should map to "NE"
        let ne_entries = matcher.remind_index.get("2>/dev/null").unwrap();
        assert!(ne_entries.contains(&"NE".to_string()));
    }

    #[test]
    fn test_prefix_index_multibyte_keywords() {
        // Regression test: multibyte UTF-8 keywords must not panic during prefix index build
        let abbrs = vec![
            make_abbr("あいう", "hello"),
            make_abbr("あいえ", "world"),
        ];
        let matcher = build(&abbrs);

        // Prefix "あ" should match both keywords
        let candidates = matcher.prefix_index.get("あ").unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(candidates.contains(&"あいう".to_string()));
        assert!(candidates.contains(&"あいえ".to_string()));

        // Prefix "あい" should also match both
        let candidates = matcher.prefix_index.get("あい").unwrap();
        assert_eq!(candidates.len(), 2);

        // prefix_candidates should work with multibyte prefix
        let results = prefix_candidates(&matcher, "あ", true, None);
        assert_eq!(results.len(), 2);
    }
}
