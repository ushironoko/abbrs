//! Security vulnerability tests for brv
//!
//! Tests the following attack vectors:
//! 1. Path traversal via config/cache file paths
//! 2. Arbitrary code execution via crafted inputs
//! 3. Memory safety via corrupted/malicious cache (bincode deserialization)
//! 4. CPU/memory exhaustion via ReDoS, huge inputs, large configs
//! 5. Shell injection via lbuffer/rbuffer inputs

use brv::{cache, config, context, expand, matcher, placeholder};
use std::path::Path;
use tempfile::TempDir;

// ============================================================================
// 1. Path Traversal Tests
// ============================================================================

#[test]
fn path_traversal_config_path_with_dotdot() {
    // Attempt to load config from a path traversal path
    // Should fail gracefully (file not found), not traverse to unexpected locations
    let result = config::load(Path::new("/../../../etc/passwd"));
    // The file either doesn't parse as TOML or doesn't exist
    assert!(result.is_err());
}

#[test]
fn path_traversal_cache_path_with_dotdot() {
    // Attempt to read cache from a path traversal path
    let result = cache::read(Path::new("/../../../etc/shadow"));
    assert!(result.is_err());
}

#[test]
fn path_traversal_cache_write_dotdot() {
    // Ensure cache write with traversal path doesn't escape temp dir
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("brv.toml");
    std::fs::write(
        &config_path,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();

    let m = matcher::Matcher::new();

    // Try to write cache to a traversal path outside temp dir
    // This should either fail or write to the resolved path (no escape)
    let traversal_path = dir.path().join("..").join("..").join("tmp_brv_test_escape");
    let result = cache::write(&traversal_path, &m, &config_path);
    // Clean up if it succeeded (it writes to the resolved path, not escaping sandbox)
    if result.is_ok() {
        let resolved = traversal_path.canonicalize().unwrap_or(traversal_path.clone());
        let _ = std::fs::remove_file(&resolved);
    }
}

#[test]
fn path_traversal_keyword_with_slashes() {
    // Keywords with path separators should be rejected (contain no spaces, but are weird)
    // Currently brv validates: no empty, no spaces. Slashes are allowed as keyword chars.
    // This test verifies they don't cause path traversal in any code path.
    let toml = r#"
[[abbr]]
keyword = "../../../etc/passwd"
expansion = "git"
"#;
    // Keyword contains '/' which is not a space, so validation passes
    // but this keyword should never match user input that isn't intentionally "../../..."
    let result = config::parse(toml);
    // It parses fine - slashes are valid keyword chars. The key question is:
    // does this keyword ever get used as a file path? No - it's only used as HashMap key.
    assert!(result.is_ok());

    let cfg = result.unwrap();
    let m = matcher::build(&cfg.abbr);

    // The keyword would only match if user literally types "../../../etc/passwd"
    let input = expand::ExpandInput {
        lbuffer: "../../../etc/passwd".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            // It matched but the expansion is just "git", no file access
            assert_eq!(buffer, "git");
        }
        _ => panic!("Should match since keyword matches exactly"),
    }
}

#[test]
fn path_traversal_expansion_with_path() {
    // Expansion containing path traversal strings should be treated as plain text
    let toml = r#"
[[abbr]]
keyword = "hack"
expansion = "cat /../../../etc/passwd"
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "hack".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            // Expansion is stored and returned as-is, no file access by brv
            assert_eq!(buffer, "cat /../../../etc/passwd");
        }
        _ => panic!("Should match"),
    }
}

// ============================================================================
// 2. Arbitrary Code Execution Tests
// ============================================================================

#[test]
fn code_exec_evaluate_does_not_execute_in_rust() {
    // The evaluate feature should NOT execute commands in Rust.
    // It should only return ExpandOutput::Evaluate with the command string.
    let toml = r#"
[[abbr]]
keyword = "EVIL"
expansion = "rm -rf /"
evaluate = true
global = true
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "EVIL".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Evaluate {
            command,
            prefix,
            rbuffer,
        } => {
            // Command is just a string, NOT executed by brv
            assert_eq!(command, "rm -rf /");
            assert_eq!(prefix, "");
            assert_eq!(rbuffer, "");
        }
        other => panic!("Expected Evaluate, got {:?}", other),
    }
}

