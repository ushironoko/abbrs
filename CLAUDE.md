# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

abbrs is a fast, safe zsh abbreviation expansion tool written in Rust.
It uses a compile-then-expand architecture: `abbrs compile` validates abbreviations against PATH commands and zsh builtins, then generates a binary cache (bitcode). `abbrs expand` reads the cache for O(1) HashMap lookup at runtime. A socket-based daemon mode (`abbrs serve`) eliminates per-invocation fork+exec overhead for sub-millisecond latency.

## Build & Development Commands

```bash
cargo build                  # Debug build
cargo build --release        # Release build (strip + thin LTO + opt-level 3)
cargo test                   # Run all unit + integration tests
cargo test <test_name>       # Run a single test
cargo bench                  # Run criterion benchmarks
```

Rust edition 2021, minimum supported Rust version: 1.80+.

## Architecture

Two-phase design: **compile** (offline validation + cache generation) and **expand** (runtime lookup from cache).

```
abbrs.toml → abbrs compile → conflict detection → abbrs.cache (bitcode v4)
                                                        ↓
                ZLE widget ← stdout protocol ← abbrs expand (HashMap lookup)
                     ↑                              ↑
                     └── Unix socket ←── abbrs serve (daemon, persistent matcher)
```

### Expansion priority

Contextual (regex match) > Command-scoped > Regular (command position only) > Global (any position) > Regex keywords > Prefix match fallback.

### ZLE output protocol

abbrs communicates with the zsh widget (`shells/zsh/abbrs.zsh`) via a line-based stdout protocol:
- `success\n{buffer}\n{cursor}` — expanded text with cursor position
- `evaluate\n{command}\n{prefix}\n{rbuffer}` — shell eval required
- `function\n{function_name}\n{matched_token}\n{prefix}\n{rbuffer}` — shell function call required
- `candidates\n{count}\n{keyword}\t{expansion}\n...` — multiple prefix-match candidates found (newlines/tabs escaped)
- `stale_cache` — triggers auto-recompile in widget
- `no_match` — fallback to normal key behavior

### Serve mode protocol

The daemon (`abbrs serve`) communicates over a Unix domain socket at `/tmp/abbrs-{uid}/abbrs.sock`. Requests are tab-separated with `\x1e` (record separator) as end-of-record:
- `expand\t{lbuffer}\t{rbuffer}` — abbreviation expansion
- `placeholder\t{lbuffer}\t{rbuffer}` — placeholder navigation
- `remind\t{buffer}` — abbreviation reminder check
- `reload` — force config reload
- `ping` — health check

The daemon auto-reloads when config file mtime changes.

### Cache freshness

Cache stores a hash of config file content. `abbrs expand` checks freshness on every invocation; if stale, returns `stale_cache` and the zsh widget runs `abbrs compile` then retries.

## Module Responsibilities

- **main.rs** — CLI entry point (clap derive). Routes to ~15 subcommands.
- **compiler.rs** — Orchestrates the compile pipeline: config parse → PATH scan → conflict detect → matcher build → cache write.
- **conflict.rs** — PATH scanning, zsh builtin list (92 commands, binary search), three conflict types: ExactPathMatch, ShellBuiltin, DuplicateKeyword.
- **matcher.rs** — `Matcher` struct with `FxHashMap` indices for regular, global, command-scoped, contextual lookups. Also maintains `prefix_index` (O(1) prefix matching) and `remind_index` (expansion → keywords).
- **expand.rs** — Keyword extraction from lbuffer, command position detection, quote-aware segment parsing (pipes, semicolons, `&&`, `||`), lookup priority chain.
- **context.rs** — Regex-based lbuffer/rbuffer context matching with lazy-compiled `RegexCache`.
- **placeholder.rs** — `{{name}}` placeholder removal and cursor positioning.
- **cache.rs** — bitcode serialize/deserialize with version check (current: v4). Uses `DefaultHasher` for config content hashing.
- **output.rs** — `ExpandOutput` / `PlaceholderOutput` enums with `Display` impl for the stdout protocol.
- **config.rs** — TOML deserialization, validation rules (including regex pattern validation), XDG path resolution.
- **add.rs** — `abbrs add` command with interactive terminal UI (crossterm). Pre-write validation and scope-aware duplicate detection.
- **manage.rs** — `abbrs erase`, `rename`, `query`, `show` commands. Uses `toml_edit` for format-preserving TOML modifications.
- **import.rs** — Import from zsh aliases, fish abbreviations, and git aliases. Export support. Auto-marks `allow_conflict = true` for conflicting imports.
- **serve.rs** — Socket-based daemon mode. Private socket directory per UID (`/tmp/abbrs-{uid}/`). Auto-reload on config mtime change.

## Test Structure

Integration tests in `tests/`:
- **cli_test.rs** — End-to-end CLI subcommand tests
- **compile_test.rs** — Compile pipeline and validation tests
- **expand_test.rs** — Expansion priority chain and edge case tests
- **serve_test.rs** — Daemon mode and socket communication tests

Benchmarks in `benches/` (criterion):
- **expansion.rs** — Lookup latency benchmarks
- **config_loading.rs** — TOML parsing benchmarks
- **conflict_check.rs** — PATH scanning benchmarks

## CI/CD

GitHub Actions release pipeline (`.github/workflows/release.yml`):
- Stages: validate → test → build (multi-platform) → release → publish → rollback
- Platforms: x86_64-linux, aarch64-linux, x86_64-macos, aarch64-macos
- All action versions pinned to commit SHA
- Trusted publishing via OIDC for crates.io

## Dependency Version Policy

Always specify exact versions in Cargo.toml (no `^` or `*`).

## Key Dependencies

- **clap** — CLI argument parsing (derive macros)
- **bitcode** — Binary cache serialization
- **rustc-hash** — FxHashMap for fast non-cryptographic hashing
- **regex** — Pattern matching for contextual/regex abbreviations
- **toml / toml_edit** — Config reading (serde) and format-preserving editing
- **crossterm** — Terminal UI for interactive `abbrs add`
- **xdg** — XDG Base Directory path resolution
