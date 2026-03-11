use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn abbrs_cmd() -> Command {
    cargo_bin_cmd!("abbrs")
}

fn create_config(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let config_path = dir.path().join("abbrs.toml");
    std::fs::write(&config_path, content).unwrap();
    config_path
}

#[test]
fn test_help() {
    abbrs_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("abbreviation"));
}

#[test]
fn test_version() {
    abbrs_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("abbrs"));
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

    abbrs_cmd()
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

    abbrs_cmd()
        .args(["check", "--config", config_path.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_check_missing_config() {
    abbrs_cmd()
        .args(["check", "--config", "/nonexistent/abbrs.toml"])
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

    abbrs_cmd()
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

    abbrs_cmd()
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

    abbrs_cmd()
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

    abbrs_cmd()
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

    abbrs_cmd()
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

    abbrs_cmd()
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
    abbrs_cmd()
        .args(["add", "g", "git", "--config", "/nonexistent/abbrs.toml"])
        .assert()
        .failure();
}

#[test]
fn test_init_zsh_outputs_shell_script() {
    abbrs_cmd()
        .args(["init", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("abbrs-expand-space"))
        .stdout(predicate::str::contains("zle -N"))
        .stdout(predicate::str::contains("bindkey"))
        // Candidate cycling: hook widget, autoload, and add-zle-hook-widget registration
        .stdout(predicate::str::contains("_abbrs_check_cycling"))
        .stdout(predicate::str::contains("zle -N _abbrs_check_cycling"))
        .stdout(predicate::str::contains(
            "autoload -Uz +X add-zle-hook-widget",
        ))
        .stdout(predicate::str::contains(
            "add-zle-hook-widget line-pre-redraw _abbrs_check_cycling",
        ))
        // Candidate cycling widgets
        .stdout(predicate::str::contains("abbrs-next-placeholder"))
        .stdout(predicate::str::contains("abbrs-literal-space"));
}

/// Regression test: verify that `abbrs init zsh` autoloads `add-zle-hook-widget`
/// and uses it for hook registration in a clean zsh session (`zsh -f`).
#[test]
fn test_init_zsh_hook_chaining_in_clean_shell() {
    // Skip if zsh is not available
    let zsh_check = std::process::Command::new("zsh")
        .args(["--version"])
        .output();
    if zsh_check.is_err() || !zsh_check.unwrap().status.success() {
        eprintln!("skipping: zsh not available");
        return;
    }

    let abbrs_bin = cargo_bin_cmd!("abbrs").get_program().to_owned();
    let abbrs_path = abbrs_bin.to_str().unwrap();

    // Source the init script in zsh -f (no user rc files) and check that
    // add-zle-hook-widget was autoloaded and used (i.e. the direct
    // zle -N zle-line-pre-redraw fallback was NOT taken).
    //
    // We probe this by:
    //   1. Checking that `add-zle-hook-widget` is a loaded function after sourcing.
    //   2. Checking that the direct fallback bind (`zle -N zle-line-pre-redraw _abbrs_check_cycling`)
    //      was NOT called. Note: `add-zle-hook-widget` itself registers a dispatcher widget
    //      (`zle -N zle-line-pre-redraw azhw:zle-line-pre-redraw`), so we match the exact
    //      fallback signature to distinguish the two paths.
    let test_script = format!(
        r#"
        # Source the init script (stub out zle/bindkey since we're non-interactive)
        zle() {{
            _ZLE_CALLS+=("$*")
        }}
        bindkey() {{ : }}

        eval "$("{abbrs}" init zsh)"

        # After sourcing, add-zle-hook-widget must be a function
        if (( $+functions[add-zle-hook-widget] )); then
            echo "AUTOLOAD_OK"
        else
            echo "AUTOLOAD_MISSING"
        fi

        # The fallback path registers "zle -N zle-line-pre-redraw _abbrs_check_cycling".
        # add-zle-hook-widget itself also calls "zle -N zle-line-pre-redraw azhw:..."
        # so we must match the exact fallback signature to distinguish the two paths.
        local found_fallback=0
        for call in "${{_ZLE_CALLS[@]}}"; do
            if [[ "$call" == *"-N zle-line-pre-redraw _abbrs_check_cycling"* ]]; then
                found_fallback=1
            fi
        done
        if (( found_fallback )); then
            echo "FALLBACK_TAKEN"
        else
            echo "HOOK_CHAINED"
        fi
        "#,
        abbrs = abbrs_path,
    );

    let output = std::process::Command::new("zsh")
        .args(["-f", "-c", &test_script])
        .output()
        .expect("failed to run zsh");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("AUTOLOAD_OK"),
        "add-zle-hook-widget was not autoloaded in clean zsh. stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("HOOK_CHAINED"),
        "fallback path was taken instead of add-zle-hook-widget. stdout: {stdout}, stderr: {stderr}"
    );
}

#[test]
fn test_init_config_creates_template() {
    let dir = TempDir::new().unwrap();

    abbrs_cmd()
        .args(["init", "config"])
        .env("XDG_CONFIG_HOME", dir.path())
        .assert()
        .success();

    let xdg_config_path = dir.path().join("abbrs").join("abbrs.toml");
    assert!(xdg_config_path.exists(), "config file should be created at XDG path");
    let content = std::fs::read_to_string(&xdg_config_path).unwrap();
    assert!(content.contains("[settings]"), "config template should contain [settings]");
}

#[test]
fn test_init_without_subcommand_shows_help() {
    abbrs_cmd()
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

    abbrs_cmd()
        .args(["list", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("no abbreviations registered"));
}

#[test]
fn test_list_keywords_output_format() {
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

[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true
"#,
    );

    abbrs_cmd()
        .args(["_list-keywords", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("g:git"))
        .stdout(predicate::str::contains("gc:git commit"))
        .stdout(predicate::str::contains("NE:2>/dev/null"));
}

#[test]
fn test_list_keywords_escapes_colons() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(
        &dir,
        r#"
[[abbr]]
keyword = "g:co"
expansion = "git checkout"
allow_conflict = true

[[abbr]]
keyword = "ns"
expansion = "npm run start:dev"
"#,
    );

    abbrs_cmd()
        .args(["_list-keywords", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        // Colons in keyword and expansion must be escaped for zsh _describe
        .stdout(predicate::str::contains("g\\:co:git checkout"))
        .stdout(predicate::str::contains("ns:npm run start\\:dev"));
}

#[test]
fn test_list_keywords_empty_config() {
    let dir = TempDir::new().unwrap();
    let config_path = create_config(&dir, "");

    abbrs_cmd()
        .args(["_list-keywords", "--config", config_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn test_list_keywords_hidden_from_help() {
    abbrs_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("_list-keywords").not());
}
