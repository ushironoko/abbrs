use anyhow::{Context as _, Result};
use std::path::Path;

use crate::config;

/// Erase an abbreviation from the config file
/// Returns true if an entry was removed, false if not found
pub fn erase(
    config_path: &Path,
    keyword: &str,
    command: Option<&str>,
    global: bool,
) -> Result<bool> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file: {}", config_path.display()))?;

    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .context("failed to parse config as TOML")?;

    let Some(abbr_array) = doc.get_mut("abbr").and_then(|v| v.as_array_of_tables_mut()) else {
        return Ok(false);
    };

    let mut found_indices = Vec::new();
    for (i, table) in abbr_array.iter().enumerate() {
        let kw = table.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
        if kw != keyword {
            continue;
        }
        // Match scope
        if let Some(cmd) = command {
            let entry_cmd = table.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if entry_cmd == cmd {
                found_indices.push(i);
            }
        } else if global {
            let entry_global = table
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if entry_global {
                found_indices.push(i);
            }
        } else {
            // Regular: no command, no global, no context
            let has_command = table.get("command").is_some();
            let has_global = table
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let has_context = table.get("context").is_some();
            if !has_command && !has_global && !has_context {
                found_indices.push(i);
            }
        }
    }

    if found_indices.is_empty() {
        return Ok(false);
    }
    if found_indices.len() > 1 {
        anyhow::bail!(
            "multiple entries match keyword \"{}\". Use --command or --global to specify scope.",
            keyword
        );
    }

    // Remove the entry (remove from back to preserve indices)
    abbr_array.remove(found_indices[0]);

    let new_content = doc.to_string();
    // Verify the result is still valid
    config::parse(&new_content).context("config validation failed after erase")?;
    std::fs::write(config_path, &new_content)
        .with_context(|| format!("failed to write config file: {}", config_path.display()))?;

    Ok(true)
}

/// Rename an abbreviation's keyword
/// Returns true if renamed, false if not found
pub fn rename(
    config_path: &Path,
    old_keyword: &str,
    new_keyword: &str,
    command: Option<&str>,
    global: bool,
) -> Result<bool> {
    // Validate new keyword
    if new_keyword.is_empty() {
        anyhow::bail!("new keyword must not be empty");
    }
    if new_keyword.contains(' ') {
        anyhow::bail!("new keyword must not contain spaces");
    }

    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file: {}", config_path.display()))?;

    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .context("failed to parse config as TOML")?;

    let Some(abbr_array) = doc.get_mut("abbr").and_then(|v| v.as_array_of_tables_mut()) else {
        return Ok(false);
    };

    let mut found_indices = Vec::new();
    for (i, table) in abbr_array.iter().enumerate() {
        let kw = table.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
        if kw != old_keyword {
            continue;
        }
        if let Some(cmd) = command {
            let entry_cmd = table.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if entry_cmd == cmd {
                found_indices.push(i);
            }
        } else if global {
            let entry_global = table
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if entry_global {
                found_indices.push(i);
            }
        } else {
            let has_command = table.get("command").is_some();
            let has_global = table
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let has_context = table.get("context").is_some();
            if !has_command && !has_global && !has_context {
                found_indices.push(i);
            }
        }
    }

    if found_indices.is_empty() {
        return Ok(false);
    }
    if found_indices.len() > 1 {
        anyhow::bail!(
            "multiple entries match keyword \"{}\". Use --command or --global to specify scope.",
            old_keyword
        );
    }

    let idx = found_indices[0];
    abbr_array
        .get_mut(idx)
        .unwrap()
        .insert("keyword", toml_edit::value(new_keyword));

    let new_content = doc.to_string();
    config::parse(&new_content).context("config validation failed after rename")?;
    std::fs::write(config_path, &new_content)
        .with_context(|| format!("failed to write config file: {}", config_path.display()))?;

    Ok(true)
}

