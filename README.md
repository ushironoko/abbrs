# abbrs

Fast and safe abbreviation expansion for zsh.

abbrs pre-validates your abbreviations at compile time — catching conflicts with existing commands before they cause problems — then uses a binary cache for instant expansion at runtime.

## What's abbrs

Traditional zsh abbreviation tools expand keywords at runtime without checking whether they collide with real commands in your `$PATH` or zsh builtins — you only discover the conflict when something breaks. They also rely on shell-script lookups that slow down linearly as your abbreviation list grows.

abbrs takes a different approach: **compile, then expand**. Running `abbrs compile` scans your PATH and builtins, rejects dangerous conflicts up front, and writes a binary cache. At expansion time, abbrs reads that cache for O(1) HashMap lookup — or runs as a persistent daemon (`abbrs serve`) communicating via Unix domain socket for sub-100 µs latency, regardless of how many abbreviations you have.

## Features

- **Compile-time safety** — Detects conflicts with PATH commands and zsh builtins before they cause problems
- **Sub-100 µs expansion** — Persistent daemon mode via Unix domain socket + binary cache (bitcode) for imperceptible latency
- **Layered expansion priority** — Contextual > Command-scoped > Regular > Global > Regex keywords > Prefix candidates
- **Multiple expansion modes** — Replace, Evaluate (shell command output), Function call, and Placeholder (cursor positioning)
- **Auto-recompilation** — Detects config changes automatically; no manual recompile needed
- **Prefix candidates** — Partial keyword input shows matching abbreviations as you type
- **Abbreviation reminders** — Notifies you when a shorter form was available for what you typed
- **Zero-friction migration** — Import from zsh aliases, fish abbreviations, and git aliases

## Installation

### From crates.io

```bash
cargo install abbrs
```

### From GitHub Releases (via mise)

```bash
mise install github:ushironoko/abbrs
```

### Build from source

```bash
cargo install --path .
```

### Setup

1. Generate a config file:

```bash
abbrs init config
```

This creates `~/.config/abbrs/abbrs.toml`.

2. Add the zsh integration to your `.zshrc`:

```bash
eval "$(abbrs init zsh)"
```

3. Compile your config:

```bash
abbrs compile
```

## Migrating from Aliases

### From zsh aliases

Pipe the output of `alias` into `abbrs import aliases`:

```bash
alias | abbrs import aliases
```

This parses each `alias name='expansion'` line and appends it to your `abbrs.toml`. Aliases that conflict with PATH commands are automatically marked with `allow_conflict = true`.

### From fish abbreviations

Pipe the output of `abbr` into `abbrs import fish`, or pass a file path:

```bash
fish -c "abbr" | abbrs import fish
# or
abbrs import fish /path/to/abbreviations.txt
```

### From git aliases

```bash
abbrs import git-aliases
```

## Configuration

Edit `~/.config/abbrs/abbrs.toml` to define your abbreviations.

### Regular Abbreviations

Expand only at command position (the beginning of a command):

```toml
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit"
```

Typing `g` then pressing Space expands to `git `. But `echo g` does not expand, because `g` is not in command position.

### Global Abbreviations

Expand anywhere in the line:

```toml
[[abbr]]
keyword = "NE"
expansion = "2>/dev/null"
global = true
```

`curl example.com NE` expands to `curl example.com 2>/dev/null`.

### Contextual Abbreviations

Expand only when surrounding text matches regex patterns:

```toml
[[abbr]]
keyword = "main"
expansion = "main --branch"
context.lbuffer = "^git (checkout|switch) "
```

`main` expands to `main --branch` only after `git checkout` or `git switch`.

### Placeholders

Use `{{name}}` to mark positions where you want to type after expansion:

```toml
[[abbr]]
keyword = "gc"
expansion = "git commit -m '{{message}}'"
```

`gc` expands to `git commit -m ''` with the cursor placed between the quotes. Press Tab to jump to the next placeholder if there are multiple.

### Evaluate Mode

Execute a shell command and insert its output:

```toml
[[abbr]]
keyword = "TODAY"
expansion = "date +%Y-%m-%d"
evaluate = true
global = true
```

