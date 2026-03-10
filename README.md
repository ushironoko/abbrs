# kort

Fast and safe abbreviation expansion for zsh.

kort pre-validates your abbreviations at compile time — catching conflicts with existing commands before they cause problems — then uses a binary cache for instant expansion at runtime.

## Installation

### From crates.io

```bash
cargo install kort
```

### From GitHub Releases (via mise)

```bash
mise install github:ushironoko/kort
```

### Build from source

```bash
cargo install --path .
```

### Setup

1. Generate a config file:

```bash
kort init config
```

This creates `~/.config/kort/kort.toml`.

2. Add the zsh integration to your `.zshrc`:

```bash
eval "$(kort init zsh)"
```

3. Compile your config:

```bash
kort compile
```

## Migrating from Aliases

### From zsh aliases

Pipe the output of `alias` into `kort import aliases`:

```bash
alias | kort import aliases
```

This parses each `alias name='expansion'` line and appends it to your `kort.toml`. Aliases that conflict with PATH commands are automatically marked with `allow_conflict = true`.

### From fish abbreviations

```bash
kort import fish
```

### From git aliases

```bash
kort import git-aliases
```

## Configuration

Edit `~/.config/kort/kort.toml` to define your abbreviations.

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
```

## Conflict Detection

When you run `kort compile`, kort scans your `$PATH` and checks zsh builtins to detect abbreviations that shadow existing commands.

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
| Space | Expand abbreviation, then insert space |
| Enter | Expand abbreviation, then execute |
| Tab | Jump to next `{{placeholder}}` (falls back to normal completion) |
| accept-line | Check for abbreviation reminders (when `remind = true`) |

## Prefix Candidates

When you type a partial keyword and press Space, kort shows matching abbreviations as candidates if no exact match is found.

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

The prefix index is built automatically during `kort compile` — no extra configuration needed. Candidates are shown only when 2 or more matches exist.

## Adding Abbreviations from the CLI

Instead of editing `kort.toml` by hand, you can use `kort add`:

### Non-interactive

```bash
kort add g "git"
kort add gc "git commit -m '{{message}}'" --global
kort add main "main --branch" --context-lbuffer "^git (checkout|switch) "
kort add TODAY "date +%Y-%m-%d" --evaluate --global
kort add gs "git status --short" --allow-conflict
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

Run `kort add` without arguments to enter interactive mode:

```bash
kort add
```

You will be prompted for the keyword, expansion, type (regular / global / context), and other options.

## Commands

| Command | Description |
|---|---|
| `kort init config` | Generate a config template at `~/.config/kort/kort.toml` |
| `kort init zsh` | Output zsh integration script (usage: `eval "$(kort init zsh)"`) |
| `kort add` | Add an abbreviation interactively |
| `kort add <keyword> <expansion>` | Add an abbreviation with options |
| `kort erase <keyword>` | Erase an abbreviation from config |
| `kort rename <old> <new>` | Rename an abbreviation keyword |
| `kort query <keyword>` | Check if an abbreviation exists (exit code 0 = found) |
| `kort show [keyword]` | Show abbreviations in re-importable `kort add` format |
| `kort compile` | Validate config, detect conflicts, and generate binary cache |
| `kort check` | Validate config syntax without compiling |
| `kort list` | Show all registered abbreviations |
| `kort import aliases` | Import from zsh aliases (stdin) |
| `kort import fish [file]` | Import from fish abbreviations |
| `kort import git-aliases` | Import from git aliases |
| `kort export` | Export abbreviations in `kort add` format |
| `kort remind` | Check for abbreviation reminders (called by ZLE) |
| `kort expand` | Expand an abbreviation (called by the zsh widget) |
| `kort next-placeholder` | Jump to next placeholder (called by the zsh widget) |

## Auto-Recompilation

When you edit `kort.toml`, the next expansion automatically detects the stale cache and recompiles. No manual `kort compile` needed after config changes.

## Performance

kort is designed for imperceptible expansion latency. Below are benchmark results comparing kort with [zsh-abbr](https://github.com/olets/zsh-abbr).

### Architecture comparison

| | kort | zsh-abbr |
|---|---|---|
| Language | Rust (compiled binary) | Zsh (shell script) |
| Data structure | `FxHashMap` (O(1) lookup) | Zsh associative array |
| Invocation | External process / coproc (`kort serve`) | In-process function call |
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

| Abbreviation count | kort expand | kort serve (coproc) | zsh-abbr |
|---|---|---|---|
| 10 | ~1.0 ms | ~0.05 ms | ~0.07 ms |
| 50 | ~1.0 ms | ~0.05 ms | ~0.12 ms |
| 100 | ~1.0 ms | ~0.05 ms | ~0.18 ms |
| 500 | ~1.1 ms | ~0.06 ms | ~0.70 ms |

> **Note:** `kort expand` includes fork+exec overhead (~1 ms), which dominates the actual lookup time. `kort serve` eliminates this by running as a persistent coproc, communicating via pipe — achieving **sub-100µs** latency that is faster than zsh-abbr at any scale.

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
