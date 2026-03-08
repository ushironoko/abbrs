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
fn test_help() {
    brv_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("abbreviation"));
}

#[test]
fn test_version() {
    brv_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("brv"));
}

#[test]
fn test_check_valid_config() {
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
        .args(["check", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("config is valid"));
}

#[test]
fn test_check_invalid_config() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"
[[abbr]]
keyword = ""
expansion = "git"
"#,
    );

    brv_cmd()
        .args(["check", "--config", config_path.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_check_missing_config() {
    brv_cmd()
        .args(["check", "--config", "/nonexistent/brv.toml"])
        .assert()
        .failure();
}

#[test]
fn test_list() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
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

    brv_cmd()
        .args(["list", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("g"))
        .stdout(predicate::str::contains("NE"))
        .stdout(predicate::str::contains("global"))
        .stdout(predicate::str::contains("Total: 2"));
}

#[test]
fn test_add_with_args() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"[settings]
strict = false
"#,
    );

    brv_cmd()
        .args([
            "add",
            "g",
            "git",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("added: g → git"));

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("keyword = \"g\""));
    assert!(content.contains("expansion = \"git\""));
}

#[test]
fn test_add_with_global_flag() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"[settings]
strict = false
"#,
    );

    brv_cmd()
        .args([
            "add",
            "NE",
            "2>/dev/null",
            "--global",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("global = true"));
}

#[test]
fn test_add_with_context() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"[settings]
strict = false
"#,
    );

    brv_cmd()
        .args([
            "add",
            "main",
            "main --branch",
            "--context-lbuffer",
            "^git (checkout|switch)",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("context.lbuffer"));
}

#[test]
fn test_add_duplicate_keyword_error() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    brv_cmd()
        .args([
            "add",
            "g",
            "git status",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_add_missing_expansion_error() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(&dir, "[settings]\nstrict = false\n");

    brv_cmd()
        .args([
            "add",
            "g",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn test_add_missing_config() {
    brv_cmd()
        .args(["add", "g", "git", "--config", "/nonexistent/brv.toml"])
        .assert()
        .failure();
}

#[test]
fn test_list_empty() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(&dir, "");

    brv_cmd()
        .args(["list", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("no abbreviations registered"));
}
