use crate::config::Abbreviation;
use std::path::PathBuf;

/// Conflict type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictType {
    /// Exact match with command on PATH → error
    ExactPathMatch,
    /// Suffix match with command on PATH → warning
    SuffixPathMatch,
    /// Shell builtin → error
    ShellBuiltin,
}

/// Conflict information
#[derive(Debug, Clone)]
pub struct Conflict {
    pub keyword: String,
    pub conflict_type: ConflictType,
    pub conflicting_command: String,
    pub command_path: Option<PathBuf>,
}

/// Conflict detection report
#[derive(Debug, Default)]
pub struct ConflictReport {
    pub errors: Vec<Conflict>,
    pub warnings: Vec<Conflict>,
}

impl ConflictReport {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.conflict_type {
            ConflictType::ExactPathMatch => {
                write!(
                    f,
                    "\"{}\" conflicts with command on PATH: {} (exact match)",
                    self.keyword,
                    self.command_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| self.conflicting_command.clone())
                )
            }
            ConflictType::SuffixPathMatch => {
                write!(
                    f,
                    "\"{}\" matches suffix of command \"{}\" on PATH: {}",
                    self.keyword,
                    self.conflicting_command,
                    self.command_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            }
            ConflictType::ShellBuiltin => {
                write!(
                    f,
                    "\"{}\" conflicts with zsh builtin command",
                    self.keyword
                )
            }
        }
    }
}

/// List of zsh builtin commands
pub fn zsh_builtins() -> &'static [&'static str] {
    &[
        "alias",
        "autoload",
        "bg",
        "bindkey",
        "break",
        "builtin",
        "bye",
        "cd",
        "chdir",
        "command",
        "comparguments",
        "compcall",
        "compctl",
        "compdescribe",
        "compfiles",
        "compgroups",
        "compquote",
        "comptags",
        "comptry",
        "compvalues",
        "continue",
        "declare",
        "dirs",
        "disable",
        "disown",
        "echo",
        "emulate",
        "enable",
        "eval",
        "exec",
        "exit",
        "export",
        "false",
        "fc",
        "fg",
        "float",
        "functions",
        "getln",
        "getopts",
        "hash",
        "history",
        "integer",
        "jobs",
        "kill",
        "let",
        "limit",
        "local",
        "log",
        "logout",
        "noglob",
        "popd",
        "print",
        "printf",
        "pushd",
        "pushln",
        "pwd",
        "read",
        "readonly",
        "rehash",
        "return",
        "sched",
        "set",
        "setopt",
        "shift",
        "source",
        "suspend",
        "test",
        "times",
        "trap",
        "true",
        "ttyctl",
        "type",
        "typeset",
        "ulimit",
        "umask",
        "unalias",
        "unfunction",
        "unhash",
        "unlimit",
        "unset",
        "unsetopt",
        "vared",
        "wait",
        "whence",
        "where",
        "which",
        "zcompile",
        "zformat",
        "zle",
        "zmodload",
        "zparseopts",
        "zregexparse",
        "zstyle",
    ]
}

/// Scan $PATH and collect command names
pub fn scan_path() -> Vec<(String, PathBuf)> {
    let path_var = match std::env::var("PATH") {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut commands = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();

    for dir in path_var.split(':') {
        let dir_path = PathBuf::from(dir);
        let entries = match std::fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Skip duplicates (first found takes priority)
            if seen.contains(&file_name) {
                continue;
            }

            // Check if file is executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                        seen.insert(file_name.clone());
                        commands.push((file_name, entry.path()));
                    }
                }
            }

            #[cfg(not(unix))]
            {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        seen.insert(file_name.clone());
                        commands.push((file_name, entry.path()));
                    }
                }
            }
        }
    }

    commands
}