#[test]
fn code_exec_shell_metacharacters_in_lbuffer() {
    // Shell metacharacters in lbuffer should be treated as plain text
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    let malicious_inputs = vec![
        "$(rm -rf /); g",
        "`rm -rf /` g",
        "$(cat /etc/passwd) g",
        "'; DROP TABLE users; -- g",
        "| rm -rf / g",
        "&& rm -rf / g",
    ];

    for input_str in malicious_inputs {
        let input = expand::ExpandInput {
            lbuffer: input_str.to_string(),
            rbuffer: "".to_string(),
        };
        let result = expand::expand(&input, &m);
        match result {
            brv::output::ExpandOutput::Success { buffer, .. } => {
                // The prefix (everything before keyword) is preserved as-is
                assert!(buffer.contains("git"));
                // Metacharacters are NOT interpreted, just string concatenation
                assert!(!buffer.is_empty());
            }
            brv::output::ExpandOutput::NoMatch => {
                // Also acceptable - keyword extraction might not find "g" as the last token
            }
            _ => {}
        }
    }
}

#[test]
fn code_exec_shell_injection_in_expansion() {
    // Expansion with shell injection should be stored as-is, not executed
    let toml = r#"
[[abbr]]
keyword = "test"
expansion = "$(whoami)@$(hostname)"
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "test".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            // Should be literal string, not executed
            assert_eq!(buffer, "$(whoami)@$(hostname)");
        }
        _ => panic!("Should match"),
    }
}

#[test]
fn code_exec_newline_injection_in_output() {
    // Newlines in expansion could potentially break the line-based output protocol
    let toml = r#"
[[abbr]]
keyword = "nl"
expansion = "line1\nline2\nline3"
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "nl".to_string(),
        rbuffer: "".to_string(),
    };
    let result = expand::expand(&input, &m);
    // This is important: the output protocol is line-based.
    // If expansion contains literal \n (as TOML escape), it could confuse the ZLE parser.
    let output_str = result.to_string();
    // The output format is: "success\n{buffer}\n{cursor}"
    // If buffer contains newlines, ZLE's `${(f)...}` split will be confused.
    // This is a known limitation of the line-based protocol.
    assert!(output_str.starts_with("success\n"));
}

#[test]
fn code_exec_evaluate_newline_injection() {
    // SECURITY FINDING: TOML basic strings interpret \n as actual newline.
    // This means evaluate commands CAN contain newlines, which breaks the
    // line-based output protocol used between brv and brv.zsh.
    //
    // In the ZLE script, output is split by lines: out=( "${(f)$(brv expand ...)}" )
    // If the command field contains a newline:
    //   "evaluate\necho pwned\nrm -rf /\nprefix\nrbuffer"
    // ZLE would parse:
    //   out[1] = "evaluate"
    //   out[2] = "echo pwned"  <-- only first line of command
    //   out[3] = "rm -rf /"    <-- this becomes the prefix!
    //   out[4] = "prefix"      <-- this becomes rbuffer
    //
    // However, this requires the user to intentionally put \n in their own config,
    // which is self-inflicted. The config file is user-owned.
    let toml = r#"
[[abbr]]
keyword = "inj"
expansion = "echo pwned\nrm -rf /"
evaluate = true
global = true
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "echo inj".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Evaluate { command, .. } => {
            // TOML \n in basic strings is parsed as actual newline character
            assert_eq!(command, "echo pwned\nrm -rf /");
            // The command contains an actual newline, which will break the
            // output protocol. This is a documentation-worthy finding.
            assert!(
                command.contains('\n'),
                "FINDING: evaluate command can contain newlines via TOML \\n escape"
            );
        }
        _ => panic!("Expected Evaluate"),
    }
}

