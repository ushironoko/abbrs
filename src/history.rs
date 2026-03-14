use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub timestamp: u64,
    pub keyword: String,
    pub expansion: String,
}

impl HistoryEntry {
    pub fn new(keyword: String, expansion: String) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            timestamp,
            keyword,
            expansion,
        }
    }
}

/// Returns the default history file path: `$XDG_DATA_HOME/abbrs/history`
pub fn default_history_path() -> Result<PathBuf> {
    let base = xdg::BaseDirectories::with_prefix("abbrs")
        .context("failed to determine XDG base directories")?;
    Ok(base.get_data_home().join("history"))
}

/// Escape expansion text for TSV storage (same pattern as output.rs candidates)
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Unescape expansion text from TSV storage
fn unescape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Format a single history entry as a TSV line
fn format_entry(entry: &HistoryEntry) -> String {
    format!(
        "{}\t{}\t{}",
        entry.timestamp,
        escape(&entry.keyword),
        escape(&entry.expansion)
    )
}

/// Parse a single TSV line into a HistoryEntry
fn parse_line(line: &str) -> Option<HistoryEntry> {
    let mut parts = line.splitn(3, '\t');
    let timestamp: u64 = parts.next()?.parse().ok()?;
    let keyword = unescape(parts.next()?);
    let expansion = unescape(parts.next()?);
    if keyword.is_empty() {
        return None;
    }
    Some(HistoryEntry {
        timestamp,
        keyword,
        expansion,
    })
}

/// Append a single history entry to the file.
/// Uses O_APPEND for POSIX atomic append.
pub fn append(path: &Path, entry: &HistoryEntry) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create history directory: {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open history file: {}", path.display()))?;

    writeln!(file, "{}", format_entry(entry))
        .with_context(|| "failed to write history entry")?;

    Ok(())
}

/// Append multiple history entries at once (daemon batch flush).
pub fn flush_batch(path: &Path, entries: &[HistoryEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create history directory: {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open history file: {}", path.display()))?;

    for entry in entries {
        writeln!(file, "{}", format_entry(entry))?;
    }

    Ok(())
}

/// Load history entries, newest first. Returns up to `limit` entries.
/// Silently skips malformed lines. Returns empty Vec if file doesn't exist.
pub fn load(path: &Path, limit: usize) -> Result<Vec<HistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to open history file: {}", path.display()))?;

    let reader = BufReader::new(file);
    let mut entries: Vec<HistoryEntry> = reader
        .lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| parse_line(&line))
        .collect();

    // Newest first
    entries.reverse();

    if entries.len() > limit {
        entries.truncate(limit);
    }

    Ok(entries)
}

/// Clear the history file.
pub fn clear(path: &Path) -> Result<()> {
    if path.exists() {
        fs::write(path, "").with_context(|| "failed to clear history file")?;
    }
    Ok(())
}

