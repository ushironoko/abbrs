use crate::add::{self, AddParams};
use anyhow::Result;
use std::path::Path;

/// Import from zsh alias output
/// Expected format: `alias_name='command'` or `alias_name="command"` or `alias_name=command`
pub fn import_aliases(alias_output: &str, config_path: &Path) -> Result<ImportResult> {
    let mut result = ImportResult::default();

    for line in alias_output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        match parse_zsh_alias(line) {
            Some((name, value)) => {
                if name.contains(' ') || name.is_empty() || value.is_empty() {
                    result.skipped.push(format!("{} (invalid format)", line));
                    continue;
                }
                let params = AddParams {
                    keyword: name,
                    expansion: value,
                    global: false,
                    evaluate: false,
                    allow_conflict: false,
                    context_lbuffer: None,
                    context_rbuffer: None,
                };
                match add::append_to_config(config_path, &params) {
                    Ok(()) => result.imported += 1,
                    Err(e) => result.skipped.push(format!("{} ({})", line, e)),
                }
            }
            None => {
                result.skipped.push(format!("{} (unrecognized format)", line));
            }
        }
    }

    Ok(result)
}

fn parse_zsh_alias(line: &str) -> Option<(String, String)> {
    // Format: name='value' or name="value" or name=value
    // Also handle: alias name='value'
    let line = line.strip_prefix("alias ").unwrap_or(line);

    let eq_pos = line.find('=')?;
    let name = line[..eq_pos].trim().to_string();
    let mut value = line[eq_pos + 1..].trim().to_string();

    // Strip surrounding quotes
    if (value.starts_with('\'') && value.ends_with('\''))
        || (value.starts_with('"') && value.ends_with('"'))
    {
        value = value[1..value.len() - 1].to_string();
    }

    Some((name, value))
}

/// Import from fish abbr output
/// Expected format: `abbr -a -- name 'expansion'` or `abbr -a name expansion`
pub fn import_fish(content: &str, config_path: &Path) -> Result<ImportResult> {
    let mut result = ImportResult::default();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        match parse_fish_abbr(line) {
            Some((name, value, is_global)) => {
                if name.contains(' ') || name.is_empty() || value.is_empty() {
                    result.skipped.push(format!("{} (invalid format)", line));
                    continue;
                }
                let params = AddParams {
                    keyword: name,
                    expansion: value,
                    global: is_global,
                    evaluate: false,
                    allow_conflict: false,
                    context_lbuffer: None,
                    context_rbuffer: None,
                };
                match add::append_to_config(config_path, &params) {
                    Ok(()) => result.imported += 1,
                    Err(e) => result.skipped.push(format!("{} ({})", line, e)),
                }
            }
            None => {
                result.skipped.push(format!("{} (unsupported fish format)", line));
            }
        }
    }

    Ok(result)
}

fn parse_fish_abbr(line: &str) -> Option<(String, String, bool)> {
    // Supported formats:
    // abbr -a name expansion
    // abbr -a -- name expansion
    // abbr -a -U name expansion
    // abbr --add name expansion
    let line = line.trim();
    if !line.starts_with("abbr") {
        return None;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }

    let mut is_global = false;
    let mut i = 1;

    // Skip flags
    while i < parts.len() {
        match parts[i] {
            "-a" | "--add" | "-U" | "--universal" | "-g" => {
                if parts[i] == "-g" {
                    is_global = true;
                }
                i += 1;
            }
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') => {
                // Skip unknown flags (like --position, --regex, --function)
                // These are unsupported features, will be handled at usage time
                if s == "--position" || s == "--set-cursor" {
                    i += 2; // skip flag and its argument
                } else {
                    i += 1;
                }
            }
            _ => break,
        }
    }

    if i >= parts.len() {
        return None;
    }

    let name = parts[i].to_string();
    i += 1;

    if i >= parts.len() {
        return None;
    }

    // Rest is the expansion (may be quoted)
    let expansion_str = parts[i..].join(" ");
    let expansion = strip_quotes(&expansion_str);

    Some((name, expansion, is_global))
}

