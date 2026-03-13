use crate::matcher::Matcher;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Cache format version
const CACHE_VERSION: u32 = 5;

/// Cached settings (stored in cache for expand-time access)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedSettings {
    pub remind: bool,
    pub prefixes: Vec<String>,
    pub serve: bool,
}

/// Binary cache
#[derive(Debug, Serialize, Deserialize)]
pub struct CompiledCache {
    pub version: u32,
    pub config_hash: u64,
    pub matcher: Matcher,
    pub settings: CachedSettings,
}

/// Compute hash of config file content
pub fn hash_config(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Write cache to file
pub fn write(
    output_path: &Path,
    matcher: &Matcher,
    settings: &CachedSettings,
    config_path: &Path,
) -> Result<()> {
    let config_content = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file: {}", config_path.display()))?;

    let cache = CompiledCache {
        version: CACHE_VERSION,
        config_hash: hash_config(&config_content),
        matcher: matcher.clone(),
        settings: settings.clone(),
    };

    // Create parent directories
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory: {}", parent.display()))?;
    }

    let encoded = bitcode::serialize(&cache).context("failed to serialize cache")?;
    std::fs::write(output_path, encoded)
        .with_context(|| format!("failed to write cache file: {}", output_path.display()))?;

    Ok(())
}

/// Read cache from file
pub fn read(cache_path: &Path) -> Result<CompiledCache> {
    let data = std::fs::read(cache_path)
        .with_context(|| format!("failed to read cache file: {}", cache_path.display()))?;

    let cache: CompiledCache =
        bitcode::deserialize(&data).context("failed to deserialize cache (may need recompile)")?;

    if cache.version != CACHE_VERSION {
        anyhow::bail!(
            "cache version mismatch (expected: {}, got: {}). Run `abbrs compile` to update.",
            CACHE_VERSION,
            cache.version
        );
    }

    Ok(cache)
}

/// Check cache freshness
/// Compare config content hash with cache hash
pub fn is_fresh(cache: &CompiledCache, config_path: &Path) -> Result<bool> {
    let config_content = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file: {}", config_path.display()))?;

    let current_hash = hash_config(&config_content);
    Ok(cache.config_hash == current_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::Matcher;
    use tempfile::TempDir;

    fn create_test_config(dir: &TempDir) -> std::path::PathBuf {
        let config_path = dir.path().join("abbrs.toml");
        std::fs::write(
            &config_path,
            r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
        )
        .unwrap();
        config_path
    }

    #[test]
    fn test_hash_config_deterministic() {
        let content = "test content";
        assert_eq!(hash_config(content), hash_config(content));
    }

    #[test]
    fn test_hash_config_different_content() {
        assert_ne!(hash_config("content a"), hash_config("content b"));
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("abbrs.cache");

        let matcher = Matcher::new();
        let settings = CachedSettings::default();
        write(&cache_path, &matcher, &settings, &config_path).unwrap();

        let loaded = read(&cache_path).unwrap();
        assert_eq!(loaded.version, CACHE_VERSION);
        assert!(loaded.matcher.regular.is_empty());
        assert!(loaded.matcher.global.is_empty());
        assert!(loaded.matcher.contextual.is_empty());
        assert!(loaded.matcher.command_scoped.is_empty());
        assert!(loaded.matcher.regex_abbrs.is_empty());
        assert!(!loaded.settings.remind);
        assert!(loaded.settings.prefixes.is_empty());
    }

    #[test]
    fn test_write_and_read_roundtrip_with_settings() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("abbrs.cache");

        let matcher = Matcher::new();
        let settings = CachedSettings {
            remind: true,
            prefixes: vec!["sudo".to_string(), "doas".to_string()],
            ..Default::default()
        };
        write(&cache_path, &matcher, &settings, &config_path).unwrap();

        let loaded = read(&cache_path).unwrap();
        assert!(loaded.settings.remind);
        assert_eq!(loaded.settings.prefixes, vec!["sudo", "doas"]);
    }

    #[test]
    fn test_is_fresh_true() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("abbrs.cache");

        let matcher = Matcher::new();
        let settings = CachedSettings::default();
        write(&cache_path, &matcher, &settings, &config_path).unwrap();

        let loaded = read(&cache_path).unwrap();
        assert!(is_fresh(&loaded, &config_path).unwrap());
    }

    #[test]
    fn test_is_fresh_false_after_config_change() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("abbrs.cache");

        let matcher = Matcher::new();
        let settings = CachedSettings::default();
        write(&cache_path, &matcher, &settings, &config_path).unwrap();

        // Modify config file
        std::fs::write(
            &config_path,
            r#"
[[abbr]]
keyword = "gc"
expansion = "git commit"
"#,
        )
        .unwrap();

        let loaded = read(&cache_path).unwrap();
        assert!(!is_fresh(&loaded, &config_path).unwrap());
    }

    #[test]
    fn test_read_nonexistent_cache() {
        let result = read(Path::new("/nonexistent/abbrs.cache"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_corrupted_cache() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("abbrs.cache");
        std::fs::write(&cache_path, b"corrupted data").unwrap();

        let result = read(&cache_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("nested").join("dir").join("abbrs.cache");

        let matcher = Matcher::new();
        let settings = CachedSettings::default();
        let result = write(&cache_path, &matcher, &settings, &config_path);
        assert!(result.is_ok());
        assert!(cache_path.exists());
    }

    #[test]
    fn test_prefix_index_survives_cache_roundtrip() {
        use crate::config::Abbreviation;
        use crate::matcher;

        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("abbrs.cache");

        let abbrs = vec![
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gd".to_string(),
                expansion: "git diff".to_string(),
                ..Default::default()
            },
        ];
        let matcher = matcher::build(&abbrs);

        // prefix_index should be populated before write
        assert!(matcher.prefix_index.contains_key("g"));

        let settings = CachedSettings::default();
        write(&cache_path, &matcher, &settings, &config_path).unwrap();

        let loaded = read(&cache_path).unwrap();

        // prefix_index must survive serialization roundtrip
        let g_candidates = loaded.matcher.prefix_index.get("g").unwrap();
        assert_eq!(g_candidates, &vec!["gc".to_string(), "gd".to_string(), "gp".to_string()]);

        // prefix_candidates should work on deserialized matcher
        let candidates = matcher::prefix_candidates(&loaded.matcher, "g", true, None);
        assert_eq!(candidates.len(), 3);
    }
}