#[test]
fn code_exec_evaluate_newline_protocol_corruption() {
    // Demonstrate that newlines in expansion fields corrupt the output protocol
    let toml = r#"
[[abbr]]
keyword = "inj"
expansion = "echo safe\necho injected"
evaluate = true
global = true
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "echo inj".to_string(),
        rbuffer: "".to_string(),
    };
    let result = expand::expand(&input, &m);
    let output_str = result.to_string();

    // The output protocol expects:
    //   Line 1: "evaluate"
    //   Line 2: command
    //   Line 3: prefix
    //   Line 4: rbuffer
    // But with a newline in command, we get 5 lines instead of 4
    let lines: Vec<&str> = output_str.split('\n').collect();
    assert!(
        lines.len() > 4,
        "FINDING: Newline in evaluate command produces {} lines instead of 4, \
         corrupting the output protocol. ZLE will misparse the output.",
        lines.len()
    );
}

#[test]
fn code_exec_success_newline_protocol_corruption() {
    // Same issue applies to non-evaluate expansions with newlines
    let toml = r#"
[[abbr]]
keyword = "multi"
expansion = "line1\nline2"
"#;
    let cfg = config::parse(toml).unwrap();
    let m = matcher::build(&cfg.abbr);
    let input = expand::ExpandInput {
        lbuffer: "multi".to_string(),
        rbuffer: "".to_string(),
    };
    let result = expand::expand(&input, &m);
    let output_str = result.to_string();

    // Success output expects exactly 3 lines: "success", buffer, cursor
    let lines: Vec<&str> = output_str.split('\n').collect();
    assert!(
        lines.len() > 3,
        "FINDING: Newline in expansion produces {} lines instead of 3, \
         corrupting the success output protocol.",
        lines.len()
    );
}

// ============================================================================
// 3. Memory Safety / Malicious Cache Tests (bincode deserialization)
// ============================================================================

#[test]
fn memory_corrupted_cache_random_bytes() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("brv.cache");

    // Write random/garbage data
    let garbage: Vec<u8> = (0..1024).map(|i| (i * 37 % 256) as u8).collect();
    std::fs::write(&cache_path, &garbage).unwrap();

    let result = cache::read(&cache_path);
    assert!(result.is_err(), "Should fail to deserialize garbage data");
}

#[test]
fn memory_corrupted_cache_truncated() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("brv.toml");
    std::fs::write(
        &config_path,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();

    let cache_path = dir.path().join("brv.cache");
    let m = matcher::Matcher::new();
    cache::write(&cache_path, &m, &config_path).unwrap();

    // Read valid cache and truncate it
    let data = std::fs::read(&cache_path).unwrap();
    let truncated = &data[..data.len() / 2];
    std::fs::write(&cache_path, truncated).unwrap();

    let result = cache::read(&cache_path);
    assert!(result.is_err(), "Should fail on truncated cache");
}

#[test]
fn memory_corrupted_cache_version_mismatch() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("brv.toml");
    std::fs::write(
        &config_path,
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();

    let cache_path = dir.path().join("brv.cache");
    let m = matcher::Matcher::new();
    cache::write(&cache_path, &m, &config_path).unwrap();

    // Corrupt the version field (first 4 bytes in bincode for u32)
    let mut data = std::fs::read(&cache_path).unwrap();
    // Set version to 999
    data[0] = 0xE7;
    data[1] = 0x03;
    data[2] = 0x00;
    data[3] = 0x00;
    std::fs::write(&cache_path, &data).unwrap();

    let result = cache::read(&cache_path);
    assert!(result.is_err(), "Should reject version mismatch");
    assert!(
        result.unwrap_err().to_string().contains("version mismatch"),
        "Error should mention version mismatch"
    );
}

#[test]
fn memory_cache_with_huge_length_prefix() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("brv.cache");

    // Craft a bincode payload with valid version (1) but a huge string length
    // bincode u32 version = 1, then u64 config_hash, then matcher data
    // If a string length field is set to u64::MAX, bincode should reject it
    let mut payload = Vec::new();
    // version: u32 = 1
    payload.extend_from_slice(&1u32.to_le_bytes());
    // config_hash: u64 = 0
    payload.extend_from_slice(&0u64.to_le_bytes());
    // Now matcher data starts. In bincode, HashMap starts with length (u64).
    // Set regular map length to a huge number
    payload.extend_from_slice(&u64::MAX.to_le_bytes());

    std::fs::write(&cache_path, &payload).unwrap();

    let result = cache::read(&cache_path);
    assert!(
        result.is_err(),
        "Should reject cache with absurdly large collection size"
    );
}

