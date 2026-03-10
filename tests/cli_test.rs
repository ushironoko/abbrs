use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn kort_cmd() -> Command {
    cargo_bin_cmd!("kort")
}

fn create_config(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let config_path = dir.path().join("kort.toml");
    std::fs::write(&config_path, content).unwrap();
    config_path
}

#[test]
fn test_help() {
    kort_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("abbreviation"));
}

#[test]
fn test_version() {
    kort_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("kort"));
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

    kort_cmd()
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

    kort_cmd()
        .args(["check", "--config", config_path.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_check_missing_config() {
    kort_cmd()
        .args(["check", "--config", "/nonexistent/kort.toml"])
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

    kort_cmd()
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

"#,
    );

    kort_cmd()
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

"#,
    );

    kort_cmd()
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

"#,
    );

    kort_cmd()
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

    kort_cmd()
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
    let config_path = create_config(&dir, "[settings]\n");

    kort_cmd()
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
    kort_cmd()
        .args(["add", "g", "git", "--config", "/nonexistent/kort.toml"])
        .assert()
        .failure();
}

#[test]
fn test_init_zsh_outputs_shell_script() {
    kort_cmd()
        .args(["init", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("kort-expand-space"))
        .stdout(predicate::str::contains("zle -N"))
        .stdout(predicate::str::contains("bindkey"));
}

#[test]
fn test_init_config_creates_template() {
    let dir = TempDir::new().unwrap();

    kort_cmd()
        .args(["init", "config"])
        .env("XDG_CONFIG_HOME", dir.path())
        .assert()
        .success();

    let xdg_config_path = dir.path().join("kort").join("kort.toml");
    assert!(xdg_config_path.exists(), "config file should be created at XDG path");
    let content = std::fs::read_to_string(&xdg_config_path).unwrap();
    assert!(content.contains("[settings]"), "config template should contain [settings]");
}

#[test]
fn test_init_without_subcommand_shows_help() {
    kort_cmd()
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("zsh"))
        .stderr(predicate::str::contains("config"));
}

#[test]
fn test_list_empty() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(&dir, "");

    kort_cmd()
        .args(["list", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("no abbreviations registered"));
}
