use crate::cache::{self, CompiledCache};
use crate::context::RegexCache;
use crate::expand::{self, ExpandInput, ExpandResult};
use crate::history::{self, HistoryEntry};
use crate::output::{CandidateEntry, ExpandOutput, PlaceholderOutput};
use crate::placeholder;
use anyhow::Result;
use std::io::{BufRead, BufReader, LineWriter, Write};
use std::path::PathBuf;
use std::time::SystemTime;

const EOR: &str = "\x1e";

#[derive(Debug, PartialEq)]
enum Request {
    Expand { lbuffer: String, rbuffer: String },
    Placeholder { lbuffer: String, rbuffer: String },
    Remind { buffer: String },
    History { limit: usize },
    FlushHistory,
    ClearHistory,
    Reload,
    Ping,
}

fn parse_request(line: &str) -> Result<Request> {
    let mut parts = line.splitn(3, '\t');
    let command = parts.next().unwrap_or("");

    match command {
        "expand" => {
            let lbuffer = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing lbuffer"))?
                .to_string();
            let rbuffer = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing rbuffer"))?
                .to_string();
            Ok(Request::Expand { lbuffer, rbuffer })
        }
        "placeholder" => {
            let lbuffer = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing lbuffer"))?
                .to_string();
            let rbuffer = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing rbuffer"))?
                .to_string();
            Ok(Request::Placeholder { lbuffer, rbuffer })
        }
        "remind" => {
            let buffer = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing buffer"))?
                .to_string();
            Ok(Request::Remind { buffer })
        }
        "history" => {
            let limit: usize = parts
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(50);
            Ok(Request::History { limit })
        }
        "flush_history" => Ok(Request::FlushHistory),
        "clear_history" => Ok(Request::ClearHistory),
        "reload" => Ok(Request::Reload),
        "ping" => Ok(Request::Ping),
        other => anyhow::bail!("unknown command: {}", other),
    }
}

const HISTORY_FLUSH_THRESHOLD: usize = 10;

struct ServeState {
    compiled: Option<CompiledCache>,
    regex_cache: RegexCache,
    config_path: PathBuf,
    cache_path: PathBuf,
    config_mtime: Option<SystemTime>,
    history_path: PathBuf,
    history_enabled: bool,
    history_limit: usize,
    history_buffer: Vec<HistoryEntry>,
}

impl ServeState {
    fn new(cache_path: PathBuf, config_path: PathBuf) -> Self {
        // Read history settings from config
        let (history_enabled, history_limit) = crate::config::load(&config_path)
            .map(|cfg| (cfg.settings.history, cfg.settings.history_limit))
            .unwrap_or((true, 500));

        let history_path = history::default_history_path()
            .unwrap_or_else(|_| PathBuf::from("/tmp/abbrs-history"));

        Self {
            compiled: None,
            regex_cache: RegexCache::new(),
            config_path,
            cache_path,
            config_mtime: None,
            history_path,
            history_enabled,
            history_limit,
            history_buffer: Vec::new(),
        }
    }

    fn load_cache(&mut self) {
        match cache::read(&self.cache_path) {
            Ok(c) => {
                self.config_mtime = std::fs::metadata(&self.config_path)
                    .and_then(|m| m.modified())
                    .ok();
                self.compiled = Some(c);
            }
            Err(_) => {
                self.compiled = None;
            }
        }
    }

    fn check_and_reload_if_needed(&mut self) -> bool {
        let current_mtime = std::fs::metadata(&self.config_path)
            .and_then(|m| m.modified())
            .ok();

        // If mtime hasn't changed, cache is still fresh
        if current_mtime == self.config_mtime {
            return true;
        }

        // mtime changed — check hash
        if let Some(ref compiled) = self.compiled {
            if let Ok(fresh) = cache::is_fresh(compiled, &self.config_path) {
                if fresh {
                    // Hash still matches despite mtime change (e.g., touch)
                    self.config_mtime = current_mtime;
                    return true;
                }
            }
        }

        // Stale — try to reload cache from disk
        match cache::read(&self.cache_path) {
            Ok(c) => {
                if let Ok(fresh) = cache::is_fresh(&c, &self.config_path) {
                    if fresh {
                        self.compiled = Some(c);
                        self.config_mtime = current_mtime;
                        // Refresh history settings from config on hot-reload
                        if let Ok(cfg) = crate::config::load(&self.config_path) {
                            self.history_enabled = cfg.settings.history;
                            self.history_limit = cfg.settings.history_limit;
                        }
                        return true;
                    }
                }
                // Cache on disk is also stale
                false
            }
            Err(_) => false,
        }
    }
}

