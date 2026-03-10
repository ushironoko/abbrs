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
                    function: false,
                    regex: false,
                    command: None,
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
            Some(attrs) => {
                if attrs.name.contains(' ') || attrs.name.is_empty() || attrs.expansion.is_empty() {
                    result.skipped.push(format!("{} (invalid format)", line));
                    continue;
                }
                let params = AddParams {
                    keyword: attrs.name,
                    expansion: attrs.expansion,
                    global: attrs.is_global,
                    evaluate: false,
                    function: attrs.is_function,
                    regex: attrs.is_regex,
                    command: attrs.command,
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

/// Parsed fish abbreviation attributes
struct FishAbbrAttrs {
    name: String,
    expansion: String,
    is_global: bool,
    is_function: bool,
    is_regex: bool,
    command: Option<String>,
}

fn parse_fish_abbr(line: &str) -> Option<FishAbbrAttrs> {
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
    let mut command: Option<String> = None;
    let mut i = 1;

    // Known fish abbr flags and their arities:
    // No-argument flags: -a/--add, -U/--universal, -g/--global,
    //                    -e/--erase, -l/--list, -s/--show, -q/--query, -h/--help
    // Flags that take a value: --function/-f FUNCTION, --regex/-r PATTERN,
    //                          --command/-c COMMAND, --position/-p VALUE, --set-cursor[=MARKER]
    let mut function_name: Option<String> = None;
    let mut regex_pattern: Option<String> = None;

    while i < parts.len() {
        match parts[i] {
            "-a" | "--add" | "-U" | "--universal" | "-e" | "--erase"
            | "-l" | "--list" | "-s" | "--show" | "-q" | "--query"
            | "-h" | "--help" => {
                i += 1;
            }
            "-r" | "--regex" => {
                // --regex PATTERN: takes the next token as the regex pattern
                if i + 1 < parts.len() {
                    regex_pattern = Some(strip_quotes(parts[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "-f" | "--function" => {
                // --function FUNCTION: takes the next token as the function name
                if i + 1 < parts.len() {
                    function_name = Some(strip_quotes(parts[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "-g" | "--global" => {
                is_global = true;
                i += 1;
            }
            "--" => {
                i += 1;
                break;
            }
            // --position anywhere means global in kort
            "--position" | "-p" => {
                if i + 1 < parts.len() {
                    if parts[i + 1] == "anywhere" {
                        is_global = true;
                    }
                    i += 2; // skip flag and its value
                } else {
                    i += 1;
                }
            }
            "--set-cursor" => {
                i += 2; // skip flag and its value
            }
            "--command" | "-c" => {
                if i + 1 < parts.len() {
                    command = Some(parts[i + 1].to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            s if s.starts_with('-') => {
                // Unknown flag — skip it and its potential value conservatively
                // If next token also starts with '-', it's another flag; otherwise skip it as a value
                i += 1;
                if i < parts.len() && !parts[i].starts_with('-') {
                    // Likely a value for the unknown flag
                    i += 1;
                }
            }
            _ => break,
        }
    }

    if i >= parts.len() {
        return None;
    }

    let name = strip_quotes(parts[i]);
    i += 1;

    // Collect positional expansion text (if present)
    let positional_expansion = if i < parts.len() {
        Some(strip_quotes(&parts[i..].join(" ")))
    } else {
        None
    };

    // Map to kort's model:
    // - keyword: use regex pattern if --regex was given, otherwise use NAME
    // - expansion: use positional text if present, otherwise function name (kort stores
    //   the function name in the expansion field when function = true)
    let keyword = regex_pattern.as_ref().cloned().unwrap_or(name);

    let expansion = if let Some(pos_exp) = positional_expansion {
        pos_exp
    } else if let Some(ref func) = function_name {
        func.clone()
    } else {
        return None;
    };

    Some(FishAbbrAttrs {
        name: keyword,
        expansion,
        is_global,
        is_function: function_name.is_some(),
        is_regex: regex_pattern.is_some(),
        command,
    })
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
                    // Shell aliases: raw shell command, no git prefix
                    (shell_cmd.trim().to_string(), true)
                } else {
                    // Regular aliases: prepend git
                    (format!("git {}", value), false)
                };

                let params = AddParams {
                    keyword: name,
                    expansion,
                    global: false,
                    evaluate,
                    function: false,
                    regex: false,
                    command: None,
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
        std::fs::write(&path, "[settings]\n").unwrap();
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
        let attrs = parse_fish_abbr("abbr -a g git").unwrap();
        assert_eq!(attrs.name, "g");
        assert_eq!(attrs.expansion, "git");
        assert!(!attrs.is_global);
        assert!(!attrs.is_function);
        assert!(!attrs.is_regex);
        assert!(attrs.command.is_none());
    }

    #[test]
    fn test_parse_fish_abbr_with_dashdash() {
        let attrs = parse_fish_abbr("abbr -a -- gc 'git commit'").unwrap();
        assert_eq!(attrs.name, "gc");
        assert_eq!(attrs.expansion, "git commit");
    }

    #[test]
    fn test_parse_fish_abbr_global() {
        let attrs = parse_fish_abbr("abbr -a -g NE '2>/dev/null'").unwrap();
        assert!(attrs.is_global);
    }

    #[test]
    fn test_parse_fish_abbr_with_command_flag() {
        // --command takes a value; should be preserved
        let attrs = parse_fish_abbr("abbr -a --command git co checkout").unwrap();
        assert_eq!(attrs.name, "co");
        assert_eq!(attrs.expansion, "checkout");
        assert_eq!(attrs.command, Some("git".to_string()));
    }

    #[test]
    fn test_parse_fish_abbr_with_position_flag() {
        let attrs = parse_fish_abbr("abbr -a --position anywhere NE '2>/dev/null'").unwrap();
        assert_eq!(attrs.name, "NE");
        assert_eq!(attrs.expansion, "2>/dev/null");
        assert!(attrs.is_global);
    }

    #[test]
    fn test_parse_fish_abbr_with_function_flag() {
        // --function takes the next token as the function name
        let attrs = parse_fish_abbr("abbr -a --function my_func -- mf").unwrap();
        assert_eq!(attrs.name, "mf");
        assert_eq!(attrs.expansion, "my_func");
        assert!(attrs.is_function);
    }

    #[test]
    fn test_parse_fish_abbr_with_regex_flag() {
        // --regex takes the next token as the regex pattern
        let attrs = parse_fish_abbr("abbr -a --regex '^gc$' -- gc 'git commit'").unwrap();
        assert_eq!(attrs.name, "^gc$");
        assert_eq!(attrs.expansion, "git commit");
        assert!(attrs.is_regex);
    }

    #[test]
    fn test_parse_fish_abbr_command_with_function() {
        // fish allows --command + --function together; both take values
        let attrs = parse_fish_abbr("abbr -a --command git --function my_handler -- co").unwrap();
        assert_eq!(attrs.name, "co");
        assert_eq!(attrs.expansion, "my_handler");
        assert_eq!(attrs.command, Some("git".to_string()));
        assert!(attrs.is_function);
    }

    #[test]
    fn test_parse_fish_abbr_short_command_flag() {
        // -c is short for --command
        let attrs = parse_fish_abbr("abbr -a -c git -- co checkout").unwrap();
        assert_eq!(attrs.name, "co");
        assert_eq!(attrs.expansion, "checkout");
        assert_eq!(attrs.command, Some("git".to_string()));
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
        // Shell aliases (!) should NOT get git prefix
        assert_eq!(cfg.abbr[0].expansion, "git log --oneline");
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
    fn test_import_fish_preserves_command_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        let fish_content = "abbr -a --command git co checkout\n";
        let result = import_fish(fish_content, &path).unwrap();
        assert_eq!(result.imported, 1);

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr[0].keyword, "co");
        assert_eq!(cfg.abbr[0].expansion, "checkout");
        assert_eq!(cfg.abbr[0].command, Some("git".to_string()));
    }

    #[test]
    fn test_import_fish_preserves_function_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        // --function takes a value; kort stores it as expansion with function=true
        let fish_content = "abbr -a --function my_func -- mf\n";
        let result = import_fish(fish_content, &path).unwrap();
        assert_eq!(result.imported, 1);

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr[0].keyword, "mf");
        assert_eq!(cfg.abbr[0].expansion, "my_func");
        assert!(cfg.abbr[0].function);
    }

    #[test]
    fn test_import_fish_preserves_regex_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        // --regex takes a value (the pattern); kort uses it as keyword
        let fish_content = "abbr -a --regex '^gc$' -- gc 'git commit'\n";
        let result = import_fish(fish_content, &path).unwrap();
        assert_eq!(result.imported, 1);

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr[0].keyword, "^gc$");
        assert_eq!(cfg.abbr[0].expansion, "git commit");
        assert!(cfg.abbr[0].regex);
    }

    #[test]
    fn test_import_fish_preserves_short_command_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = setup_config(&dir);

        // -c is short for --command
        let fish_content = "abbr -a -c git -- co checkout\n";
        let result = import_fish(fish_content, &path).unwrap();
        assert_eq!(result.imported, 1);

        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr[0].keyword, "co");
        assert_eq!(cfg.abbr[0].expansion, "checkout");
        assert_eq!(cfg.abbr[0].command, Some("git".to_string()));
    }

    #[test]
    fn test_strip_quotes() {
        assert_eq!(strip_quotes("'hello'"), "hello");
        assert_eq!(strip_quotes("\"hello\""), "hello");
        assert_eq!(strip_quotes("hello"), "hello");
    }
}
