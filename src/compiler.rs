use crate::{cache, config, conflict, matcher};
use anyhow::Result;
use std::path::Path;

/// Compile result
#[derive(Debug)]
pub struct CompileResult {
    pub abbr_count: usize,
}

/// Main flow of kort compile
pub fn compile(config_path: &Path, output_path: &Path) -> Result<CompileResult> {
    // 1. Parse TOML
    let cfg = config::load(config_path)?;

    // 2. Scan PATH
    let path_commands = conflict::scan_path();

    // 3. Detect conflicts
    let report = conflict::detect_conflicts(&cfg.abbr, &path_commands);

    // 4. Fail compilation if there are errors
    if report.has_errors() {
        let mut message = String::from("Conflicts detected:\n");
        for err in &report.errors {
            message.push_str(&format!("  ✗ {}\n", err));
        }
        message.push_str("\nHint: set allow_conflict = true to allow individual conflicts");
        anyhow::bail!(message);
    }

    // 5. Build Matcher and write binary cache
    let matcher = matcher::build(&cfg.abbr);
    let settings = cache::CachedSettings {
        remind: cfg.settings.remind,
        prefixes: cfg.settings.prefixes.clone(),
    };
    cache::write(output_path, &matcher, &settings, config_path)?;

    Ok(CompileResult {
        abbr_count: cfg.abbr.len(),
    })
}

/// Syntax check only (no compilation)
pub fn check(config_path: &Path) -> Result<usize> {
    let cfg = config::load(config_path)?;
    Ok(cfg.abbr.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_config(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let config_path = dir.path().join("kort.toml");
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
        let cache_path = dir.path().join("kort.cache");

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
        let cache_path = dir.path().join("kort.cache");

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
        let cache_path = dir.path().join("kort.cache");

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
        let cache_path = dir.path().join("kort.cache");

        let result = compile(&config_path, &cache_path).unwrap();
        assert_eq!(result.abbr_count, 1);

        // Verify settings are stored in cache
        let loaded = cache::read(&cache_path).unwrap();
        assert!(loaded.settings.remind);
        assert_eq!(loaded.settings.prefixes, vec!["sudo", "doas"]);
    }
}