fn write_response<W: Write>(writer: &mut W, response: &str) -> std::io::Result<()> {
    writeln!(writer, "{}", response)?;
    writeln!(writer, "{}", EOR)?;
    Ok(())
}

fn write_empty_eor<W: Write>(writer: &mut W) -> std::io::Result<()> {
    writeln!(writer, "{}", EOR)?;
    Ok(())
}

fn handle_expand(state: &mut ServeState, lbuffer: &str, rbuffer: &str) -> ExpandResult {
    if state.compiled.is_none() {
        return ExpandResult { output: ExpandOutput::StaleCache, matched_expansion: None };
    }

    // Check freshness
    if !state.check_and_reload_if_needed() {
        return ExpandResult { output: ExpandOutput::StaleCache, matched_expansion: None };
    }

    let compiled = state.compiled.as_ref().unwrap();

    let input = ExpandInput {
        lbuffer: lbuffer.to_string(),
        rbuffer: rbuffer.to_string(),
    };
    expand::expand(&input, &compiled.matcher, &compiled.settings.prefixes, &state.regex_cache)
}

fn flush_history_buffer(state: &mut ServeState) {
    if state.history_buffer.is_empty() {
        return;
    }
    match history::flush_batch(&state.history_path, &state.history_buffer) {
        Ok(()) => {
            state.history_buffer.clear();
        }
        Err(e) => {
            eprintln!("abbrs serve: failed to flush history (retaining buffer): {}", e);
        }
    }
}

fn handle_history<W: Write>(state: &mut ServeState, limit: usize, writer: &mut W) -> std::io::Result<()> {
    if !state.history_enabled {
        return write_response(writer, "no_match");
    }

    // Flush pending entries before reading
    flush_history_buffer(state);

    let limit = if limit == 0 { state.history_limit } else { limit };

    let entries = match history::load(&state.history_path, limit) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("abbrs serve: failed to load history: {}", e);
            return write_response(writer, "no_match");
        }
    };

    if entries.is_empty() {
        return write_response(writer, "no_match");
    }

    // Format as candidates protocol
    let candidates: Vec<CandidateEntry> = entries
        .into_iter()
        .map(|e| CandidateEntry {
            keyword: e.keyword,
            expansion: e.expansion,
        })
        .collect();

    let output = ExpandOutput::Candidates { candidates };
    write_response(writer, &output.to_string())
}

fn handle_placeholder<W: Write>(lbuffer: &str, rbuffer: &str, writer: &mut W) -> std::io::Result<()> {
    let full_buffer = format!("{}{}", lbuffer, rbuffer);
    let cursor = lbuffer.len();

    match placeholder::find_next_placeholder(&full_buffer, cursor) {
        Some((start, end)) => {
            let mut new_buffer = String::with_capacity(full_buffer.len() - (end - start));
            new_buffer.push_str(&full_buffer[..start]);
            new_buffer.push_str(&full_buffer[end..]);

            let output = PlaceholderOutput::Success {
                buffer: new_buffer,
                cursor: start,
            };
            write_response(writer, &output.to_string())
        }
        None => write_response(writer, &PlaceholderOutput::NoPlaceholder.to_string()),
    }
}

fn handle_remind<W: Write>(state: &ServeState, buffer: &str, writer: &mut W) -> std::io::Result<()> {
    let compiled = match &state.compiled {
        Some(c) => c,
        None => return write_empty_eor(writer),
    };

    if !compiled.settings.remind {
        return write_empty_eor(writer);
    }

    if let Some((keyword, expansion)) = expand::check_remind(buffer, &compiled.matcher) {
        let msg = format!(
            "abbrs: you could have used \"{}\" instead of \"{}\"",
            keyword, expansion
        );
        write_response(writer, &msg)
    } else {
        write_empty_eor(writer)
    }
}

