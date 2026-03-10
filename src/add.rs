use anyhow::{Context as _, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal;
use std::io::{self, Write};
use std::path::Path;

use crate::config;

/// Parameters for adding a new abbreviation
pub struct AddParams {
    pub keyword: String,
    pub expansion: String,
    pub global: bool,
    pub evaluate: bool,
    pub function: bool,
    pub regex: bool,
    pub command: Option<String>,
    pub allow_conflict: bool,
    pub context_lbuffer: Option<String>,
    pub context_rbuffer: Option<String>,
}

/// Append a new abbreviation entry to the config file
pub fn append_to_config(path: &Path, params: &AddParams) -> Result<()> {
    // Validate the new entry fields before touching the file
    validate_params(params)?;

    // Read existing content (or empty string for new file)
    let existing_content = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?
    } else {
        String::new()
    };

    // Check for duplicate keywords in existing config (scope-aware)
    if !existing_content.is_empty() {
        let existing = config::parse(&existing_content)?;
        if let Some(dup) = existing.abbr.iter().find(|a| {
            if a.keyword != params.keyword {
                return false;
            }
            // Same keyword — only a true duplicate if scopes match
            let same_command = a.command == params.command;
            let same_global = a.global == params.global;
            let same_regex = a.regex == params.regex;
            let same_context = match (&a.context, &params.context_lbuffer, &params.context_rbuffer) {
                // Both have no context
                (None, None, None) => true,
                // Existing has context, new does not (or vice versa)
                (Some(_), None, None) | (None, Some(_), _) | (None, _, Some(_)) => false,
                // Both have context — compare actual pattern values
                (Some(ctx), _, _) => {
                    ctx.lbuffer.as_deref() == params.context_lbuffer.as_deref()
                        && ctx.rbuffer.as_deref() == params.context_rbuffer.as_deref()
                }
            };
            same_command && same_global && same_regex && same_context
        }) {
            anyhow::bail!(
                "keyword \"{}\" already exists in config with the same scope (expansion: \"{}\")",
                params.keyword,
                dup.expansion
            );
        }
    }

    let entry = build_toml_entry(params);

    // Build the combined content and validate BEFORE writing to disk
    let mut combined = existing_content.clone();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&entry);

    config::parse(&combined)
        .with_context(|| "config validation failed: the new entry is invalid")?;

    // Validation passed — now write to disk
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open config file: {}", path.display()))?;

    if !existing_content.is_empty() && !existing_content.ends_with('\n') {
        writeln!(file)?;
    }

    write!(file, "{}", entry)?;

    Ok(())
}

fn validate_params(params: &AddParams) -> Result<()> {
    if params.keyword.is_empty() {
        anyhow::bail!("keyword must not be empty");
    }
    if params.keyword.contains(' ') {
        anyhow::bail!("keyword must not contain spaces");
    }
    if params.expansion.is_empty() {
        anyhow::bail!("expansion must not be empty");
    }
    // Mutual exclusion checks (same rules as config::validate)
    if params.function && params.evaluate {
        anyhow::bail!(
            "keyword \"{}\" cannot have both function and evaluate",
            params.keyword
        );
    }
    if params.command.is_some() && params.global {
        anyhow::bail!(
            "keyword \"{}\" cannot have both command and global",
            params.keyword
        );
    }
    if params.regex {
        regex::Regex::new(&params.keyword).with_context(|| {
            format!(
                "keyword \"{}\" has regex = true but invalid regex pattern",
                params.keyword
            )
        })?;
    }
    if let Some(ref pat) = params.context_lbuffer {
        regex::Regex::new(pat)
            .with_context(|| format!("invalid context.lbuffer regex: {}", pat))?;
    }
    if let Some(ref pat) = params.context_rbuffer {
        regex::Regex::new(pat)
            .with_context(|| format!("invalid context.rbuffer regex: {}", pat))?;
    }
    Ok(())
}