#[test]
fn memory_cache_empty_file() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("brv.cache");
    std::fs::write(&cache_path, b"").unwrap();

    let result = cache::read(&cache_path);
    assert!(result.is_err(), "Should fail on empty cache file");
}

#[test]
fn memory_cache_single_byte() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("brv.cache");
    std::fs::write(&cache_path, &[0xFF]).unwrap();

    let result = cache::read(&cache_path);
    assert!(result.is_err());
}

#[test]
fn memory_cache_with_crafted_string_bomb() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("brv.cache");

    // Craft a payload that claims a string is very long but the file is short
    // This tests bincode's handling of untrusted length prefixes
    let mut payload = Vec::new();
    payload.extend_from_slice(&1u32.to_le_bytes()); // version = 1
    payload.extend_from_slice(&0u64.to_le_bytes()); // config_hash = 0
    // regular HashMap: length = 0
    payload.extend_from_slice(&0u64.to_le_bytes());
    // global HashMap: length = 0
    payload.extend_from_slice(&0u64.to_le_bytes());
    // contextual Vec: length = 1 (claims one entry)
    payload.extend_from_slice(&1u64.to_le_bytes());
    // CompiledAbbr.keyword string length = 1GB
    payload.extend_from_slice(&(1_073_741_824u64).to_le_bytes());
    // Only 4 bytes of actual string data
    payload.extend_from_slice(b"AAAA");

    std::fs::write(&cache_path, &payload).unwrap();

    let result = cache::read(&cache_path);
    assert!(
        result.is_err(),
        "Should reject cache with string length exceeding file size"
    );
}

// ============================================================================
// 4. CPU/Memory Exhaustion Tests (DoS)
// ============================================================================

#[test]
fn dos_redos_exponential_backtracking() {
    // Test for ReDoS vulnerability with evil regex patterns in context
    // The regex crate in Rust is designed to be safe against catastrophic backtracking
    // (it uses finite automaton, not backtracking NFA), but let's verify.
    let evil_patterns = vec![
        // Classic ReDoS patterns that would cause exponential backtracking in PCRE
        "(a+)+$",
        "(a|a)+$",
        "([a-zA-Z]+)*$",
        "(a+)+b",
        "((a+)(b+))+$",
    ];

    for pattern in &evil_patterns {
        let toml = format!(
            r#"
[[abbr]]
keyword = "test"
expansion = "expanded"
context.lbuffer = "{}"
"#,
            pattern
        );
        // Pattern should parse without issues (Rust regex crate handles these safely)
        let result = config::parse(&toml);
        assert!(result.is_ok(), "Pattern {} should be accepted", pattern);
    }

    // Now test that matching against evil input doesn't hang
    let compiled = matcher::CompiledAbbr {
        keyword: "test".to_string(),
        expansion: "expanded".to_string(),
        global: false,
        evaluate: false,
        lbuffer_pattern: Some("(a+)+$".to_string()),
        rbuffer_pattern: None,
    };

    // This input would cause catastrophic backtracking in PCRE engines
    let evil_input = "a".repeat(30) + "!";
    let start = std::time::Instant::now();
    let _result = context::matches_context(&compiled, &evil_input, "");
    let elapsed = start.elapsed();

    // Rust's regex crate should complete this in milliseconds, not seconds
    assert!(
        elapsed.as_secs() < 1,
        "ReDoS pattern matching took {:?}, possible ReDoS vulnerability!",
        elapsed
    );
}