/// Compact the history file: if it exceeds max_entries, keep only the newest 75%.
/// Should be called at daemon startup, not on the hot path.
pub fn compact(path: &Path, max_entries: usize) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to open history file: {}", path.display()))?;

    let reader = BufReader::new(file);
    let entries: Vec<String> = reader
        .lines()
        .filter_map(|line| line.ok())
        .collect();

    if entries.len() <= max_entries {
        return Ok(());
    }

    // Keep newest 75%
    let keep = max_entries * 3 / 4;
    let skip = entries.len() - keep;
    let kept: Vec<&str> = entries.iter().skip(skip).map(|s| s.as_str()).collect();

    let mut content = kept.join("\n");
    content.push('\n');

    fs::write(path, content).with_context(|| "failed to write compacted history")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_unescape() {
        let original = "git commit -m 'hello\nworld'\tfoo\\bar";
        let escaped = escape(original);
        assert_eq!(escaped, "git commit -m 'hello\\nworld'\\tfoo\\\\bar");
        let unescaped = unescape(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn test_escape_unescape_no_special() {
        let s = "git push origin main";
        assert_eq!(unescape(&escape(s)), s);
    }

    #[test]
    fn test_format_and_parse() {
        let entry = HistoryEntry {
            timestamp: 1710288000,
            keyword: "g".to_string(),
            expansion: "git".to_string(),
        };
        let line = format_entry(&entry);
        assert_eq!(line, "1710288000\tg\tgit");
        let parsed = parse_line(&line).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_parse_with_special_chars() {
        let entry = HistoryEntry {
            timestamp: 100,
            keyword: "gc".to_string(),
            expansion: "git commit -m '{{msg}}'".to_string(),
        };
        let line = format_entry(&entry);
        let parsed = parse_line(&line).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_parse_malformed() {
        assert!(parse_line("").is_none());
        assert!(parse_line("not_a_number\tg\tgit").is_none());
        assert!(parse_line("123\t\tgit").is_none()); // empty keyword
        assert!(parse_line("123\tg").is_none()); // missing expansion
    }

    #[test]
    fn test_append_and_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        let e1 = HistoryEntry {
            timestamp: 100,
            keyword: "g".to_string(),
            expansion: "git".to_string(),
        };
        let e2 = HistoryEntry {
            timestamp: 200,
            keyword: "gp".to_string(),
            expansion: "git push".to_string(),
        };

        append(&path, &e1).unwrap();
        append(&path, &e2).unwrap();

        let loaded = load(&path, 50).unwrap();
        assert_eq!(loaded.len(), 2);
        // Newest first
        assert_eq!(loaded[0], e2);
        assert_eq!(loaded[1], e1);
    }

    #[test]
    fn test_load_with_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        for i in 0..10 {
            append(
                &path,
                &HistoryEntry {
                    timestamp: i,
                    keyword: format!("k{}", i),
                    expansion: format!("e{}", i),
                },
            )
            .unwrap();
        }

        let loaded = load(&path, 3).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].keyword, "k9");
        assert_eq!(loaded[1].keyword, "k8");
        assert_eq!(loaded[2].keyword, "k7");
    }

    #[test]
    fn test_flush_batch() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        let entries = vec![
            HistoryEntry {
                timestamp: 100,
                keyword: "g".to_string(),
                expansion: "git".to_string(),
            },
            HistoryEntry {
                timestamp: 200,
                keyword: "gp".to_string(),
                expansion: "git push".to_string(),
            },
        ];

        flush_batch(&path, &entries).unwrap();

        let loaded = load(&path, 50).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].keyword, "gp");
        assert_eq!(loaded[1].keyword, "g");
    }

    #[test]
    fn test_flush_batch_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");
        flush_batch(&path, &[]).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent");
        let loaded = load(&path, 50).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_malformed_line_skipped() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        let content = "100\tg\tgit\nbad_line\n200\tgp\tgit push\n";
        fs::write(&path, content).unwrap();

        let loaded = load(&path, 50).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].keyword, "gp");
        assert_eq!(loaded[1].keyword, "g");
    }

    #[test]
    fn test_clear() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        append(
            &path,
            &HistoryEntry {
                timestamp: 100,
                keyword: "g".to_string(),
                expansion: "git".to_string(),
            },
        )
        .unwrap();

        clear(&path).unwrap();
        let loaded = load(&path, 50).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_clear_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent");
        clear(&path).unwrap(); // Should not error
    }

    #[test]
    fn test_compact() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        for i in 0..20 {
            append(
                &path,
                &HistoryEntry {
                    timestamp: i,
                    keyword: format!("k{}", i),
                    expansion: format!("e{}", i),
                },
            )
            .unwrap();
        }

        // max_entries=10, keep 75% = 7
        compact(&path, 10).unwrap();

        let loaded = load(&path, 50).unwrap();
        assert_eq!(loaded.len(), 7);
        // Should keep newest 7: k13..k19
        assert_eq!(loaded[0].keyword, "k19");
        assert_eq!(loaded[6].keyword, "k13");
    }

    #[test]
    fn test_compact_under_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("history");

        for i in 0..5 {
            append(
                &path,
                &HistoryEntry {
                    timestamp: i,
                    keyword: format!("k{}", i),
                    expansion: format!("e{}", i),
                },
            )
            .unwrap();
        }

        compact(&path, 10).unwrap();

        let loaded = load(&path, 50).unwrap();
        assert_eq!(loaded.len(), 5); // No change
    }

    #[test]
    fn test_compact_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent");
        compact(&path, 10).unwrap(); // Should not error
    }
}