fn build_toml_entry(params: &AddParams) -> String {
    let mut entry = String::new();
    entry.push_str("\n[[abbr]]\n");
    entry.push_str(&format!("keyword = {}\n", toml_quote(&params.keyword)));
    entry.push_str(&format!("expansion = {}\n", toml_quote(&params.expansion)));

    if params.global {
        entry.push_str("global = true\n");
    }
    if params.evaluate {
        entry.push_str("evaluate = true\n");
    }
    if params.function {
        entry.push_str("function = true\n");
    }
    if params.regex {
        entry.push_str("regex = true\n");
    }
    if let Some(ref cmd) = params.command {
        entry.push_str(&format!("command = {}\n", toml_quote(cmd)));
    }
    if params.allow_conflict {
        entry.push_str("allow_conflict = true\n");
    }
    if let Some(ref pat) = params.context_lbuffer {
        entry.push_str(&format!("context.lbuffer = {}\n", toml_quote(pat)));
    }
    if let Some(ref pat) = params.context_rbuffer {
        entry.push_str(&format!("context.rbuffer = {}\n", toml_quote(pat)));
    }

    entry
}

/// Quote a string for TOML, using basic or literal string as appropriate
fn toml_quote(s: &str) -> String {
    // Use toml crate's serializer for correct escaping
    toml::Value::String(s.to_string()).to_string()
}

/// Run interactive prompt to collect abbreviation parameters
pub fn interactive_prompt() -> Result<AddParams> {
    let keyword = prompt_required("keyword")?;
    let expansion = prompt_required("expansion")?;

    let abbr_type = prompt_select(
        "type",
        &["regular", "global", "context"],
    )?;

    let global = abbr_type == "global";

    let (context_lbuffer, context_rbuffer) = if abbr_type == "context" {
        let lb = prompt_optional("context.lbuffer (regex, Enter to skip)")?;
        let rb = prompt_optional("context.rbuffer (regex, Enter to skip)")?;
        (lb, rb)
    } else {
        (None, None)
    };

    let evaluate = prompt_confirm("evaluate (run as command)?", false)?;
    let allow_conflict = prompt_confirm("allow conflict with PATH commands?", false)?;

    Ok(AddParams {
        keyword,
        expansion,
        global,
        evaluate,
        function: false,
        regex: false,
        command: None,
        allow_conflict,
        context_lbuffer,
        context_rbuffer,
    })
}

/// Prompt for a required string input
fn prompt_required(label: &str) -> Result<String> {
    loop {
        eprint!("  {} > ", label);
        io::stderr().flush()?;
        let input = read_line()?;
        let trimmed = input.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
        eprintln!("  (required)");
    }
}

/// Prompt for an optional string input
fn prompt_optional(label: &str) -> Result<Option<String>> {
    eprint!("  {} > ", label);
    io::stderr().flush()?;
    let input = read_line()?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

/// Prompt for a yes/no confirmation using crossterm key events
fn prompt_confirm(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    eprint!("  {} [{}] > ", label, hint);
    io::stderr().flush()?;

    terminal::enable_raw_mode()?;
    let result = loop {
        if let Event::Key(key_event) = event::read()? {
            if key_event.kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            match key_event.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    break Ok(true);
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    break Ok(false);
                }
                KeyCode::Enter => {
                    break Ok(default);
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Err(anyhow::anyhow!("cancelled"));
                }
                _ => continue,
            }
        }
    };
    terminal::disable_raw_mode()?;

    match &result {
        Ok(true) => eprintln!("yes"),
        Ok(false) => eprintln!("no"),
        Err(_) => eprintln!(),
    }

    result
}

/// Prompt for selecting one option from a list using crossterm arrow keys
fn prompt_select(label: &str, options: &[&str]) -> Result<String> {
    eprintln!("  {} (↑↓ to select, Enter to confirm):", label);

    let mut selected: usize = 0;
    print_select_options(options, selected);

    terminal::enable_raw_mode()?;
    let result = loop {
        if let Event::Key(key_event) = event::read()? {
            if key_event.kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if selected > 0 {
                        selected -= 1;
                    }
                    clear_select_lines(options.len());
                    print_select_options(options, selected);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected < options.len() - 1 {
                        selected += 1;
                    }
                    clear_select_lines(options.len());
                    print_select_options(options, selected);
                }
                KeyCode::Enter => {
                    break Ok(options[selected].to_string());
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Err(anyhow::anyhow!("cancelled"));
                }
                _ => continue,
            }
        }
    };
    terminal::disable_raw_mode()?;

    result
}

fn print_select_options(options: &[&str], selected: usize) {
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == selected { "▸" } else { " " };
        eprintln!("    {} {}", marker, opt);
    }
}

fn clear_select_lines(count: usize) {
    // Move cursor up and clear each line
    for _ in 0..count {
        eprint!("\x1b[A\x1b[2K");
    }
}