fn handle_reload<W: Write>(state: &mut ServeState, writer: &mut W) -> std::io::Result<()> {
    state.load_cache();
    state.regex_cache = RegexCache::new();
    write_response(writer, "ok")
}

fn handle_ping<W: Write>(writer: &mut W) -> std::io::Result<()> {
    write_response(writer, "pong")
}

fn resolve_paths(
    cache_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<(PathBuf, PathBuf)> {
    let cache_file = match cache_path {
        Some(p) => p,
        None => crate::config::default_cache_path()?,
    };
    let cfg_path = match config_path {
        Some(p) => p,
        None => crate::config::default_config_path()?,
    };
    Ok((cache_file, cfg_path))
}

/// Process a single parsed request against state and write the response.
fn process_request<W: Write>(
    state: &mut ServeState,
    request: &Request,
    writer: &mut W,
) -> std::io::Result<()> {
    match request {
        Request::Expand { lbuffer, rbuffer } => {
            let result = handle_expand(state, lbuffer, rbuffer);

            // Record history using the exact matched expansion (not re-derived)
            if state.history_enabled {
                if let Some(expansion) = &result.matched_expansion {
                    if let Some((_, keyword)) = expand::extract_keyword(lbuffer) {
                        state.history_buffer.push(HistoryEntry::new(keyword.to_string(), expansion.clone()));
                        if state.history_buffer.len() >= HISTORY_FLUSH_THRESHOLD {
                            flush_history_buffer(state);
                        }
                    }
                }
            }

            write_response(writer, &result.output.to_string())
        }
        Request::Placeholder { lbuffer, rbuffer } => {
            handle_placeholder(lbuffer, rbuffer, writer)
        }
        Request::Remind { buffer } => {
            handle_remind(state, buffer, writer)
        }
        Request::History { limit } => {
            handle_history(state, *limit, writer)
        }
        Request::FlushHistory => {
            flush_history_buffer(state);
            write_response(writer, "ok")
        }
        Request::ClearHistory => {
            state.history_buffer.clear();
            let _ = history::clear(&state.history_path);
            write_response(writer, "ok")
        }
        Request::Reload => {
            flush_history_buffer(state);
            // Re-read history settings from config on reload
            if let Ok(cfg) = crate::config::load(&state.config_path) {
                state.history_enabled = cfg.settings.history;
                state.history_limit = cfg.settings.history_limit;
            }
            handle_reload(state, writer)
        }
        Request::Ping => handle_ping(writer),
    }
}

fn serve_connection<R: BufRead, W: Write>(
    state: &mut ServeState,
    reader: R,
    writer: &mut W,
) -> Result<()> {
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                eprintln!("abbrs serve: read error: {}", e);
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        let request = match parse_request(&line) {
            Ok(r) => r,
            Err(e) => {
                let result = write_response(
                    writer,
                    &format!("error\t{}", e),
                );
                if let Err(write_err) = result {
                    if write_err.kind() == std::io::ErrorKind::BrokenPipe {
                        break;
                    }
                    eprintln!("abbrs serve: write error: {}", write_err);
                }
                continue;
            }
        };

        let result = process_request(state, &request, writer);

        if let Err(e) = result {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            }
            eprintln!("abbrs serve: write error: {}", e);
        }
    }

    // Flush remaining history buffer on disconnect
    flush_history_buffer(state);

    Ok(())
}