/// Query if an abbreviation exists
/// Returns true if found
pub fn query(
    config_path: &Path,
    keyword: &str,
    command: Option<&str>,
    global: bool,
) -> Result<bool> {
    let cfg = config::load(config_path)?;

    for abbr in &cfg.abbr {
        if abbr.keyword != keyword {
            continue;
        }
        if let Some(cmd) = command {
            if abbr.command.as_deref() == Some(cmd) {
                return Ok(true);
            }
        } else if global {
            if abbr.global {
                return Ok(true);
            }
        } else {
            if abbr.command.is_none() && !abbr.global && abbr.context.is_none() {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Show abbreviations in `abbrs add` re-importable format
pub fn show(config_path: &Path, keyword_filter: Option<&str>) -> Result<Vec<String>> {
    let cfg = config::load(config_path)?;
    let mut lines = Vec::new();

    for abbr in &cfg.abbr {
        if let Some(filter) = keyword_filter {
            if abbr.keyword != filter {
                continue;
            }
        }

        let mut cmd = format!("abbrs add {} {}", shell_quote(&abbr.keyword), shell_quote(&abbr.expansion));

        if abbr.global {
            cmd.push_str(" --global");
        }
        if abbr.evaluate {
            cmd.push_str(" --evaluate");
        }
        if abbr.function {
            cmd.push_str(" --function");
        }
        if let Some(ref c) = abbr.command {
            cmd.push_str(&format!(" --command {}", shell_quote(c)));
        }
        if abbr.regex {
            cmd.push_str(" --regex");
        }
        if abbr.allow_conflict {
            cmd.push_str(" --allow-conflict");
        }
        if let Some(ref ctx) = abbr.context {
            if let Some(ref lb) = ctx.lbuffer {
                cmd.push_str(&format!(" --context-lbuffer {}", shell_quote(lb)));
            }
            if let Some(ref rb) = ctx.rbuffer {
                cmd.push_str(&format!(" --context-rbuffer {}", shell_quote(rb)));
            }
        }

        lines.push(cmd);
    }

    Ok(lines)
}

fn shell_quote(s: &str) -> String {
    if s.contains(|c: char| c.is_whitespace() || "'\"\\|&;()$`!{}[]<>?*#~".contains(c)) {
        // POSIX single-quote escaping: wrap in single quotes,
        // and replace each embedded ' with '\'' (end quote, escaped quote, start quote)
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_config(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let config_path = dir.path().join("abbrs.toml");
        std::fs::write(&config_path, content).unwrap();
        config_path
    }

    #[test]
    fn test_erase_regular() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit"
"#,
        );

        assert!(erase(&path, "g", None, false).unwrap());
        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr.len(), 1);
        assert_eq!(cfg.abbr[0].keyword, "gc");
    }

    #[test]
    fn test_erase_not_found() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );

        assert!(!erase(&path, "missing", None, false).unwrap());
    }

    #[test]
    fn test_erase_command_scoped() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "co"
expansion = "checkout"
command = "git"

[[abbr]]
keyword = "co"
expansion = "compile"
"#,
        );

        assert!(erase(&path, "co", Some("git"), false).unwrap());
        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr.len(), 1);
        assert_eq!(cfg.abbr[0].expansion, "compile");
    }

    #[test]
    fn test_erase_global() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true

[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );

        assert!(erase(&path, "NE", None, true).unwrap());
        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr.len(), 1);
        assert_eq!(cfg.abbr[0].keyword, "g");
    }

    #[test]
    fn test_rename_regular() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );

        assert!(rename(&path, "g", "gt", None, false).unwrap());
        let cfg = config::load(&path).unwrap();
        assert_eq!(cfg.abbr[0].keyword, "gt");
    }

    #[test]
    fn test_rename_not_found() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );

        assert!(!rename(&path, "missing", "new", None, false).unwrap());
    }

    #[test]
    fn test_rename_invalid_keyword() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );

        assert!(rename(&path, "g", "", None, false).is_err());
        assert!(rename(&path, "g", "a b", None, false).is_err());
    }

    #[test]
    fn test_query_found() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true

[[abbr]]
keyword = "co"
expansion = "checkout"
command = "git"
"#,
        );

        assert!(query(&path, "g", None, false).unwrap());
        assert!(query(&path, "NE", None, true).unwrap());
        assert!(query(&path, "co", Some("git"), false).unwrap());
    }

    #[test]
    fn test_query_not_found() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );

        assert!(!query(&path, "missing", None, false).unwrap());
        assert!(!query(&path, "g", None, true).unwrap()); // g is not global
        assert!(!query(&path, "g", Some("git"), false).unwrap()); // g has no command
    }

    #[test]
    fn test_show_all() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true
"#,
        );

        let lines = show(&path, None).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("abbrs add g git"));
        assert!(lines[1].contains("--global"));
    }

    #[test]
    fn test_show_filtered() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit"
"#,
        );

        let lines = show(&path, Some("gc")).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("abbrs add gc"));
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("git"), "git");
        assert_eq!(shell_quote("git commit"), "'git commit'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("2>/dev/null"), "'2>/dev/null'");
        // Strings with $, backticks, etc. are safely single-quoted
        assert_eq!(shell_quote("echo $HOME"), "'echo $HOME'");
        assert_eq!(shell_quote("echo `date`"), "'echo `date`'");
    }
}
