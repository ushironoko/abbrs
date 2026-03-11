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
                    &mut writer,
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
            eprintln!("abbrs serve: write error: {}", e);
        }
    }

    Ok(())
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
        let mut buf = Vec::new();
        handle_expand(&mut state, "g", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_handle_expand_with_placeholder() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let mut buf = Vec::new();
        handle_expand(&mut state, "gc", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        success
        git commit -m ''
        15
        ");
    }

    #[test]
    fn test_handle_expand_global() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let mut buf = Vec::new();
        handle_expand(&mut state, "echo NE", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        success
        echo 2>/dev/null
        16
        ");
    }

    #[test]
    fn test_handle_expand_evaluate() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let mut buf = Vec::new();
        handle_expand(&mut state, "echo TODAY", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        evaluate
        date +%Y-%m-%d
        echo

        ");
    }

    #[test]
    fn test_handle_expand_no_match() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let mut buf = Vec::new();
        handle_expand(&mut state, "unknown", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"no_match");
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
        let mut buf = Vec::new();
        handle_expand(&mut state, "g", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        candidates
        3
        gc	git commit
        gd	git diff
        gp	git push
        ");
    }

    #[test]
    fn test_handle_expand_prefix_candidates_single_returns_no_match() {
        let abbrs = vec![
            Abbreviation {
                keyword: "gc".to_string(),
                expansion: "git commit".to_string(),
                ..Default::default()
            },
        ];
        let (mut state, _dir) = create_test_state(&abbrs, CachedSettings::default());
        let mut buf = Vec::new();
        handle_expand(&mut state, "g", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"no_match");
    }

    #[test]
    fn test_handle_expand_exact_match_over_candidates() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let mut buf = Vec::new();
        // "g" has an exact match → expands to "git", not candidates
        handle_expand(&mut state, "g", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
        success
        git
        3
        ");
    }

    #[test]
    fn test_handle_expand_with_rbuffer() {
        let (mut state, _dir) = create_test_state(&default_abbrs(), CachedSettings::default());
        let mut buf = Vec::new();
        handle_expand(&mut state, "g", " --help", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
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
        let mut buf = Vec::new();
        handle_expand(&mut state, "git co", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
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
        let mut buf = Vec::new();
        handle_expand(&mut state, "npm co", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"no_match");
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
        let mut buf = Vec::new();
        handle_expand(&mut state, "mf", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @r"
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
        let mut buf = Vec::new();
        handle_expand(&mut state, "g", "", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"stale_cache");
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
        };
        let (state, _dir) = create_test_state(&abbrs, settings);
        let mut buf = Vec::new();
        handle_remind(&state, "git push", &mut buf).unwrap();
        assert_snapshot!(response_body(&buf), @"");
    }
}
