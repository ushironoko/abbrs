use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn abbrs_cmd() -> Command {
    cargo_bin_cmd!("abbrs")
}

fn setup_compiled(dir: &TempDir, config_content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_path = dir.path().join("abbrs.toml");
    std::fs::write(&config_path, config_content).unwrap();

    // Compile
    cargo_bin_cmd!("abbrs")
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();

    let cache_path = dir.path().join("cache").join("abbrs").join("abbrs.cache");
    (config_path, cache_path)
}

#[test]
fn test_expand_regular() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("success\n"))
        .stdout(predicate::str::contains("git"));
}

#[test]
fn test_expand_no_match() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "unknown",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("no_match"));
}

#[test]
fn test_expand_global() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true
"#,
    );

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "echo hello NE",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("success\n"))
        .stdout(predicate::str::contains("2>/dev/null"));
}

#[test]
fn test_expand_with_placeholder() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "gc"
expansion = "git commit -m '{{message}}'"
"#,
    );

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "gc",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("success\n"))
        .stdout(predicate::str::contains("git commit -m ''"));
}

#[test]
fn test_expand_stale_cache() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    // Modify config to make cache stale
    std::fs::write(
        &config_path,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit"
"#,
    )
    .unwrap();

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("stale_cache"));
}

#[test]
fn test_expand_missing_cache() {
    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            "/nonexistent/abbrs.cache",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("stale_cache"));
}

#[test]
fn test_expand_both_config_and_cache_absent() {
    // When both config and cache are missing, expand should return no_match
    // (not stale_cache) to avoid triggering repeated recompile attempts.
    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            "/nonexistent/abbrs.cache",
            "--config",
            "/nonexistent/abbrs.toml",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("no_match"));
}

#[test]
fn test_next_placeholder() {
    abbrs_cmd()
        .args([
            "next-placeholder",
            "--lbuffer",
            "git commit -m '",
            "--rbuffer",
            "' --author='{{author}}'",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("success\n"));
}

#[test]
fn test_next_placeholder_none() {
    abbrs_cmd()
        .args([
            "next-placeholder",
            "--lbuffer",
            "no placeholder",
            "--rbuffer",
            "",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("no_placeholder"));
}

#[test]
fn test_expand_prefix_candidates() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "gc"
expansion = "git commit"

[[abbr]]
keyword = "gp"
expansion = "git push"

[[abbr]]
keyword = "gd"
expansion = "git diff"
"#,
    );

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("candidates\n"))
        .stdout(predicate::str::contains("3\n0\n"))
        .stdout(predicate::str::contains("gc\tgit commit"))
        .stdout(predicate::str::contains("gp\tgit push"))
        .stdout(predicate::str::contains("gd\tgit diff"));
}

#[test]
fn test_expand_exact_match_over_prefix() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit"

[[abbr]]
keyword = "gp"
expansion = "git push"
"#,
    );

    // "g" has exact match → should return success, not candidates
    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("success\n"))
        .stdout(predicate::str::contains("git"));
}

#[test]
fn test_expand_prefix_candidates_with_page_size() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[settings]
page_size = 3

[[abbr]]
keyword = "gc"
expansion = "git commit"

[[abbr]]
keyword = "gp"
expansion = "git push"

[[abbr]]
keyword = "gd"
expansion = "git diff"
"#,
    );

    abbrs_cmd()
        .args([
            "expand",
            "--lbuffer",
            "g",
            "--rbuffer",
            "",
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("candidates\n"))
        .stdout(predicate::str::contains("3\n3\n"))
        .stdout(predicate::str::contains("gc\tgit commit"))
        .stdout(predicate::str::contains("gp\tgit push"))
        .stdout(predicate::str::contains("gd\tgit diff"));
}