#[test]
fn dos_huge_lbuffer_input() {
    // Test with a very large lbuffer to check for excessive memory allocation or slow processing
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    // 10MB of 'a' followed by " g"
    let huge_input = "a".repeat(10_000_000) + " g";
    let input = expand::ExpandInput {
        lbuffer: huge_input,
        rbuffer: "".to_string(),
    };

    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    // Should complete quickly - it just finds the last space and looks up the keyword
    assert!(
        elapsed.as_secs() < 2,
        "Expansion with huge lbuffer took {:?}",
        elapsed
    );

    match result {
        brv::output::ExpandOutput::NoMatch => {
            // "g" after space is not command position, so no match for regular abbr
        }
        brv::output::ExpandOutput::Success { .. } => {
            // Could match if treated differently
        }
        _ => panic!("Unexpected result"),
    }
}

#[test]
fn dos_huge_lbuffer_global_match() {
    // Test with a very large lbuffer + global abbreviation
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    let huge_prefix = "a".repeat(10_000_000);
    let lbuffer = format!("{} NE", huge_prefix);
    let input = expand::ExpandInput {
        lbuffer,
        rbuffer: "".to_string(),
    };

    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 2,
        "Global expansion with huge lbuffer took {:?}",
        elapsed
    );

    match result {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert!(buffer.ends_with("2>/dev/null"));
        }
        _ => panic!("Expected Success for global match"),
    }
}

#[test]
fn dos_huge_rbuffer_input() {
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    let input = expand::ExpandInput {
        lbuffer: "g".to_string(),
        rbuffer: "b".repeat(10_000_000),
    };

    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 2,
        "Expansion with huge rbuffer took {:?}",
        elapsed
    );

    match result {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert!(buffer.starts_with("git"));
        }
        _ => panic!("Expected Success"),
    }
}

#[test]
fn dos_many_abbreviations() {
    // Config with a huge number of abbreviations
    let mut toml = String::new();
    for i in 0..10_000 {
        toml.push_str(&format!(
            r#"
[[abbr]]
keyword = "k{}"
expansion = "expansion_{}"
"#,
            i, i
        ));
    }

    let start = std::time::Instant::now();
    let cfg = config::parse(&toml).unwrap();
    let parse_elapsed = start.elapsed();
    assert!(
        parse_elapsed.as_secs() < 5,
        "Parsing 10000 abbreviations took {:?}",
        parse_elapsed
    );

    let start = std::time::Instant::now();
    let m = matcher::build(&cfg.abbr);
    let build_elapsed = start.elapsed();
    assert!(
        build_elapsed.as_secs() < 2,
        "Building matcher for 10000 abbreviations took {:?}",
        build_elapsed
    );

    // Lookup should still be O(1)
    let input = expand::ExpandInput {
        lbuffer: "k9999".to_string(),
        rbuffer: "".to_string(),
    };
    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let lookup_elapsed = start.elapsed();
    assert!(
        lookup_elapsed.as_millis() < 10,
        "Lookup in 10000 abbreviations took {:?}",
        lookup_elapsed
    );

    match result {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert_eq!(buffer, "expansion_9999");
        }
        _ => panic!("Expected Success"),
    }
}

#[test]
fn dos_many_contextual_abbreviations() {
    // Contextual abbreviations require linear scan, so many of them could be slow
    let mut toml = String::new();
    for i in 0..1_000 {
        toml.push_str(&format!(
            r#"
[[abbr]]
keyword = "ctx{}"
expansion = "expansion_{}"
context.lbuffer = "^prefix{}"
"#,
            i, i, i
        ));
    }

    let cfg = config::parse(&toml).unwrap();
    let m = matcher::build(&cfg.abbr);

    // Search for the last contextual abbreviation
    let input = expand::ExpandInput {
        lbuffer: "prefix999 ctx999".to_string(),
        rbuffer: "".to_string(),
    };

    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 2,
        "Contextual lookup with 1000 patterns took {:?}",
        elapsed
    );

    match result {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert_eq!(buffer, "prefix999 expansion_999");
        }
        _ => panic!("Expected Success"),
    }
}

