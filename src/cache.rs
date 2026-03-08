use crate::matcher::Matcher;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Cache format version
const CACHE_VERSION: u32 = 1;

/// Binary cache
#[derive(Debug, Serialize, Deserialize)]
pub struct CompiledCache {
    pub version: u32,
    pub config_hash: u64,
    pub matcher: Matcher,
}

/// Compute hash of config file content
pub fn hash_config(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Write cache to file
pub fn write(output_path: &Path, matcher: &Matcher, config_path: &Path) -> Result<()> {
    let config_content = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file: {}", config_path.display()))?;

    let cache = CompiledCache {
        version: CACHE_VERSION,
        config_hash: hash_config(&config_content),
        matcher: matcher.clone(),
    };

    // Create parent directories
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory: {}", parent.display()))?;
    }

    let encoded = bincode::serialize(&cache).context("failed to serialize cache")?;
    std::fs::write(output_path, encoded)
        .with_context(|| format!("failed to write cache file: {}", output_path.display()))?;

    Ok(())
}

/// Read cache from file
pub fn read(cache_path: &Path) -> Result<CompiledCache> {
    let data = std::fs::read(cache_path)
        .with_context(|| format!("failed to read cache file: {}", cache_path.display()))?;

    let cache: CompiledCache =
        bincode::deserialize(&data).context("failed to deserialize cache")?;

    if cache.version != CACHE_VERSION {
        anyhow::bail!(
            "cache version mismatch (expected: {}, got: {})",
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
        let config_path = dir.path().join("brv.toml");
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
        let cache_path = dir.path().join("brv.cache");

        let matcher = Matcher::new();
        write(&cache_path, &matcher, &config_path).unwrap();

        let loaded = read(&cache_path).unwrap();
        assert_eq!(loaded.version, CACHE_VERSION);
        assert!(loaded.matcher.regular.is_empty());
        assert!(loaded.matcher.global.is_empty());
        assert!(loaded.matcher.contextual.is_empty());
    }

    #[test]
    fn test_is_fresh_true() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("brv.cache");

        let matcher = Matcher::new();
        write(&cache_path, &matcher, &config_path).unwrap();

        let loaded = read(&cache_path).unwrap();
        assert!(is_fresh(&loaded, &config_path).unwrap());
    }

    #[test]
    fn test_is_fresh_false_after_config_change() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("brv.cache");

        let matcher = Matcher::new();
        write(&cache_path, &matcher, &config_path).unwrap();

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
        let result = read(Path::new("/nonexistent/brv.cache"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_corrupted_cache() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("brv.cache");
        std::fs::write(&cache_path, b"corrupted data").unwrap();

        let result = read(&cache_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let config_path = create_test_config(&dir);
        let cache_path = dir.path().join("nested").join("dir").join("brv.cache");

        let matcher = Matcher::new();
        let result = write(&cache_path, &matcher, &config_path);
        assert!(result.is_ok());
        assert!(cache_path.exists());
    }
}
