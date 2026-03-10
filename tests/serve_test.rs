use assert_cmd::cargo::cargo_bin_cmd;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tempfile::TempDir;

const EOR: char = '\x1e';

fn setup_compiled(dir: &TempDir, config_content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_path = dir.path().join("kort.toml");
    std::fs::write(&config_path, config_content).unwrap();

    cargo_bin_cmd!("kort")
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();

    let cache_path = dir.path().join("cache").join("kort").join("kort.cache");
    (config_path, cache_path)
}

struct ServeProcess {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    reader: BufReader<std::process::ChildStdout>,
}

impl ServeProcess {
    fn start(cache_path: &std::path::Path, config_path: &std::path::Path) -> Self {
        let kort_bin = cargo_bin_cmd!("kort").get_program().to_owned();
        let mut child = Command::new(kort_bin)
            .args([
                "serve",
                "--cache",
                cache_path.to_str().unwrap(),
                "--config",
                config_path.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start kort serve");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        Self {
            child,
            stdin: Some(stdin),
            reader,
        }
    }

    fn send(&mut self, request: &str) -> Vec<String> {
        let stdin = self.stdin.as_mut().expect("stdin already closed");
        writeln!(stdin, "{}", request).expect("failed to write to stdin");
        stdin.flush().expect("failed to flush stdin");
        self.read_response()
    }

    fn read_response(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).expect("failed to read line");
            if n == 0 {
                break; // EOF
            }
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            if trimmed.starts_with(EOR) {
                break;
            }
            lines.push(trimmed.to_string());
        }
        lines
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn close(mut self) {
        self.stdin.take();
        let _ = self.child.wait();
    }
}

impl Drop for ServeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn test_serve_ping() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("ping");
    assert_eq!(response, vec!["pong"]);
    proc.close();
}

#[test]
fn test_serve_expand_basic() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("expand\tg\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git");
    assert_eq!(response[2], "3");
    proc.close();
}

#[test]
fn test_serve_expand_no_match() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("expand\tunknown\t");
    assert_eq!(response, vec!["no_match"]);
    proc.close();
}

#[test]
fn test_serve_multiple_requests() {
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
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);

    // First request
    let response = proc.send("expand\tg\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git");

    // Second request
    let response = proc.send("expand\tgc\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git commit");

    // Third request — no match
    let response = proc.send("expand\txyz\t");
    assert_eq!(response, vec!["no_match"]);

    // Ping still works
    let response = proc.send("ping");
    assert_eq!(response, vec!["pong"]);

    proc.close();
}

#[test]
fn test_serve_placeholder() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("kort.toml");
    std::fs::write(&config_path, "").unwrap();
    let cache_path = dir.path().join("nonexistent.cache");

    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("placeholder\tgit commit -m '\t' --author='{{author}}'");
    assert_eq!(response[0], "success");
    assert!(response[1].contains("git commit -m '' --author=''"));
    proc.close();
}

#[test]
fn test_serve_placeholder_none() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("kort.toml");
    std::fs::write(&config_path, "").unwrap();
    let cache_path = dir.path().join("nonexistent.cache");

    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("placeholder\tno placeholder here\t");
    assert_eq!(response, vec!["no_placeholder"]);
    proc.close();
}

#[test]
fn test_serve_remind() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[settings]
remind = true

[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("remind\tgit push");
    assert_eq!(response.len(), 1);
    assert!(response[0].contains("could have used"));
    assert!(response[0].contains("\"g\""));
    proc.close();
}

#[test]
fn test_serve_remind_no_match() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[settings]
remind = true

[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("remind\techo hello");
    // No reminder → empty response (just EOR)
    assert!(response.is_empty());
    proc.close();
}

#[test]
fn test_serve_reload() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);

    // Initial expand works
    let response = proc.send("expand\tg\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git");

    // Update config and recompile
    let new_config = r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gp"
expansion = "git push"
"#;
    std::fs::write(&config_path, new_config).unwrap();
    cargo_bin_cmd!("kort")
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();

    // Reload
    let response = proc.send("reload");
    assert_eq!(response, vec!["ok"]);

    // New abbreviation should work
    let response = proc.send("expand\tgp\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git push");

    proc.close();
}

#[test]
fn test_serve_stale_cache() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);

    // Ensure process is ready and cache is loaded with original mtime
    let response = proc.send("ping");
    assert_eq!(response, vec!["pong"]);

    // Ensure mtime differs — use 1s for portability across filesystems
    // (e.g., older HFS+ has 1-second mtime granularity)
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Modify config without recompiling → stale
    std::fs::write(
        &config_path,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "new"
expansion = "new_command"
"#,
    )
    .unwrap();

    let response = proc.send("expand\tg\t");
    assert_eq!(response, vec!["stale_cache"]);

    proc.close();
}

#[test]
fn test_serve_eof_shutdown() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);

    // Close stdin → should cause clean exit
    proc.close_stdin();
    let status = proc.child.wait().expect("failed to wait for process");
    assert!(status.success());
}

#[test]
fn test_serve_unknown_command() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("badcommand");
    assert_eq!(response.len(), 1);
    assert!(response[0].starts_with("error\t"));
    assert!(response[0].contains("unknown command"));
    proc.close();
}

#[test]
fn test_serve_malformed_request() {
    let dir = TempDir::new().unwrap();
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let mut proc = ServeProcess::start(&cache_path, &config_path);
    let response = proc.send("expand\tonly_lbuffer");
    assert_eq!(response.len(), 1);
    assert!(response[0].starts_with("error\t"));

    // Process should still be alive
    let response = proc.send("ping");
    assert_eq!(response, vec!["pong"]);

    proc.close();
}

#[test]
fn test_serve_no_cache_file() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("kort.toml");
    std::fs::write(&config_path, r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#).unwrap();
    let cache_path = dir.path().join("nonexistent.cache");

    let mut proc = ServeProcess::start(&cache_path, &config_path);

    // No cache → stale_cache
    let response = proc.send("expand\tg\t");
    assert_eq!(response, vec!["stale_cache"]);

    // Ping still works
    let response = proc.send("ping");
    assert_eq!(response, vec!["pong"]);

    proc.close();
}