/// Read a line from stdin (used for text input prompts)
fn read_line() -> Result<String> {
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .context("failed to read input")?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_build_toml_entry_minimal() {
        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };
        let entry = build_toml_entry(&params);
        assert!(entry.contains("[[abbr]]"));
        assert!(entry.contains("keyword = \"g\""));
        assert!(entry.contains("expansion = \"git\""));
        assert!(!entry.contains("global"));
        assert!(!entry.contains("evaluate"));
        assert!(!entry.contains("allow_conflict"));
        assert!(!entry.contains("context"));
    }

    #[test]
    fn test_build_toml_entry_full() {
        let params = AddParams {
            keyword: "main".to_string(),
            expansion: "main --branch".to_string(),
            global: true,
            evaluate: true,
            function: false,
            regex: false,
            command: None,
            allow_conflict: true,
            context_lbuffer: Some("^git (checkout|switch)".to_string()),
            context_rbuffer: Some(".*$".to_string()),
        };
        let entry = build_toml_entry(&params);
        assert!(entry.contains("global = true"));
        assert!(entry.contains("evaluate = true"));
        assert!(entry.contains("allow_conflict = true"));
        assert!(entry.contains("context.lbuffer = \"^git (checkout|switch)\""));
        assert!(entry.contains("context.rbuffer = \".*$\""));
    }

    #[test]
    fn test_build_toml_entry_escaping() {
        let params = AddParams {
            keyword: "gc".to_string(),
            expansion: "git commit -m '{{message}}'".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };
        let entry = build_toml_entry(&params);
        // The toml crate should handle escaping of single quotes
        assert!(entry.contains("keyword = \"gc\""));
        assert!(entry.contains("expansion = \"git commit -m '{{message}}'\""));
    }

    #[test]
    fn test_validate_params_valid() {
        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };
        assert!(validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_empty_keyword() {
        let params = AddParams {
            keyword: "".to_string(),
            expansion: "git".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };
        let err = validate_params(&params).unwrap_err();
        assert!(err.to_string().contains("keyword must not be empty"));
    }

    #[test]
    fn test_validate_params_keyword_with_space() {
        let params = AddParams {
            keyword: "g c".to_string(),
            expansion: "git commit".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };
        let err = validate_params(&params).unwrap_err();
        assert!(err.to_string().contains("must not contain spaces"));
    }

    #[test]
    fn test_validate_params_empty_expansion() {
        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };
        let err = validate_params(&params).unwrap_err();
        assert!(err.to_string().contains("expansion must not be empty"));
    }

    #[test]
    fn test_validate_params_invalid_lbuffer_regex() {
        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: Some("[invalid".to_string()),
            context_rbuffer: None,
        };
        let err = validate_params(&params).unwrap_err();
        assert!(err.to_string().contains("invalid context.lbuffer regex"));
    }

    #[test]
    fn test_validate_params_invalid_rbuffer_regex() {
        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: Some("[invalid".to_string()),
        };
        let err = validate_params(&params).unwrap_err();
        assert!(err.to_string().contains("invalid context.rbuffer regex"));
    }

    #[test]
    fn test_toml_quote_simple() {
        assert_eq!(toml_quote("git"), "\"git\"");
    }

    #[test]
    fn test_toml_quote_with_special_chars() {
        let quoted = toml_quote("git commit -m '{{message}}'");
        assert!(quoted.starts_with('"') || quoted.starts_with('\''));
    }

    #[test]
    fn test_append_to_config_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        // Create a minimal config first
        std::fs::write(&path, "[settings]\n").unwrap();

        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "git".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let mut content = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        assert!(content.contains("[[abbr]]"));
        assert!(content.contains("keyword = \"g\""));
        assert!(content.contains("expansion = \"git\""));

        // Verify the config is still parseable
        config::parse(&content).unwrap();
    }

    #[test]
    fn test_append_to_config_duplicate_keyword() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = r#"[settings]