/// Main entry point for conflict detection
pub fn detect_conflicts(
    abbreviations: &[Abbreviation],
    path_commands: &[(String, PathBuf)],
    strict: bool,
) -> ConflictReport {
    let mut report = ConflictReport::default();
    let builtins = zsh_builtins();

    for abbr in abbreviations {
        // Skip if allow_conflict is set
        if abbr.allow_conflict {
            continue;
        }

        let keyword = &abbr.keyword;

        // 1. Check for shell builtin conflicts (binary search on sorted array)
        if builtins.binary_search(&keyword.as_str()).is_ok() {
            report.errors.push(Conflict {
                keyword: keyword.clone(),
                conflict_type: ConflictType::ShellBuiltin,
                conflicting_command: keyword.clone(),
                command_path: None,
            });
            continue; // Report builtin conflict as error and skip PATH check
        }

        // 2. Check for PATH command conflicts
        for (cmd_name, cmd_path) in path_commands {
            // Exact match
            if cmd_name == keyword {
                report.errors.push(Conflict {
                    keyword: keyword.clone(),
                    conflict_type: ConflictType::ExactPathMatch,
                    conflicting_command: cmd_name.clone(),
                    command_path: Some(cmd_path.clone()),
                });
            }
            // Suffix match (not exact match, command name ends with keyword)
            else if cmd_name.len() > keyword.len() && cmd_name.ends_with(keyword.as_str()) {
                let conflict = Conflict {
                    keyword: keyword.clone(),
                    conflict_type: ConflictType::SuffixPathMatch,
                    conflicting_command: cmd_name.clone(),
                    command_path: Some(cmd_path.clone()),
                };
                if strict {
                    report.errors.push(conflict);
                } else {
                    report.warnings.push(conflict);
                }
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Abbreviation;

    fn make_abbr(keyword: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: "dummy".to_string(),
            ..Default::default()
        }
    }

    fn make_abbr_allow_conflict(keyword: &str) -> Abbreviation {
        Abbreviation {
            keyword: keyword.to_string(),
            expansion: "dummy".to_string(),
            allow_conflict: true,
            ..Default::default()
        }
    }

    fn make_path_commands(commands: &[(&str, &str)]) -> Vec<(String, PathBuf)> {
        commands
            .iter()
            .map(|(name, path)| (name.to_string(), PathBuf::from(path)))
            .collect()
    }

    #[test]
    fn test_detect_exact_path_match() {
        let abbrs = vec![make_abbr("cc")];
        let path_cmds = make_path_commands(&[("cc", "/usr/bin/cc")]);
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].conflict_type, ConflictType::ExactPathMatch);
        assert_eq!(report.errors[0].keyword, "cc");
    }

    #[test]
    fn test_detect_suffix_path_match_warning() {
        let abbrs = vec![make_abbr("cc")];
        let path_cmds = make_path_commands(&[("gcc", "/usr/bin/gcc")]);
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        assert!(report.errors.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].conflict_type,
            ConflictType::SuffixPathMatch
        );
    }

    #[test]
    fn test_detect_suffix_path_match_strict() {
        let abbrs = vec![make_abbr("cc")];
        let path_cmds = make_path_commands(&[("gcc", "/usr/bin/gcc")]);
        let report = detect_conflicts(&abbrs, &path_cmds, true);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].conflict_type,
            ConflictType::SuffixPathMatch
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_detect_shell_builtin() {
        let abbrs = vec![make_abbr("cd"), make_abbr("echo")];
        let path_cmds = vec![];
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        assert_eq!(report.errors.len(), 2);
        assert!(report
            .errors
            .iter()
            .all(|c| c.conflict_type == ConflictType::ShellBuiltin));
    }

    #[test]
    fn test_allow_conflict_skips() {
        let abbrs = vec![make_abbr_allow_conflict("cc")];
        let path_cmds = make_path_commands(&[("cc", "/usr/bin/cc")]);
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_no_conflicts() {
        let abbrs = vec![make_abbr("g"), make_abbr("gc")];
        let path_cmds = make_path_commands(&[("git", "/usr/bin/git"), ("ls", "/bin/ls")]);
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_multiple_conflicts() {
        let abbrs = vec![make_abbr("cc"), make_abbr("gs"), make_abbr("cd")];
        let path_cmds = make_path_commands(&[
            ("cc", "/usr/bin/cc"),
            ("gcc", "/usr/bin/gcc"),
            ("gs", "/usr/local/bin/gs"),
        ]);
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        // cc: exact match (error) + gcc suffix (warning)
        // gs: exact match (error)
        // cd: builtin (error)
        assert_eq!(report.errors.len(), 3); // cc exact, gs exact, cd builtin
        assert_eq!(report.warnings.len(), 1); // gcc suffix
    }

    #[test]
    fn test_zsh_builtins_contains_common_commands() {
        let builtins = zsh_builtins();
        assert!(builtins.contains(&"cd"));
        assert!(builtins.contains(&"echo"));
        assert!(builtins.contains(&"eval"));
        assert!(builtins.contains(&"source"));
        assert!(builtins.contains(&"export"));
        assert!(builtins.contains(&"alias"));
        assert!(builtins.contains(&"test"));
        assert!(builtins.contains(&"which"));
        assert!(builtins.contains(&"type"));
        assert!(builtins.contains(&"command"));
    }

    #[test]
    fn test_conflict_display() {
        let conflict = Conflict {
            keyword: "cc".to_string(),
            conflict_type: ConflictType::ExactPathMatch,
            conflicting_command: "cc".to_string(),
            command_path: Some(PathBuf::from("/usr/bin/cc")),
        };
        let display = conflict.to_string();
        assert!(display.contains("cc"));
        assert!(display.contains("/usr/bin/cc"));
        assert!(display.contains("exact match"));
    }

    #[test]
    fn test_conflict_report_has_errors() {
        let mut report = ConflictReport::default();
        assert!(!report.has_errors());

        report.errors.push(Conflict {
            keyword: "cd".to_string(),
            conflict_type: ConflictType::ShellBuiltin,
            conflicting_command: "cd".to_string(),
            command_path: None,
        });
        assert!(report.has_errors());
    }

    #[test]
    fn test_suffix_match_not_triggered_for_same_length() {
        // "git" and "git" is an exact match, not a suffix match
        let abbrs = vec![make_abbr("git")];
        let path_cmds = make_path_commands(&[("git", "/usr/bin/git")]);
        let report = detect_conflicts(&abbrs, &path_cmds, false);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].conflict_type, ConflictType::ExactPathMatch);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_scan_path_returns_results() {
        // Verify that scanning the actual PATH returns results
        let commands = scan_path();
        // At least some commands should exist even in CI environments
        assert!(!commands.is_empty());
    }
}