fn strip_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Import from git aliases
/// Parses `git config --get-regexp ^alias\.` output
pub fn import_git_aliases(
    git_config_output: &str,
    config_path: &Path,
) -> Result<ImportResult> {
    let mut result = ImportResult::default();

    for line in git_config_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: alias.name value
        if let Some(rest) = line.strip_prefix("alias.") {
            if let Some(space_pos) = rest.find(|c: char| c.is_whitespace()) {
                let name = rest[..space_pos].to_string();
                let value = rest[space_pos..].trim().to_string();

                if name.is_empty() || value.is_empty() {
                    result.skipped.push(format!("{} (empty name or value)", line));
                    continue;
                }

                // Check if the alias is a shell command (starts with !)
                let (expansion, evaluate) = if let Some(shell_cmd) = value.strip_prefix('!') {
                    (shell_cmd.trim().to_string(), true)
                } else {
                    (value.clone(), false)
                };

                let params = AddParams {
                    keyword: name,
                    expansion: format!("git {}", expansion),
                    global: false,
                    evaluate,
                    allow_conflict: false,
                    context_lbuffer: None,
                    context_rbuffer: None,
                };
                match add::append_to_config(config_path, &params) {
                    Ok(()) => result.imported += 1,
                    Err(e) => result.skipped.push(format!("{} ({})", line, e)),
                }
            } else {
                result.skipped.push(format!("{} (no value)", line));
            }
        } else {
            result.skipped.push(format!("{} (not an alias)", line));
        }
    }

    Ok(result)
}

/// Export abbreviations in `kort add` format
pub fn export(config_path: &Path) -> Result<Vec<String>> {
    crate::manage::show(config_path, None)
}

#[derive(Debug, Default)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn setup_config(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("kort.toml");
        std::fs::write(&path, "[settings]\nstrict = false\n").unwrap();
        path
    }

    #[test]
    fn test_parse_zsh_alias_simple() {
        let (name, value) = parse_zsh_alias("g='git'").unwrap();
        assert_eq!(name, "g");
        assert_eq!(value, "git");
    }

    #[test]
    fn test_parse_zsh_alias_double_quotes() {
        let (name, value) = parse_zsh_alias("gc=\"git commit\"").unwrap();
        assert_eq!(name, "gc");
        assert_eq!(value, "git commit");
    }

    #[test]
    fn test_parse_zsh_alias_with_alias_prefix() {
        let (name, value) = parse_zsh_alias("alias g='git'").unwrap();
        assert_eq!(name, "g");
        assert_eq!(value, "git");
    }

    #[test]
    fn test_parse_zsh_alias_no_quotes() {
        let (name, value) = parse_zsh_alias("g=git").unwrap();
        assert_eq!(name, "g");
        assert_eq!(value, "git");
    }

    #[test]
    fn test_import_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        let alias_output = "g='git'\ngc='git commit'\n";
        let result = import_aliases(alias_output, &path).unwrap();
        assert_eq!(result.imported, 2);
        assert!(result.skipped.is_empty());

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr.len(), 2);
    }

    #[test]
    fn test_import_aliases_skips_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        let alias_output = "g='git'\n# comment\n=empty\n";
        let result = import_aliases(alias_output, &path).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_parse_fish_abbr_simple() {
        let (name, value, global) = parse_fish_abbr("abbr -a g git").unwrap();
        assert_eq!(name, "g");
        assert_eq!(value, "git");
        assert!(!global);
    }

    #[test]
    fn test_parse_fish_abbr_with_dashdash() {
        let (name, value, _) = parse_fish_abbr("abbr -a -- gc 'git commit'").unwrap();
        assert_eq!(name, "gc");
        assert_eq!(value, "git commit");
    }

    #[test]
    fn test_parse_fish_abbr_global() {
        let (_, _, global) = parse_fish_abbr("abbr -a -g NE '2>/dev/null'").unwrap();
        assert!(global);
    }

    #[test]
    fn test_import_fish() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        let fish_content = "abbr -a g git\nabbr -a gc 'git commit'\n";
        let result = import_fish(fish_content, &path).unwrap();
        assert_eq!(result.imported, 2);

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr.len(), 2);
    }

    #[test]
    fn test_import_git_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        let git_output = "alias.co checkout\nalias.ci commit\nalias.st status\n";
        let result = import_git_aliases(git_output, &path).unwrap();
        assert_eq!(result.imported, 3);

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr.len(), 3);
        assert_eq!(cfg.abbr[0].expansion, "git checkout");
    }

    #[test]
    fn test_import_git_aliases_shell_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        let git_output = "alias.lg !git log --oneline\n";
        let result = import_git_aliases(git_output, &path).unwrap();
        assert_eq!(result.imported, 1);

        let cfg = config::load(&path).unwrap();
        assert!(cfg.abbr[0].evaluate);
        assert_eq!(cfg.abbr[0].expansion, "git git log --oneline");
    }

    #[test]
    fn test_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");
        std::fs::write(
            &path,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true
"#,
        )
        .unwrap();

        let lines = export(&path).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("kort add "));
        assert!(lines[1].contains("--global"));
    }

    #[test]
    fn test_strip_quotes() {
        assert_eq!(strip_quotes("'hello'"), "hello");
        assert_eq!(strip_quotes("\"hello\""), "hello");
        assert_eq!(strip_quotes("hello"), "hello");
    }
}