/// Serve a single connection using shared state (for multi-threaded socket mode).
/// Locks the mutex per-request so other connections can interleave.
fn serve_connection_shared<R: BufRead, W: Write>(
    state: &std::sync::Arc<std::sync::Mutex<ServeState>>,
    reader: R,
    writer: &mut W,
) -> Result<()> {
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                eprintln!("abbrs serve: read error: {}", e);
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        let request = match parse_request(&line) {
            Ok(r) => r,
            Err(e) => {
                let result = write_response(
                    writer,
                    &format!("error\t{}", e),
                );
                if let Err(write_err) = result {
                    if write_err.kind() == std::io::ErrorKind::BrokenPipe {
                        break;
                    }
                    eprintln!("abbrs serve: write error: {}", write_err);
                }
                continue;
            }
        };

        // Lock state only for the duration of request processing.
        // Between requests (while waiting for the next line), the lock is released,
        // allowing other connections (e.g. notify_all_daemons) to be served.
        let result = {
            let mut state = state.lock().unwrap();
            process_request(&mut state, &request, writer)
        };

        if let Err(e) = result {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            }
            eprintln!("abbrs serve: write error: {}", e);
        }
    }

    // Flush remaining history buffer on disconnect
    let mut state = state.lock().unwrap();
    flush_history_buffer(&mut state);

    Ok(())
}

pub fn run(cache_path: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    let (cache_file, cfg_path) = resolve_paths(cache_path, config_path)?;

    let mut state = ServeState::new(cache_file, cfg_path);
    state.load_cache();

    // Compact history at daemon startup
    if state.history_enabled {
        let _ = history::compact(&state.history_path, state.history_limit);
    }

    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = LineWriter::new(stdout.lock());

    serve_connection(&mut state, reader, &mut writer)
}

/// Ensure the parent directory of the socket path is a private directory
/// owned by the current user with mode 0700. This prevents TOCTOU races
/// in stale socket cleanup: since only the owner can modify files inside
/// a 0700 directory, no other process can swap in a non-socket between
/// our check and unlink.
fn ensure_private_socket_dir(socket_path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    let parent = socket_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("socket path has no parent directory"))?;

    let our_uid = unsafe { libc::getuid() };

    match std::fs::DirBuilder::new()
        .mode(0o700)
        .create(parent)
    {
        Ok(()) => {
            // DirBuilder::mode() is filtered by the process umask, so the
            // on-disk mode may be more restrictive than 0700.  Explicitly
            // set_permissions to guarantee the owner has full rwx.
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Directory exists — verify it's safe
            let meta = std::fs::symlink_metadata(parent)?;
            if !meta.file_type().is_dir() {
                anyhow::bail!(
                    "socket parent path is not a directory: {}",
                    parent.display()
                );
            }
            if meta.uid() != our_uid {
                anyhow::bail!(
                    "socket directory not owned by current user: {}",
                    parent.display()
                );
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o700 {
                anyhow::bail!(
                    "socket directory has unsafe permissions {:o} (expected 0700): {}",
                    mode,
                    parent.display()
                );
            }
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

pub fn run_socket(
    socket_path: PathBuf,
    cache_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::{Arc, Mutex};

    let (cache_file, cfg_path) = resolve_paths(cache_path, config_path)?;

    // Ensure socket lives inside a private directory (mode 0700, owned by us).
    // This eliminates TOCTOU races in the stale-cleanup below because only the
    // owner can create/replace files inside the directory.
    ensure_private_socket_dir(&socket_path)?;

    // Stale cleanup: safe now because the parent directory is private
    if socket_path.exists() {
        let metadata = std::fs::symlink_metadata(&socket_path)?;
        if !metadata.file_type().is_socket() {
            anyhow::bail!(
                "path exists and is not a socket: {}",
                socket_path.display()
            );
        }
        match UnixStream::connect(&socket_path) {
            Ok(_) => anyhow::bail!("socket already in use: {}", socket_path.display()),
            Err(_) => {
                // Socket file exists but no process is listening — stale
                std::fs::remove_file(&socket_path)?;
            }
        }
    }

    let listener = UnixListener::bind(&socket_path)?;

    // Restrict socket file permissions to owner only
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }

    let mut state = ServeState::new(cache_file, cfg_path);
    state.load_cache();

    // Compact history at daemon startup
    if state.history_enabled {
        let _ = history::compact(&state.history_path, state.history_limit);
    }

    // Wrap state in Arc<Mutex> for concurrent connection handling.
    // This allows notify_all_daemons() to reach the daemon even while
    // a shell connection is active, by serving each connection in its
    // own thread with per-request locking.
    let state = Arc::new(Mutex::new(state));

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("abbrs serve: accept error: {}", e);
                continue;
            }
        };
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            let reader = match stream.try_clone() {
                Ok(s) => BufReader::new(s),
                Err(e) => {
                    eprintln!("abbrs serve: clone error: {}", e);
                    return;
                }
            };
            let mut writer = LineWriter::new(stream);
            if let Err(e) = serve_connection_shared(&state, reader, &mut writer) {
                eprintln!("abbrs serve: connection error: {}", e);
            }
        });
    }

    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