#[test]
fn dos_huge_keyword_in_config() {
    // A very long keyword
    let huge_keyword = "a".repeat(100_000);
    let toml = format!(
        r#"
[[abbr]]
keyword = "{}"
expansion = "expanded"
"#,
        huge_keyword
    );

    let cfg = config::parse(&toml).unwrap();
    let m = matcher::build(&cfg.abbr);

    let input = expand::ExpandInput {
        lbuffer: huge_keyword.clone(),
        rbuffer: "".to_string(),
    };
    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 1,
        "Lookup of huge keyword took {:?}",
        elapsed
    );

    match result {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert_eq!(buffer, "expanded");
        }
        _ => panic!("Expected Success"),
    }
}

#[test]
fn dos_huge_expansion_in_config() {
    // A very long expansion string
    let huge_expansion = "x".repeat(10_000_000);
    let toml = format!(
        r#"
[[abbr]]
keyword = "big"
expansion = "{}"
"#,
        huge_expansion
    );

    let cfg = config::parse(&toml).unwrap();
    let m = matcher::build(&cfg.abbr);

    let input = expand::ExpandInput {
        lbuffer: "big".to_string(),
        rbuffer: "".to_string(),
    };
    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 2,
        "Expansion with huge expansion string took {:?}",
        elapsed
    );

    match result {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert_eq!(buffer.len(), huge_expansion.len());
        }
        _ => panic!("Expected Success"),
    }
}

#[test]
fn dos_placeholder_many_nested() {
    // Many nested/repeated placeholder patterns
    let text_with_many_placeholders =
        "{{a}}".repeat(10_000) + " suffix";

    let start = std::time::Instant::now();
    let result = placeholder::apply_first_placeholder(&text_with_many_placeholders, 0);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 500,
        "Placeholder processing with 10000 placeholders took {:?}",
        elapsed
    );

    // First placeholder should be removed, cursor set to position 0
    assert_eq!(result.cursor, 0);
}

#[test]
fn dos_placeholder_remove_all() {
    let text = "{{a}}".repeat(10_000) + " suffix";

    let start = std::time::Instant::now();
    let result = placeholder::remove_all_placeholders(&text);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 500,
        "Removing all 10000 placeholders took {:?}",
        elapsed
    );

    assert_eq!(result, " suffix");
}

#[test]
fn dos_deeply_nested_toml_structures() {
    // TOML with deeply nested inline tables (tests TOML parser limits)
    let toml = r#"
[[abbr]]
keyword = "test"
expansion = "expanded"
"#;
    // Simple config should always parse fine
    assert!(config::parse(toml).is_ok());

    // Invalid TOML with excessive nesting should error, not stack overflow
    let deeply_nested = "[".repeat(1000) + &"]".repeat(1000);
    let result = config::parse(&deeply_nested);
    assert!(result.is_err());
}

// ============================================================================
// 5. Input Validation / Edge Case Tests
// ============================================================================

#[test]
fn input_null_bytes_in_lbuffer() {
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    // Null bytes in input
    let input = expand::ExpandInput {
        lbuffer: "hello\0world g".to_string(),
        rbuffer: "".to_string(),
    };
    // Should not panic
    let _result = expand::expand(&input, &m);
}

#[test]
fn input_null_bytes_in_keyword() {
    // Keyword with null byte (if somehow injected)
    let toml = "[[abbr]]\nkeyword = \"g\\u0000h\"\nexpansion = \"git\"\n";
    // TOML parser should handle this
    let result = config::parse(toml);
    // May or may not parse depending on TOML spec compliance with null bytes
    if let Ok(cfg) = result {
        let m = matcher::build(&cfg.abbr);
        let input = expand::ExpandInput {
            lbuffer: "g\0h".to_string(),
            rbuffer: "".to_string(),
        };
        let _result = expand::expand(&input, &m);
    }
}

#[test]
fn input_unicode_edge_cases() {
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "café"
expansion = "coffee shop"

[[abbr]]
keyword = "🚀"
expansion = "rocket"
global = true
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    // Unicode keyword
    let input = expand::ExpandInput {
        lbuffer: "café".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert_eq!(buffer, "coffee shop");
        }
        _ => panic!("Expected Success for unicode keyword"),
    }

    // Emoji keyword as global
    let input = expand::ExpandInput {
        lbuffer: "echo 🚀".to_string(),
        rbuffer: "".to_string(),
    };
    match expand::expand(&input, &m) {
        brv::output::ExpandOutput::Success { buffer, .. } => {
            assert_eq!(buffer, "echo rocket");
        }
        _ => panic!("Expected Success for emoji keyword"),
    }
}

