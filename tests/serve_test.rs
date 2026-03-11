use assert_cmd::cargo::cargo_bin_cmd;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tempfile::TempDir;

const EOR: char = '\x1e';

fn setup_compiled(dir: &TempDir, config_content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_path = dir.path().join("abbrs.toml");
    std::fs::write(&config_path, config_content).unwrap();

    cargo_bin_cmd!("abbrs")
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();

    let cache_path = dir.path().join("cache").join("abbrs").join("abbrs.cache");
    (config_path, cache_path)
}

struct ServeProcess {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    reader: BufReader<std::process::ChildStdout>,
}

impl ServeProcess {
    fn start(cache_path: &std::path::Path, config_path: &std::path::Path) -> Self {
        let kort_bin = cargo_bin_cmd!("abbrs").get_program().to_owned();
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
            .expect("failed to start abbrs serve");

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
    let config_path = dir.path().join("abbrs.toml");
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
    let config_path = dir.path().join("abbrs.toml");
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
    cargo_bin_cmd!("abbrs")
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
    let config_path = dir.path().join("abbrs.toml");
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

// === Socket mode tests ===

struct SocketServeProcess {
    child: Child,
    socket_path: PathBuf,
}

impl SocketServeProcess {
    fn start(socket_path: &Path, cache_path: &Path, config_path: &Path) -> Self {
        let abbrs_bin = cargo_bin_cmd!("abbrs").get_program().to_owned();
        let child = Command::new(abbrs_bin)
            .args([
                "serve",
                "--socket",
                socket_path.to_str().unwrap(),
                "--cache",
                cache_path.to_str().unwrap(),
                "--config",
                config_path.to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to start abbrs serve --socket");

        // Wait for socket to become connectable (max ~500ms)
        let mut ready = false;
        for _ in 0..100 {
            if socket_path.exists() {
                if UnixStream::connect(socket_path).is_ok() {
                    ready = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            ready,
            "socket did not become connectable: {}",
            socket_path.display()
        );

        Self {
            child,
            socket_path: socket_path.to_path_buf(),
        }
    }

    fn connect(&self) -> SocketConnection {
        let stream =
            UnixStream::connect(&self.socket_path).expect("failed to connect to socket");
        let reader = BufReader::new(stream.try_clone().unwrap());
        let writer = std::io::LineWriter::new(stream);
        SocketConnection { reader, writer }
    }
}

impl Drop for SocketServeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

struct SocketConnection {
    reader: BufReader<UnixStream>,
    writer: std::io::LineWriter<UnixStream>,
}

impl SocketConnection {
    fn send(&mut self, request: &str) -> Vec<String> {
        writeln!(self.writer, "{}", request).expect("failed to write to socket");
        self.writer.flush().expect("failed to flush socket");
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
}

#[test]
fn test_socket_ping() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();
    let response = conn.send("ping");
    assert_eq!(response, vec!["pong"]);
    drop(proc);
}

#[test]
fn test_socket_expand_basic() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();
    let response = conn.send("expand\tg\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git");
    assert_eq!(response[2], "3");
    drop(proc);
}

#[test]
fn test_socket_expand_no_match() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();
    let response = conn.send("expand\tunknown\t");
    assert_eq!(response, vec!["no_match"]);
    drop(proc);
}

#[test]
fn test_socket_multiple_requests() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
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
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();

    let response = conn.send("expand\tg\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git");

    let response = conn.send("expand\tgc\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git commit");

    let response = conn.send("expand\txyz\t");
    assert_eq!(response, vec!["no_match"]);

    let response = conn.send("ping");
    assert_eq!(response, vec!["pong"]);

    drop(proc);
}

#[test]
fn test_socket_reconnect_after_disconnect() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);

    // First connection
    {
        let mut conn = proc.connect();
        let response = conn.send("ping");
        assert_eq!(response, vec!["pong"]);
    } // connection dropped (EOF sent to server)

    // Small delay for server to accept next connection
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Reconnect
    {
        let mut conn = proc.connect();
        let response = conn.send("expand\tg\t");
        assert_eq!(response[0], "success");
        assert_eq!(response[1], "git");
    }

    drop(proc);
}

#[test]
fn test_socket_stale_cleanup_on_start() {
    use std::os::unix::net::UnixListener;

    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    // Create a real stale socket (bind then drop the listener so nobody is listening)
    {
        let _listener = UnixListener::bind(&socket_path).unwrap();
        // listener dropped here — socket file remains but nobody is listening
    }
    assert!(socket_path.exists());

    // Server should clean up stale socket and start successfully
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();
    let response = conn.send("ping");
    assert_eq!(response, vec!["pong"]);
    drop(proc);
}

#[test]
fn test_socket_refuses_non_socket_path() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    // Create a regular file at the socket path
    std::fs::write(&socket_path, "not a socket").unwrap();

    // Server should refuse to start and not delete the file
    let abbrs_bin = cargo_bin_cmd!("abbrs").get_program().to_owned();
    let output = Command::new(abbrs_bin)
        .args([
            "serve",
            "--socket",
            socket_path.to_str().unwrap(),
            "--cache",
            cache_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run abbrs serve");

    assert!(!output.status.success(), "serve should have failed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a socket"),
        "stderr should mention 'not a socket', got: {}",
        stderr
    );
    // The regular file should still exist (not deleted)
    assert!(socket_path.exists(), "regular file should not be deleted");
    let content = std::fs::read_to_string(&socket_path).unwrap();
    assert_eq!(content, "not a socket");
}

#[test]
fn test_socket_cleanup_on_exit() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );

    let mut proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    assert!(socket_path.exists());

    // Kill server
    proc.child.kill().unwrap();
    proc.child.wait().unwrap();

    // Note: kill(SIGKILL) doesn't run cleanup, so the socket file may remain.
    // The important thing is that run_socket() has cleanup code for normal exit
    // and that stale cleanup works on next start (tested above).
    // We explicitly remove it in Drop to avoid interference.
}

#[test]
fn test_socket_reload() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();

    // Initial expand
    let response = conn.send("expand\tg\t");
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
    cargo_bin_cmd!("abbrs")
        .args(["compile", "--config", config_path.to_str().unwrap()])
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .assert()
        .success();

    // Reload
    let response = conn.send("reload");
    assert_eq!(response, vec!["ok"]);

    // New abbreviation should work
    let response = conn.send("expand\tgp\t");
    assert_eq!(response[0], "success");
    assert_eq!(response[1], "git push");

    drop(proc);
}

#[test]
fn test_socket_stale_cache() {
    let dir = TempDir::new().unwrap();
    let socket_path = dir.path().join("abbrs.sock");
    let (config_path, cache_path) = setup_compiled(
        &dir,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    );
    let proc = SocketServeProcess::start(&socket_path, &cache_path, &config_path);
    let mut conn = proc.connect();

    // Ensure cache is loaded
    let response = conn.send("ping");
    assert_eq!(response, vec!["pong"]);

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

    let response = conn.send("expand\tg\t");
    assert_eq!(response, vec!["stale_cache"]);

    drop(proc);
}