/// Get the default socket directory path (matches zsh widget convention).
pub fn default_socket_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(format!("{}/abbrs-{}", tmpdir, uid))
}

/// Send a command to a single daemon socket and wait for the response.
fn send_to_daemon(socket_path: &std::path::Path, command: &str) -> Result<()> {
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;

    let read_stream = stream.try_clone()?;
    let mut writer = LineWriter::new(stream);
    writeln!(writer, "{}", command)?;
    writer.flush()?;

    // Read until EOR
    let reader = BufReader::new(read_stream);
    for line in reader.lines() {
        let line = line?;
        if line.starts_with('\x1e') {
            break;
        }
    }

    Ok(())
}

/// Send a command to all active daemon sockets in the socket directory.
/// Silently ignores connection failures (stale sockets, etc.).
pub fn notify_all_daemons(command: &str) {
    let sock_dir = default_socket_dir();
    if !sock_dir.exists() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(&sock_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "sock") {
                let _ = send_to_daemon(&path, command);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachedSettings;
    use crate::config::Abbreviation;
    use insta::assert_snapshot;

    // === Helpers ===

    /// Build a ServeState with a real cache from test abbreviations.
    fn create_test_state(abbrs: &[Abbreviation], settings: CachedSettings) -> (ServeState, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("abbrs.toml");
        let cache_path = dir.path().join("abbrs.cache");

        // Write a minimal config (content is hashed for freshness)
        let mut toml = String::from("[settings]\n");
        for abbr in abbrs {
            toml.push_str("\n[[abbr]]\n");
            toml.push_str(&format!("keyword = \"{}\"\n", abbr.keyword));
            toml.push_str(&format!("expansion = \"{}\"\n", abbr.expansion));
            if abbr.global {
                toml.push_str("global = true\n");
            }
            if abbr.evaluate {
                toml.push_str("evaluate = true\n");
            }
            if abbr.function {
                toml.push_str("function = true\n");
            }
            if let Some(ref cmd) = abbr.command {
                toml.push_str(&format!("command = \"{}\"\n", cmd));
            }
        }
        std::fs::write(&config_path, &toml).unwrap();

        let matcher = crate::matcher::build(abbrs);
        cache::write(&cache_path, &matcher, &settings, &config_path).unwrap();

        let mut state = ServeState::new(cache_path, config_path);
        state.load_cache();
        (state, dir)
    }

    fn default_abbrs() -> Vec<Abbreviation> {
        vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit -m '{{message}}'".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
                ..Default::default()
            },
            Abbreviation {
                keyword: "NE".to_string(),
                expansion: "2>/dev/null".to_string(),
                global: true,
                ..Default::default()
            },
            Abbreviation {
                keyword: "TODAY".to_string(),
                expansion: "date +%Y-%m-%d".to_string(),
                global: true,
                evaluate: true,
                ..Default::default()
            },
        ]
    }

    /// Extract the response body from raw wire output (strips trailing \n\x1e\n).
    fn response_body(raw: &[u8]) -> String {
        let s = String::from_utf8(raw.to_vec()).unwrap();
        let without_eor = s.strip_suffix("\x1e\n").unwrap_or(&s);
        without_eor
            .strip_suffix('\n')
            .unwrap_or(without_eor)
            .to_string()
    }

    // === Parse request tests ===

    #[test]
    fn test_parse_expand() {
        let req = parse_request("expand\tgit co\t--help").unwrap();
        assert_eq!(
            req,
            Request::Expand {
                lbuffer: "git co".to_string(),
                rbuffer: "--help".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_expand_empty_rbuffer() {
        let req = parse_request("expand\tg\t").unwrap();
        assert_eq!(
            req,
            Request::Expand {
                lbuffer: "g".to_string(),
                rbuffer: "".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_placeholder() {
        let req = parse_request("placeholder\tgit commit -m '\t' --author=''").unwrap();
        assert_eq!(
            req,
            Request::Placeholder {
                lbuffer: "git commit -m '".to_string(),
                rbuffer: "' --author=''".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_remind() {
        let req = parse_request("remind\tgit commit -m 'hello'").unwrap();
        assert_eq!(
            req,
            Request::Remind {
                buffer: "git commit -m 'hello'".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_reload() {
        let req = parse_request("reload").unwrap();
        assert_eq!(req, Request::Reload);
    }

    #[test]
    fn test_parse_ping() {
        let req = parse_request("ping").unwrap();
        assert_eq!(req, Request::Ping);
    }

    #[test]
    fn test_parse_unknown_command() {
        let result = parse_request("unknown_cmd");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown command"));
    }

    #[test]
    fn test_parse_expand_missing_rbuffer() {
        let result = parse_request("expand\tg");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing rbuffer"));
    }

    #[test]
    fn test_parse_expand_missing_lbuffer() {
        let result = parse_request("expand");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing lbuffer"));
    }

    #[test]
    fn test_parse_remind_missing_buffer() {
        let result = parse_request("remind");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing buffer"));
    }

    // === Wire format tests ===

    #[test]
    fn test_write_response() {
        let mut buf = Vec::new();
        write_response(&mut buf, "pong").unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "pong\n\x1e\n");
    }

    #[test]
    fn test_write_multiline_response() {
        let mut buf = Vec::new();
        write_response(&mut buf, "success\ngit commit\n10").unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "success\ngit commit\n10\n\x1e\n"
        );
    }

    #[test]
    fn test_write_empty_eor() {
        let mut buf = Vec::new();
        write_empty_eor(&mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "\x1e\n");
    }

    // === Handler snapshot tests ===

    #[test]
    fn test_handle_ping_response() {
        let mut buf = Vec::new();
        handle_ping(&mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"pong");
    }

    #[test]
    fn test_handle_expand_regular() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let output = handle_expand(&mut state, "g", "");
        assert_snapshot!(output.to_string(), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_handle_expand_with_placeholder() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let output = handle_expand(&mut state, "gc", "");
        assert_snapshot!(output.to_string(), @r"
        success
        git commit -m ''
        15
        ");
    }

    #[test]
    fn test_handle_expand_global() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let output = handle_expand(&mut state, "echo NE", "");
        assert_snapshot!(output.to_string(), @r"
        success
        echo 2>/dev/null
        16
        ");
    }

    #[test]
    fn test_handle_expand_evaluate() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let output = handle_expand(&mut state, "echo TODAY", "");
        assert_snapshot!(output.to_string(), @r"
        evaluate
        date +%Y-%m-%d
        echo

        ");
    }

    #[test]
    fn test_handle_expand_no_match() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let output = handle_expand(&mut state, "unknown", "");
        assert_snapshot!(output.to_string(), @"no_match");
    }

    #[test]
    fn test_handle_expand_prefix_candidates() {
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
        let (mut state, _dir) = create_test_state(&abbrs, CachedSettings::default());
        let output = handle_expand(&mut state, "g", "");
        assert_snapshot!(output.to_string(), @r"
        candidates
        3
        gc	git commit
        gd	git diff
        gp	git push
        ");
    }

    #[test]
    fn test_handle_expand_prefix_candidates_single_returns_candidates() {
        let abbrs = vec![
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit".to_string(),
                ..Default::default()
            },
        ];
        let (mut state, _dir) = create_test_state(&abbrs, CachedSettings::default());
        let output = handle_expand(&mut state, "g", "");
        assert_snapshot!(output.to_string(), @r"
        candidates
        1
        gc	git commit
        ");
    }

    #[test]
    fn test_handle_expand_exact_match_over_candidates() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        // "g" has an exact match → expands to "git", not candidates
        let output = handle_expand(&mut state, "g", "");
        assert_snapshot!(output.to_string(), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_handle_expand_with_rbuffer() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let output = handle_expand(&mut state, "g", " --help");
        assert_snapshot!(output.to_string(), @r"
        success
        git --help
        3
        ");
    }

    #[test]
    fn test_handle_expand_command_scoped() {
        let abbrs = vec![
            Abbreviation {
                keyword: "co".to_string(),
                expansion: "checkout".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
        ];
        let (mut state, _dir) = create_test_state(&abbrs, CachedSettings::default());
        let output = handle_expand(&mut state, "git co", "");
        assert_snapshot!(output.to_string(), @r"
        success
        git checkout
        12
        ");
    }

    #[test]
    fn test_handle_expand_command_scoped_wrong_command() {
        let abbrs = vec![
            Abbreviation {
                keyword: "co".to_string(),
                expansion: "checkout".to_string(),
                command: Some("git".to_string()),
                ..Default::default()
            },
        ];
        let (mut state, _dir) = create_test_state(&abbrs, CachedSettings::default());
        let output = handle_expand(&mut state, "npm co", "");
        assert_snapshot!(output.to_string(), @"no_match");
    }

    #[test]
    fn test_handle_expand_function() {
        let abbrs = vec![
            Abbreviation {
                keyword: "mf".to_string(),
                expansion: "my_func".to_string(),
                function: true,
                ..Default::default()
            },
        ];
        let (mut state, _dir) = create_test_state(&abbrs, CachedSettings::default());
        let output = handle_expand(&mut state, "mf", "");
        assert_snapshot!(output.to_string(), @r"
        function
        my_func
        mf


        ");
    }

    #[test]
    fn test_handle_expand_no_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = ServeState::new(
            dir.path().join("nonexistent.cache"),
            dir.path().join("nonexistent.toml"),
        );
        let output = handle_expand(&mut state, "g", "");
        assert_snapshot!(output.to_string(), @"stale_cache");
    }

    #[test]
    fn test_handle_placeholder_found() {
        let mut buf = Vec::new();
        handle_placeholder("git commit -m '", "' --author='{{author}}'", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        success
        git commit -m '' --author=''
        27
        ");
    }

    #[test]
    fn test_handle_placeholder_not_found() {
        let mut buf = Vec::new();
        handle_placeholder("no placeholder", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"no_placeholder");
    }

    #[test]
    fn test_handle_remind_no_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = ServeState::new(
            dir.path().join("nonexistent.cache"),
            dir.path().join("nonexistent.toml"),
        );
        let mut buf = Vec::new();
        handle_remind(&state, "git push", &mut buf).unwrap();
        // No cache → empty (just EOR marker)
        assert_snapshot!(response_body(&buf), @"");
    }

    #[test]
    fn test_handle_remind_with_match() {
        let abbrs = vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                ..Default::default()
            },
        ];
        let settings = CachedSettings {
            remind: true,
            prefixes: vec![],
            ..Default::default()
        };
        let (state, _dir) = create_test_state(&abbrs, settings);
        let mut buf = Vec::new();
        handle_remind(&state, "git push", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r#"abbrs: you could have used "g" instead of "git""#);
    }

    #[test]
    fn test_handle_remind_no_match() {
        let abbrs = vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                ..Default::default()
            },
        ];
        let settings = CachedSettings {
            remind: true,
            prefixes: vec![],
            ..Default::default()
        };
        let (state, _dir) = create_test_state(&abbrs, settings);
        let mut buf = Vec::new();
        handle_remind(&state, "cargo build", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"");
    }

    #[test]
    fn test_handle_remind_disabled() {
        let abbrs = vec![
            Abbreviation {
                keyword: "g".to_string(),
                expansion: "git".to_string(),
                ..Default::default()
            },
        ];
        let settings = CachedSettings {
            remind: false,
            prefixes: vec![],
            ..Default::default()
        };
        let (state, _dir) = create_test_state(&abbrs, settings);
        let mut buf = Vec::new();
        handle_remind(&state, "git push", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"");
    }
}