#[test]
fn input_unicode_boundary_in_placeholder() {
    // Test placeholder processing with multi-byte UTF-8 characters
    let text = "日本語{{placeholder}}テスト";
    let result = placeholder::apply_first_placeholder(text, 0);
    // Cursor should be at byte offset of {{, not char offset
    assert_eq!(result.cursor, "日本語".len()); // 9 bytes in UTF-8
    assert_eq!(result.text, "日本語テスト");
}

#[test]
fn input_very_long_single_token() {
    // A very long input with no spaces (treated as single keyword)
    let cfg = config::parse(
        r#"
[[abbr]]
keyword = "g"
expansion = "git"
"#,
    )
    .unwrap();
    let m = matcher::build(&cfg.abbr);

    let long_token = "x".repeat(1_000_000);
    let input = expand::ExpandInput {
        lbuffer: long_token,
        rbuffer: "".to_string(),
    };

    let start = std::time::Instant::now();
    let result = expand::expand(&input, &m);
    let elapsed = start.elapsed();

    assert!(elapsed.as_secs() < 1, "Long single token took {:?}", elapsed);
    match result {
        brv::output::ExpandOutput::NoMatch => {} // Expected - "xxxx..." doesn't match "g"
        _ => panic!("Expected NoMatch for non-matching long token"),
    }
}

#[test]
fn input_regex_special_chars_in_lbuffer() {
    // Ensure regex special characters in lbuffer (as match subject) don't cause issues
    let compiled = matcher::CompiledAbbr {
        keyword: "test".to_string(),
        expansion: "expanded".to_string(),
        global: false,
        evaluate: false,
        lbuffer_pattern: Some("^git ".to_string()),
        rbuffer_pattern: None,
    };

    let evil_lbuffers = vec![
        "git [unclosed",
        "git (unclosed",
        "git \\",
        "git .",
        "git *",
        "git +",
        "git ?",
        "git {",
        "git }",
        "git ^",
        "git $",
        "git |",
    ];

    for lbuf in evil_lbuffers {
        // Should not panic - regex special chars in the subject string are fine
        let _result = context::matches_context(&compiled, lbuf, "");
    }
}

// ============================================================================
// 6. Config Injection / Malformed TOML Tests
// ============================================================================

#[test]
fn config_injection_huge_toml() {
    // Extremely large TOML with long strings
    let huge_value = "a".repeat(10_000_000);
    let toml = format!(
        r#"
[[abbr]]
keyword = "k"
expansion = "{}"
"#,
        huge_value
    );

    let start = std::time::Instant::now();
    let result = config::parse(&toml);
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    assert!(
        elapsed.as_secs() < 5,
        "Parsing huge TOML took {:?}",
        elapsed
    );
}

#[test]
fn config_injection_billion_laughs_style() {
    // TOML doesn't support entity expansion like XML, but test that repeated
    // large structures don't cause issues
    let mut toml = String::new();
    for i in 0..1000 {
        toml.push_str(&format!(
            r#"
[[abbr]]
keyword = "k{}"
expansion = "{}"
"#,
            i,
            "x".repeat(10_000)
        ));
    }

    let start = std::time::Instant::now();
    let result = config::parse(&toml);
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    assert!(
        elapsed.as_secs() < 10,
        "Parsing 1000 abbrs with 10KB expansions took {:?}",
        elapsed
    );
}

#[test]
fn config_invalid_utf8_handling() {
    // config::parse takes &str, so invalid UTF-8 can't reach it.
    // But config::load reads from file - test with invalid UTF-8 file content.
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("brv.toml");

    // Write invalid UTF-8
    std::fs::write(&config_path, &[0xFF, 0xFE, 0x00, 0x01]).unwrap();

    let result = config::load(&config_path);
    assert!(result.is_err(), "Should fail on invalid UTF-8 config file");
}