[[abbr]]
keyword = "g"
expansion = "git"
"#;
        std::fs::write(&path, initial).unwrap();

        let params = AddParams {
            keyword: "g".to_string(),
            expansion: "git status".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        let err = append_to_config(&path, &params).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_append_to_config_same_keyword_different_command_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = r#"[settings]


[[abbr]]
keyword = "co"
expansion = "checkout"
command = "git"
"#;
        std::fs::write(&path, initial).unwrap();

        // Same keyword but different command scope — should succeed
        let params = AddParams {
            keyword: "co".to_string(),
            expansion: "checkout".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: Some("kubectl".to_string()),
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let cfg = config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.abbr.len(), 2);
        assert_eq!(cfg.abbr[0].command, Some("git".to_string()));
        assert_eq!(cfg.abbr[1].command, Some("kubectl".to_string()));
    }

    #[test]
    fn test_append_to_config_same_keyword_command_vs_no_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = r#"[settings]


[[abbr]]
keyword = "co"
expansion = "checkout"
"#;
        std::fs::write(&path, initial).unwrap();

        // Same keyword but one has command scope — should succeed
        let params = AddParams {
            keyword: "co".to_string(),
            expansion: "checkout".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: Some("git".to_string()),
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let cfg = config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.abbr.len(), 2);
    }

    #[test]
    fn test_append_to_config_global_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        std::fs::write(&path, "[settings]\n").unwrap();

        let params = AddParams {
            keyword: "NE".to_string(),
            expansion: "2>/dev/null".to_string(),
            global: true,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("global = true"));

        config::parse(&content).unwrap();
    }

    #[test]
    fn test_append_to_config_context_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        std::fs::write(&path, "[settings]\n").unwrap();

        let params = AddParams {
            keyword: "main".to_string(),
            expansion: "main --branch".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: Some("^git (checkout|switch)".to_string()),
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("context.lbuffer"));

        let config = config::parse(&content).unwrap();
        assert!(config.abbr[0].context.is_some());
    }

    #[test]
    fn test_append_to_config_same_keyword_different_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = r#"[settings]


[[abbr]]
keyword = "main"
expansion = "main --branch"
context.lbuffer = "^git (checkout|switch)"
"#;
        std::fs::write(&path, initial).unwrap();

        // Same keyword but different context.lbuffer — should succeed
        let params = AddParams {
            keyword: "main".to_string(),
            expansion: "main --rebase".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: Some("^git merge".to_string()),
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let cfg = config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.abbr.len(), 2);
    }

    #[test]
    fn test_append_to_config_same_keyword_same_context_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = r#"[settings]


[[abbr]]
keyword = "main"
expansion = "main --branch"
context.lbuffer = "^git (checkout|switch)"
"#;
        std::fs::write(&path, initial).unwrap();

        // Same keyword AND same context — should be rejected as duplicate
        let params = AddParams {
            keyword: "main".to_string(),
            expansion: "main --rebase".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: Some("^git (checkout|switch)".to_string()),
            context_rbuffer: None,
        };

        let err = append_to_config(&path, &params).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_append_to_config_context_vs_no_context_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = r#"[settings]


[[abbr]]
keyword = "main"
expansion = "main --branch"
context.lbuffer = "^git (checkout|switch)"
"#;
        std::fs::write(&path, initial).unwrap();

        // Same keyword but no context — different scope, should succeed
        let params = AddParams {
            keyword: "main".to_string(),
            expansion: "main".to_string(),
            global: false,
            evaluate: false,
            function: false,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        append_to_config(&path, &params).unwrap();

        let cfg = config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.abbr.len(), 2);
    }

    #[test]
    fn test_append_to_config_invalid_entry_does_not_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = "[settings]\n";
        std::fs::write(&path, initial).unwrap();

        // function + evaluate is invalid (mutually exclusive)
        let params = AddParams {
            keyword: "x".to_string(),
            expansion: "y".to_string(),
            global: false,
            evaluate: true,
            function: true,
            regex: false,
            command: None,
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        let err = append_to_config(&path, &params);
        assert!(err.is_err());

        // File should remain unchanged
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, initial);
    }

    #[test]
    fn test_append_to_config_command_and_global_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kort.toml");

        let initial = "[settings]\n";
        std::fs::write(&path, initial).unwrap();

        let params = AddParams {
            keyword: "co".to_string(),
            expansion: "checkout".to_string(),
            global: true,
            evaluate: false,
            function: false,
            regex: false,
            command: Some("git".to_string()),
            allow_conflict: false,
            context_lbuffer: None,
            context_rbuffer: None,
        };

        let err = append_to_config(&path, &params);
        assert!(err.is_err());

        // File should remain unchanged
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, initial);
    }
}
