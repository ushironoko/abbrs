use crate::cache::{self, CompiledCache};
use crate::context::RegexCache;
use crate::expand::{self, ExpandInput};
use crate::output::PlaceholderOutput;
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
        "reload" => Ok(Request::Reload),
        "ping" => Ok(Request::Ping),
        other => anyhow::bail!("unknown command: {}", other),
    }
}

struct ServeState {
    compiled: Option<CompiledCache>,
    regex_cache: RegexCache,
    config_path: PathBuf,
    cache_path: PathBuf,
    config_mtime: Option<SystemTime>,
}

impl ServeState {
    fn new(cache_path: PathBuf, config_path: PathBuf) -> Self {
        Self {
            compiled: None,
            regex_cache: RegexCache::new(),
            config_path,
            cache_path,
            config_mtime: None,
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

fn handle_expand<W: Write>(state: &mut ServeState, lbuffer: &str, rbuffer: &str, writer: &mut W) -> std::io::Result<()> {
    if state.compiled.is_none() {
        return write_response(writer, "stale_cache");
    }

    // Check freshness
    if !state.check_and_reload_if_needed() {
        return write_response(writer, "stale_cache");
    }

    let compiled = state.compiled.as_ref().unwrap();

    let input = ExpandInput {
        lbuffer: lbuffer.to_string(),
        rbuffer: rbuffer.to_string(),
    };
    let result = expand::expand(&input, &compiled.matcher, &compiled.settings.prefixes, &state.regex_cache);
    write_response(writer, &result.to_string())
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
            "kort: you could have used \"{}\" instead of \"{}\"",
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

pub fn run(cache_path: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    let cache_file = match cache_path {
        Some(p) => p,
        None => crate::config::default_cache_path()?,
    };
    let cfg_path = match config_path {
        Some(p) => p,
        None => crate::config::default_config_path()?,
    };

    let mut state = ServeState::new(cache_file, cfg_path);
    state.load_cache();

    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = LineWriter::new(stdout.lock());

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                eprintln!("kort serve: read error: {}", e);
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
                    &mut writer,
                    &format!("error\t{}", e),
                );
                if let Err(write_err) = result {
                    if write_err.kind() == std::io::ErrorKind::BrokenPipe {
                        break;
                    }
                    eprintln!("kort serve: write error: {}", write_err);
                }
                continue;
            }
        };

        let result = match request {
            Request::Expand { lbuffer, rbuffer } => {
                handle_expand(&mut state, &lbuffer, &rbuffer, &mut writer)
            }
            Request::Placeholder { lbuffer, rbuffer } => {
                handle_placeholder(&lbuffer, &rbuffer, &mut writer)
            }
            Request::Remind { buffer } => {
                handle_remind(&state, &buffer, &mut writer)
            }
            Request::Reload => handle_reload(&mut state, &mut writer),
            Request::Ping => handle_ping(&mut writer),
        };

        if let Err(e) = result {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            }
            eprintln!("kort serve: write error: {}", e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_handle_ping() {
        let mut buf = Vec::new();
        handle_ping(&mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "pong\n\x1e\n");
    }

    #[test]
    fn test_handle_placeholder_found() {
        let mut buf = Vec::new();
        handle_placeholder("git commit -m '", "' --author='{{author}}'", &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.starts_with("success\n"));
        assert!(output.contains("git commit -m '' --author=''"));
        assert!(output.ends_with("\x1e\n"));
    }

    #[test]
    fn test_handle_placeholder_not_found() {
        let mut buf = Vec::new();
        handle_placeholder("no placeholder", "", &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.starts_with("no_placeholder"));
        assert!(output.ends_with("\x1e\n"));
    }

    #[test]
    fn test_handle_expand_no_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = ServeState::new(
            dir.path().join("nonexistent.cache"),
            dir.path().join("nonexistent.toml"),
        );
        let mut buf = Vec::new();
        handle_expand(&mut state, "g", "", &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.starts_with("stale_cache"));
        assert!(output.ends_with("\x1e\n"));
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
        let output = String::from_utf8(buf).unwrap();
        // No cache → empty EOR
        assert_eq!(output, "\x1e\n");
    }
}