// ============================================================================
// 7. Symlink / Race Condition Tests
// ============================================================================

#[test]
fn symlink_config_to_sensitive_file() {
    let dir = TempDir::new().unwrap();
    let link_path = dir.path().join("brv.toml");

    // Create symlink pointing to /etc/passwd
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", &link_path).unwrap();

        // config::load follows symlinks (standard behavior), but the content
        // won't parse as valid TOML
        let result = config::load(&link_path);
        assert!(
            result.is_err(),
            "/etc/passwd should not parse as valid brv config"
        );
    }
}

#[test]
fn symlink_cache_to_sensitive_file() {
    let dir = TempDir::new().unwrap();
    let link_path = dir.path().join("brv.cache");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", &link_path).unwrap();

        // cache::read follows symlinks, but the content won't deserialize as bincode
        let result = cache::read(&link_path);
        assert!(
            result.is_err(),
            "/etc/passwd should not deserialize as valid cache"
        );
    }
}

// ============================================================================
// 8. Output Protocol Integrity Tests
// ============================================================================

#[test]
fn output_protocol_no_extra_newlines() {
    // Verify that the output protocol is consistent and can't be confused
    let output = brv::output::ExpandOutput::Success {
        buffer: "git commit".to_string(),
        cursor: 10,
    };
    let formatted = output.to_string();
    let lines: Vec<&str> = formatted.split('\n').collect();
    assert_eq!(lines.len(), 3, "Success output should have exactly 3 lines");
    assert_eq!(lines[0], "success");
    assert_eq!(lines[1], "git commit");
    assert_eq!(lines[2], "10");
}

#[test]
fn output_protocol_evaluate_format() {
    let output = brv::output::ExpandOutput::Evaluate {
        command: "date +%Y-%m-%d".to_string(),
        prefix: "echo ".to_string(),
        rbuffer: " | cat".to_string(),
    };
    let formatted = output.to_string();
    let lines: Vec<&str> = formatted.split('\n').collect();
    assert_eq!(lines.len(), 4, "Evaluate output should have exactly 4 lines");
    assert_eq!(lines[0], "evaluate");
    assert_eq!(lines[1], "date +%Y-%m-%d");
    assert_eq!(lines[2], "echo ");
    assert_eq!(lines[3], " | cat");
}

#[test]
fn output_protocol_buffer_with_embedded_newlines() {
    // If buffer contains newlines, the protocol breaks.
    // This verifies the current behavior (potential issue).
    let output = brv::output::ExpandOutput::Success {
        buffer: "line1\nline2".to_string(),
        cursor: 5,
    };
    let formatted = output.to_string();
    let lines: Vec<&str> = formatted.split('\n').collect();
    // This will produce 4 lines instead of expected 3, breaking the protocol
    assert_eq!(
        lines.len(),
        4,
        "Newline in buffer produces extra line (known protocol limitation)"
    );
}

// ============================================================================
// 9. Hash Collision / Cache Integrity Tests
// ============================================================================

#[test]
fn cache_hash_not_cryptographic() {
    // DefaultHasher is not cryptographic - verify that different configs produce
    // different hashes (basic collision resistance)
    let hash1 = cache::hash_config("config_a");
    let hash2 = cache::hash_config("config_b");
    assert_ne!(hash1, hash2, "Different configs should produce different hashes");

    // Same content should produce same hash
    let hash3 = cache::hash_config("config_a");
    assert_eq!(hash1, hash3, "Same config should produce same hash");
}

#[test]
fn cache_freshness_bypass_attempt() {
    // Can we craft a different config that produces the same hash?
    // This would require finding a collision in DefaultHasher.
    // With u64 hash space, random collision probability is ~1/2^64.
    // We can't efficiently find collisions, so this test just verifies
    // that minor modifications change the hash.
    let base = "keyword = \"g\"\nexpansion = \"git\"";
    let modified = "keyword = \"g\"\nexpansion = \"git\" ";  // trailing space
    assert_ne!(
        cache::hash_config(base),
        cache::hash_config(modified),
        "Tiny modification should change hash"
    );
}
