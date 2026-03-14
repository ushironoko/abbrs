use crate::{cache, config, conflict, matcher};
use anyhow::Result;
use std::path::Path;

/// Compile result
#[derive(Debug)]
pub struct CompileResult {
    pub abbr_count: usize,
}

/// Run full validation pipeline: syntax + duplicates + PATH/builtin conflicts.
/// Returns the parsed config on success.
fn validate_full(config_path: &Path) -> Result<config::Config> {
    // 1. Parse TOML (syntax validation)
    let cfg = config::load(config_path)?;

    // 2. Collect all errors
    let mut all_errors = Vec::new();

    // 3. Detect duplicate keywords
    let dup_report = conflict::detect_duplicates(&cfg.abbr);
    all_errors.extend(dup_report.errors);

    // 4. Scan PATH and detect conflicts
    let path_commands = conflict::scan_path();
    let conflict_report = conflict::detect_conflicts(&cfg.abbr, &path_commands);
    all_errors.extend(conflict_report.errors);

    // 5. Report all errors at once
    if !all_errors.is_empty() {
        let mut message = String::from("Validation errors:\n");
        for err in &all_errors {
            message.push_str(&format!("  ✗ {}\n", err));
        }
        message.push_str("\nHint: set allow_conflict = true to allow individual PATH/builtin conflicts");
        anyhow::bail!(message);
    }

    Ok(cfg)
}

/// Main flow of abbrs compile
pub fn compile(config_path: &Path, output_path: &Path) -> Result<CompileResult> {
    let cfg = validate_full(config_path)?;

    // Build Matcher and write binary cache
    let matcher = matcher::build(&cfg.abbr);
    let settings = cache::CachedSettings {
        remind: cfg.settings.remind,
        prefixes: cfg.settings.prefixes.clone(),
        serve: cfg.settings.serve,
        page_size: cfg.settings.page_size.unwrap_or(0),
    };
    cache::write(output_path, &matcher, &settings, config_path)?;

    Ok(CompileResult {
        abbr_count: cfg.abbr.len(),
    })
}

/// Full validation check (same pipeline as compile, without cache generation)
pub fn check(config_path: &Path) -> Result<usize> {
    let cfg = validate_full(config_path)?;
    Ok(cfg.abbr.len())
}

/// Validate a single abbreviation against PATH commands and builtins.
/// Used by `add` to check conflicts before writing to config.
pub fn check_single_conflict(
    keyword: &str,
    allow_conflict: bool,
) -> Result<()> {
    if allow_conflict {
        return Ok(());
    }

    let abbr = config::Abbreviation {
        keyword: keyword.to_string(),
        expansion: "dummy".to_string(),
        allow_conflict: false,
        ..Default::default()
    };

    let path_commands = conflict::scan_path();
    let report = conflict::detect_conflicts(&[abbr], &path_commands);

    if report.has_errors() {
        let mut message = String::new();
        for err in &report.errors {
            message.push_str(&format!("{}", err));
        }
        message.push_str("\n\nHint: use --allow-conflict to override");
        anyhow::bail!(message);
    }

    Ok(())
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
    fn test_compile_success() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
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
        let cache_path = dir.path().join("abbrs.cache");

        let result = compile(&config_path, &cache_path).unwrap();
        assert_eq!(result.abbr_count, 2);
        assert!(cache_path.exists());
    }

    #[test]
    fn test_compile_builtin_conflict_error() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "cd"
expansion = "custom_cd"
"#,
        );
        let cache_path = dir.path().join("abbrs.cache");

        let result = compile(&config_path, &cache_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("cd"));
        assert!(err_msg.contains("builtin"));
    }

    #[test]
    fn test_compile_allow_conflict() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "cd"
expansion = "custom_cd"
allow_conflict = true
"#,
        );
        let cache_path = dir.path().join("abbrs.cache");

        let result = compile(&config_path, &cache_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_valid_config() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );
        let count = check(&config_path).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_check_invalid_config() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = ""
expansion = "git"
"#,
        );
        let result = check(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_detects_builtin_conflict() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "cd"
expansion = "custom_cd"
"#,
        );
        let result = check(&config_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("cd"));
        assert!(err_msg.contains("builtin"));
    }

    #[test]
    fn test_check_detects_duplicate() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "p"
expansion = "pnpm"

[[abbr]]
keyword = "p"
expansion = "prune"
"#,
        );
        let result = check(&config_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("\"p\""));
        assert!(err_msg.contains("multiple times"));
    }

    #[test]
    fn test_check_reports_all_errors_at_once() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[[abbr]]
keyword = "p"
expansion = "pnpm"

[[abbr]]
keyword = "p"
expansion = "prune"

[[abbr]]
keyword = "cd"
expansion = "custom_cd"
"#,
        );
        let result = check(&config_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Both duplicate and builtin conflict should be reported
        assert!(err_msg.contains("\"p\""));
        assert!(err_msg.contains("cd"));
    }

    #[test]
    fn test_compile_with_settings() {
        let dir = TempDir::new().unwrap();
        let config_path = write_config(
            &dir,
            r#"
[settings]
prefixes = ["sudo", "doas"]
remind = true

[[abbr]]
keyword = "g"
expansion = "git"
"#,
        );
        let cache_path = dir.path().join("abbrs.cache");

        let result = compile(&config_path, &cache_path).unwrap();
        assert_eq!(result.abbr_count, 1);

        // Verify settings are stored in cache
        let loaded = cache::read(&cache_path).unwrap();
        assert!(loaded.settings.remind);
        assert_eq!(loaded.settings.prefixes, vec!["sudo", "doas"]);
    }

    #[test]
    fn test_check_single_conflict_builtin() {
        let result = check_single_conflict("cd", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("builtin"));
    }

    #[test]
    fn test_check_single_conflict_allowed() {
        let result = check_single_conflict("cd", true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_single_conflict_no_conflict() {
        let result = check_single_conflict("zzz_nonexistent", false);
        assert!(result.is_ok());
    }
}
