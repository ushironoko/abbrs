# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

abbrs is a fast, safe zsh abbreviation expansion tool written in Rust.
It uses a compile-then-expand architecture: `abbrs compile` validates abbreviations against PATH commands and zsh builtins, then generates a binary cache (bitcode). `abbrs expand` reads the cache for O(1) HashMap lookup at runtime.

## Build & Development Commands

```bash
cargo build                  # Debug build
cargo build --release        # Release build (strip + LTO)
cargo test                   # Run all unit + integration tests
cargo test <test_name>       # Run a single test
cargo bench                  # Run criterion benchmarks
```

## Architecture

Two-phase design: **compile** (offline validation + cache generation) and **expand** (runtime lookup from cache).

```
abbrs.toml → abbrs compile → conflict detection → abbrs.cache (bitcode)
                                                        ↓
                ZLE widget ← stdout protocol ← abbrs expand (HashMap lookup)
```

### Expansion priority

Contextual (regex match) > Command-scoped > Regular (command position only) > Global (any position) > Regex keywords > Prefix match fallback.

### ZLE output protocol

abbrs communicates with the zsh widget (`shells/zsh/abbrs.zsh`) via a line-based stdout protocol:
- `success\n{buffer}\n{cursor}` — expanded text with cursor position
- `evaluate\n{command}\n{prefix}\n{rbuffer}` — shell eval required
- `function\n{function_name}\n{matched_token}\n{prefix}\n{rbuffer}` — shell function call required
- `candidates\n{count}\n{keyword}\t{expansion}\n...` — multiple prefix-match candidates found
- `stale_cache` — triggers auto-recompile in widget
- `no_match` — fallback to normal key behavior

### Cache freshness

Cache stores a hash of config file content. `abbrs expand` checks freshness on every invocation; if stale, returns `stale_cache` and the zsh widget runs `abbrs compile` then retries.

## Module Responsibilities

- **main.rs** — CLI entry point (clap). All subcommand handlers live here.
- **compiler.rs** — Orchestrates the compile pipeline: config parse → PATH scan → conflict detect → matcher build → cache write.
- **conflict.rs** — PATH scanning, zsh builtin list (~90 commands), two conflict types (exact PATH match/builtin).
- **matcher.rs** — `Matcher` struct with `FxHashMap<String, Vec<CompiledAbbr>>` for regular/global, `Vec<CompiledAbbr>` for contextual.
- **expand.rs** — Keyword extraction from lbuffer, command position detection, lookup priority chain.
- **context.rs** — Regex-based lbuffer/rbuffer context matching for contextual abbreviations.
- **placeholder.rs** — `{{name}}` placeholder removal and cursor positioning.
- **cache.rs** — bitcode serialize/deserialize with version check (current: v4).
- **output.rs** — `ExpandOutput` / `PlaceholderOutput` enums with `Display` impl for the stdout protocol.
- **config.rs** — TOML deserialization, validation rules, XDG path resolution.

## Dependency Version Policy

Always specify exact versions in Cargo.toml (no `^` or `*`).