`TODAY` expands to the current date, e.g. `2026-03-08`.

### Command-Scoped Abbreviations

Expand only after a specific command:

```toml
[[abbr]]
keyword = "co"
expansion = "checkout"
command = "git"
```

`git co` expands to `git checkout`, but `co` alone does not expand.

### Function Mode

Run expansion as a shell function:

```toml
[[abbr]]
keyword = "mf"
expansion = "my_func"
function = true
```

### Regex Keywords

Use a regex pattern as the keyword:

```toml
[[abbr]]
keyword = "^g[0-9]$"
expansion = "git"
regex = true
```

### Settings

```toml
[settings]
# prefixes = ["sudo", "doas"]  # commands that preserve command position
# remind = false  # remind when abbreviation could have been used
# serve = true  # use persistent daemon mode for sub-100µs latency (set false to use per-process fallback)
```

## Conflict Detection

When you run `abbrs compile`, abbrs scans your `$PATH` and checks zsh builtins to detect abbreviations that shadow existing commands.

| Conflict Type | Behavior |
|---|---|
| Exact match with a command in `$PATH` | Error |
| zsh builtin (e.g. `cd`, `echo`) | Error |

To allow a specific conflict:

```toml
[[abbr]]
keyword = "gs"
expansion = "git status --short"
allow_conflict = true
```

## Key Bindings

The zsh integration sets up the following key bindings:

| Key | Action |
|---|---|
| Space | Expand abbreviation, then insert space. While prefix candidates are shown, confirm current selection and expand |
| Enter | Expand abbreviation, then execute. While prefix candidates are shown, confirm current selection and execute |
| Tab | Cycle through prefix candidates when shown; otherwise jump to next `{{placeholder}}` (falls back to normal completion) |
| Ctrl+Space | Insert a literal space (no expansion). While prefix candidates are shown, cancel and restore original input |
| accept-line | Check for abbreviation reminders (when `remind = true`) |

## Prefix Candidates

When you type a partial keyword and press Space, abbrs shows matching abbreviations as candidates if no exact match is found.

For example, with these abbreviations defined:

```toml
[[abbr]]
keyword = "gc"
expansion = "git commit"

[[abbr]]
keyword = "gp"
expansion = "git push"

[[abbr]]
keyword = "gd"
expansion = "git diff"
```

Typing `g` then pressing Space displays:

```
  gc → git commit
  gp → git push
  gd → git diff
```

Space is **not** inserted — you continue typing to narrow down the candidates. Typing `gc` then pressing Space expands to `git commit` as usual.

Candidates respect abbreviation scope:

- At **command position**: regular, global, and command-scoped abbreviations are shown
- At **argument position**: only global and matching command-scoped abbreviations are shown

The prefix index is built automatically during `abbrs compile` — no extra configuration needed. Candidates are shown only when 2 or more matches exist.

## Adding Abbreviations from the CLI

Instead of editing `abbrs.toml` by hand, you can use `abbrs add`:

### Non-interactive

```bash
abbrs add g "git"
abbrs add gc "git commit -m '{{message}}'" --global
abbrs add main "main --branch" --context-lbuffer "^git (checkout|switch) "
abbrs add TODAY "date +%Y-%m-%d" --evaluate --global
abbrs add gs "git status --short" --allow-conflict
```

| Flag | Description |
|---|---|
| `--global` | Register as a global abbreviation |
| `--evaluate` | Run expansion as a shell command |
| `--function` | Run expansion as a shell function |
| `--regex` | Keyword is a regex pattern |
| `--command <CMD>` | Only expand as argument of this command |
| `--allow-conflict` | Allow conflicts with PATH commands |
| `--context-lbuffer <REGEX>` | Left-buffer regex for context matching |
| `--context-rbuffer <REGEX>` | Right-buffer regex for context matching |
| `--config <PATH>` | Use a custom config file path |

### Interactive

Run `abbrs add` without arguments to enter interactive mode:

```bash
abbrs add
```

You will be prompted for the keyword, expansion, type (regular / global / context), and other options.

## Commands

