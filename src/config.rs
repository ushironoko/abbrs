use anyhow::{Context as _, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub abbr: Vec<Abbreviation>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub remind: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Abbreviation {
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub expansion: String,
    #[serde(default)]
    pub global: bool,
    #[serde(default)]
    pub evaluate: bool,
    #[serde(default)]
    pub function: bool,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub allow_conflict: bool,
    #[serde(default)]
    pub context: Option<AbbreviationContext>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AbbreviationContext {
    #[serde(default)]
    pub lbuffer: Option<String>,
    #[serde(default)]
    pub rbuffer: Option<String>,
}

pub fn load(path: &Path) -> Result<Config> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("failed to read config file: {}", path.display()))?;
    parse(&content)
}

pub fn parse(content: &str) -> Result<Config> {
    let config: Config = toml::from_str(content).context("TOML parse error")?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<()> {
    for abbr in &config.abbr {
        if abbr.keyword.is_empty() {
            anyhow::bail!("abbreviation has empty keyword");
        }
        if abbr.expansion.is_empty() {
            anyhow::bail!(
                "keyword \"{}\" has empty expansion",
                abbr.keyword
            );
        }
        if abbr.keyword.contains(' ') {
            anyhow::bail!(
                "keyword \"{}\" must not contain spaces",
                abbr.keyword
            );
        }
        // validate mutually exclusive options
        if abbr.function && abbr.evaluate {
            anyhow::bail!(
                "keyword \"{}\" cannot have both function and evaluate",
                abbr.keyword
            );
        }
        if abbr.command.is_some() && abbr.global {
            anyhow::bail!(
                "keyword \"{}\" cannot have both command and global",
                abbr.keyword
            );
        }
        if abbr.regex {
            regex::Regex::new(&abbr.keyword).with_context(|| {
                format!(
                    "keyword \"{}\" has regex = true but invalid regex pattern",
                    abbr.keyword
                )
            })?;
        }
        // validate context regex patterns
        if let Some(ctx) = &abbr.context {
            if let Some(ref pat) = ctx.lbuffer {
                regex::Regex::new(pat).with_context(|| {
                    format!(
                        "keyword \"{}\" has invalid context.lbuffer regex: {}",
                        abbr.keyword, pat
                    )
                })?;
            }
            if let Some(ref pat) = ctx.rbuffer {
                regex::Regex::new(pat).with_context(|| {
                    format!(
                        "keyword \"{}\" has invalid context.rbuffer regex: {}",
                        abbr.keyword, pat
                    )
                })?;
            }
        }
    }
    Ok(())
}

/// Get the default config file path
pub fn default_config_path() -> Result<std::path::PathBuf> {
    let xdg = xdg::BaseDirectories::with_prefix("kort").context("failed to get XDG directories")?;
    Ok(xdg.get_config_home().join("kort.toml"))
}

/// Get the default cache file path
pub fn default_cache_path() -> Result<std::path::PathBuf> {
    let xdg = xdg::BaseDirectories::with_prefix("kort").context("failed to get XDG directories")?;
    Ok(xdg.get_cache_home().join("kort.cache"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#;
        let config = parse(toml).unwrap();
        assert_eq!(config.abbr.len(), 1);
        assert_eq!(config.abbr[0].keyword, "g");
        assert_eq!(config.abbr[0].expansion, "git");
        assert!(!config.abbr[0].global);
        assert!(!config.abbr[0].evaluate);
        assert!(!config.abbr[0].allow_conflict);
        assert!(config.abbr[0].context.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit -m '{{message}}'"

[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true

[[abbr]]
keyword = "main"
expansion = "main --branch"
context.lbuffer = "^git (checkout|switch)"

[[abbr]]
keyword = "gs"
expansion = "git status --short"
allow_conflict = true

[[abbr]]
keyword = "TODAY"
expansion = "date +%Y-%m-%d"
evaluate = true
global = true
"#;
        let config = parse(toml).unwrap();
        assert_eq!(config.abbr.len(), 6);

        // global
        assert!(config.abbr[2].global);
        // context
        assert!(config.abbr[3].context.is_some());
        let ctx = config.abbr[3].context.as_ref().unwrap();
        assert_eq!(ctx.lbuffer.as_deref(), Some("^git (checkout|switch)"));
        // allow_conflict
        assert!(config.abbr[4].allow_conflict);
        // evaluate
        assert!(config.abbr[5].evaluate);
        assert!(config.abbr[5].global);
    }

    #[test]
    fn test_parse_empty_config() {
        let toml = "";
        let config = parse(toml).unwrap();
        assert!(config.abbr.is_empty());
    }

    #[test]
    fn test_validate_empty_keyword() {
        let toml = r#"
[[abbr]]
keyword = ""
expansion = "git"
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty keyword"));
    }

    #[test]
    fn test_validate_empty_expansion() {
        let toml = r#"
[[abbr]]
keyword = "g"
expansion = ""
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty expansion"));
    }

    #[test]
    fn test_validate_keyword_with_space() {
        let toml = r#"
[[abbr]]
keyword = "g c"
expansion = "git commit"
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must not contain spaces"));
    }

    #[test]
    fn test_validate_invalid_regex() {
        let toml = r#"
[[abbr]]
keyword = "main"
expansion = "main --branch"
context.lbuffer = "[invalid"
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid context.lbuffer regex"));
    }

    #[test]
    fn test_validate_function_and_evaluate_exclusive() {
        let toml = r#"
[[abbr]]
keyword = "x"
expansion = "y"
function = true
evaluate = true
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("function and evaluate"));
    }

    #[test]
    fn test_validate_command_and_global_exclusive() {
        let toml = r#"
[[abbr]]
keyword = "x"
expansion = "y"
command = "git"
global = true
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("command and global"));
    }

    #[test]
    fn test_validate_regex_keyword_pattern() {
        let toml = r#"
[[abbr]]
keyword = "[invalid"
expansion = "y"
regex = true
"#;
        let result = parse(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid regex pattern"));
    }

    #[test]
    fn test_parse_new_fields() {
        let toml = r#"
[settings]
prefixes = ["sudo", "doas"]
remind = true

[[abbr]]
keyword = "co"
expansion = "checkout"
command = "git"

[[abbr]]
keyword = "mf"
expansion = "my_func"
function = true
"#;
        let config = parse(toml).unwrap();
        assert_eq!(config.settings.prefixes, vec!["sudo", "doas"]);
        assert!(config.settings.remind);
        assert_eq!(config.abbr[0].command, Some("git".to_string()));
        assert!(config.abbr[1].function);
    }

    #[test]
    fn test_default_settings() {
        let config = Config {
            settings: Settings::default(),
            abbr: vec![],
        };
        assert!(config.settings.prefixes.is_empty());
        assert!(!config.settings.remind);
    }
}
