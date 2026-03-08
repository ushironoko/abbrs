use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn brv_cmd() -> Command {
    Command::cargo_bin("brv").unwrap()
}

fn create_config(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let config_path = dir.path().join("brv.toml");
    std::fs::write(&config_path, content).unwrap();
    config_path
}

#[test]
fn test_compile_success() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
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

    brv_cmd()
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success()
        .stderr(predicate::str::contains("compiled 2 abbreviation(s)"));
}

#[test]
fn test_compile_builtin_conflict() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"
[[abbr]]
keyword = "cd"
expansion = "custom_cd"
"#,
    );

    brv_cmd()
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("builtin"));
}

#[test]
fn test_compile_allow_conflict() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"
[[abbr]]
keyword = "cd"
expansion = "custom_cd"
allow_conflict = true
"#,
    );

    brv_cmd()
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();
}

#[test]
fn test_compile_creates_cache() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    brv_cmd()
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();

    // Verify cache file was generated
    let cache_dir = dir.path().join("cache").join("brv");
    assert!(cache_dir.join("brv.cache").exists());
}

#[test]
fn test_compile_missing_config() {
    brv_cmd()
        .args(["compile", "--config", "/nonexistent/brv.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}