| Command | Description |
|---|---|
| `abbrs init config` | Generate a config template at `~/.config/abbrs/abbrs.toml` |
| `abbrs init zsh` | Output zsh integration script (usage: `eval "$(abbrs init zsh)"`) |
| `abbrs add` | Add an abbreviation interactively |
| `abbrs add <keyword> <expansion>` | Add an abbreviation with options |
| `abbrs erase <keyword>` | Erase an abbreviation from config (`--command`, `--global` to target specific entries) |
| `abbrs rename <old> <new>` | Rename an abbreviation keyword (`--command`, `--global` to target specific entries) |
| `abbrs query <keyword>` | Check if an abbreviation exists (`--command`, `--global` to target specific entries) |
| `abbrs show [keyword]` | Show abbreviations in re-importable `abbrs add` format |
| `abbrs compile` | Validate config, detect conflicts, and generate binary cache |
| `abbrs check` | Validate config syntax without compiling |
| `abbrs list` | Show all registered abbreviations |
| `abbrs import aliases` | Import from zsh aliases (stdin) |
| `abbrs import fish [file]` | Import from fish abbreviations |
| `abbrs import git-aliases` | Import from git aliases |
| `abbrs export` | Export abbreviations in `abbrs add` format |
| `abbrs remind` | Check for abbreviation reminders (called by ZLE) |
| `abbrs expand` | Expand an abbreviation (called by the zsh widget) |
| `abbrs next-placeholder` | Jump to next placeholder (called by the zsh widget) |
| `abbrs serve` | Start persistent daemon mode (Unix domain socket) for sub-100µs expansion latency |

## Auto-Recompilation

When you edit `abbrs.toml`, the next expansion automatically detects the stale cache and recompiles. No manual `abbrs compile` needed after config changes.

## Performance

abbrs is designed for imperceptible expansion latency. Below are benchmark results comparing abbrs with [zsh-abbr](https://github.com/olets/zsh-abbr).

### Architecture comparison

| | abbrs | zsh-abbr |
|---|---|---|
| Language | Rust (compiled binary) | Zsh (shell script) |
| Data structure | `FxHashMap` (O(1) lookup) | Zsh associative array |
| Invocation | External process / daemon via Unix domain socket (`abbrs serve`) | In-process function call |
| Cache format | bitcode (binary) | Plain text files |

### Expansion lookup (in-process, criterion)

The core HashMap lookup scales O(1) regardless of abbreviation count:

| Abbreviation count | Lookup time |
|---|---|
| 10 | 75 ns |
| 100 | 75 ns |
| 500 | 77 ns |
| 1,000 | 77 ns |

### End-to-end expansion latency

Measured with the comparison benchmark (`benchmarks/comparison/bench.zsh`, 1000 iterations per measurement):

| Abbreviation count | abbrs expand | abbrs serve (socket) | zsh-abbr |
|---|---|---|---|
| 10 | ~1.0 ms | ~0.05 ms | ~0.07 ms |
| 50 | ~1.0 ms | ~0.05 ms | ~0.12 ms |
| 100 | ~1.0 ms | ~0.05 ms | ~0.18 ms |
| 500 | ~1.1 ms | ~0.06 ms | ~0.70 ms |

> **Note:** `abbrs expand` includes fork+exec overhead (~1 ms), which dominates the actual lookup time. `abbrs serve` eliminates this by running as a persistent daemon, communicating via Unix domain socket — achieving **sub-100µs** latency that is faster than zsh-abbr at any scale.

### Other operations (criterion)

| Operation | Time |
|---|---|
| Global expansion (100 abbrs) | 81 ns |
| Placeholder expansion | 123 ns |
| Contextual expansion (50 regex patterns) | 27 µs |
| Cache read (100 abbrs, bitcode) | 62 µs |
| Cache read (500 abbrs, bitcode) | 297 µs |
| Config parse (100 abbrs, TOML) | 150 µs |

### Run benchmarks yourself

```bash
# Criterion microbenchmarks (Rust)
cargo bench

# End-to-end comparison with zsh-abbr (requires zsh + zsh-abbr installed)
zsh benchmarks/comparison/bench.zsh [iterations]
```

## License

MIT
