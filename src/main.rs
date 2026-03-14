use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use abbrs::{add, cache, compiler, config, context, expand, history, import, manage, output, placeholder, serve};

#[derive(Parser, Debug)]
#[command(name = "abbrs")]
#[command(about = "Fast and safe abbreviation expansion for zsh")]
#[command(version)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Compile config and verify conflicts
    Compile {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Expand abbreviation (called from ZLE)
    Expand {
        /// Buffer left of cursor
        #[arg(long)]
        lbuffer: String,

        /// Buffer right of cursor
        #[arg(long)]
        rbuffer: String,

        /// Cache file path
        #[arg(long)]
        cache: Option<PathBuf>,

        /// Config file path (for freshness check)
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Jump to next placeholder
    NextPlaceholder {
        /// Buffer left of cursor
        #[arg(long)]
        lbuffer: String,

        /// Buffer right of cursor
        #[arg(long)]
        rbuffer: String,
    },

    /// List registered abbreviations
    List {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Syntax check config only
    Check {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Initialize abbrs (shell integration or config)
    Init {
        #[command(subcommand)]
        target: InitTarget,
    },

    /// Add a new abbreviation to config
    Add {
        /// Keyword to abbreviate (if omitted, interactive mode)
        #[arg(value_name = "KEYWORD")]
        keyword: Option<String>,

        /// Expansion text
        #[arg(value_name = "EXPANSION")]
        expansion: Option<String>,

        /// Register as global abbreviation
        #[arg(long, default_value = "false")]
        global: bool,

        /// Run expansion as command (evaluate mode)
        #[arg(long, default_value = "false")]
        evaluate: bool,

        /// Run expansion as shell function
        #[arg(long, default_value = "false")]
        function: bool,

        /// Keyword is a regex pattern
        #[arg(long, default_value = "false")]
        regex: bool,

        /// Only expand as argument of this command
        #[arg(long)]
        command: Option<String>,

        /// Allow conflict with PATH commands
        #[arg(long, default_value = "false")]
        allow_conflict: bool,

        /// Context lbuffer regex pattern
        #[arg(long)]
        context_lbuffer: Option<String>,

        /// Context rbuffer regex pattern
        #[arg(long)]
        context_rbuffer: Option<String>,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Erase an abbreviation from config
    Erase {
        /// Keyword to erase
        keyword: String,

        /// Only erase command-scoped entry for this command
        #[arg(long)]
        command: Option<String>,

        /// Only erase global entry
        #[arg(long, default_value = "false")]
        global: bool,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Rename an abbreviation keyword
    Rename {
        /// Current keyword
        old: String,

        /// New keyword
        new: String,

        /// Only rename command-scoped entry for this command
        #[arg(long)]
        command: Option<String>,

        /// Only rename global entry
        #[arg(long, default_value = "false")]
        global: bool,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Query if an abbreviation exists (exit code 0 = found)
    Query {
        /// Keyword to query
        keyword: String,

        /// Only query command-scoped entry for this command
        #[arg(long)]
        command: Option<String>,

        /// Only query global entry
        #[arg(long, default_value = "false")]
        global: bool,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Show abbreviations in re-importable format
    Show {
        /// Filter by keyword
        keyword: Option<String>,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Check for abbreviation reminders (called from ZLE accept-line)
    Remind {
        /// The full buffer being executed
        #[arg(long)]
        buffer: String,

        /// Cache file path
        #[arg(long)]
        cache: Option<PathBuf>,
    },

    /// Import abbreviations from external sources
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },

    /// Export abbreviations in `abbrs add` format
    Export {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// List keywords for shell completion (internal use)
    #[command(hide = true, name = "_list-keywords")]
    ListKeywords {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Check if serve mode is enabled in config (internal use)
    #[command(hide = true, name = "_serve-enabled")]
    ServeEnabled {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Manage expansion history
    History {
        #[command(subcommand)]
        action: HistoryAction,
    },

    /// Start long-running serve mode (pipe or socket communication)
    Serve {
        /// Unix domain socket path (if omitted, uses stdin/stdout pipe mode)
        #[arg(long)]
        socket: Option<PathBuf>,

        /// Cache file path
        #[arg(long)]
        cache: Option<PathBuf>,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum InitTarget {
    /// Output zsh integration script (usage: eval "$(abbrs init zsh)")
    Zsh,
    /// Generate config file template
    Config,
}

#[derive(Subcommand, Debug)]
enum ImportSource {
    /// Import from zsh aliases (reads from stdin)
    Aliases {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Import from fish abbreviations (reads from stdin or file)
    Fish {
        /// Input file (reads from stdin if omitted)
        file: Option<PathBuf>,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Import from git aliases
    GitAliases {
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum HistoryAction {
    /// List recent expansion history
    List {
        /// Maximum number of entries to show
        #[arg(long, short = 'n', default_value = "50")]
        limit: usize,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Clear all expansion history
    Clear,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Compile {
            config: cfg,
        } => cmd_compile(cfg),
        Commands::Expand {
            lbuffer,
            rbuffer,
            cache: cache_path,
            config: cfg,
        } => cmd_expand(lbuffer, rbuffer, cache_path, cfg),
        Commands::NextPlaceholder { lbuffer, rbuffer } => cmd_next_placeholder(lbuffer, rbuffer),
        Commands::List { config: cfg } => cmd_list(cfg),
        Commands::Check { config: cfg } => cmd_check(cfg),
        Commands::Init { target } => cmd_init(target),
        Commands::Add {
            keyword,
            expansion,
            global,
            evaluate,
            function,
            regex,
            command,
            allow_conflict,
            context_lbuffer,
            context_rbuffer,
            config: cfg,
        } => cmd_add(
            keyword,
            expansion,
            global,
            evaluate,
            function,
            regex,
            command,
            allow_conflict,
            context_lbuffer,
            context_rbuffer,
            cfg,
        ),
        Commands::Erase {
            keyword,
            command,
            global,
            config: cfg,
        } => cmd_erase(keyword, command, global, cfg),
        Commands::Rename {
            old,
            new,
            command,
            global,
            config: cfg,
        } => cmd_rename(old, new, command, global, cfg),
        Commands::Query {
            keyword,
            command,
            global,
            config: cfg,
        } => cmd_query(keyword, command, global, cfg),
        Commands::Show {
            keyword,
            config: cfg,
        } => cmd_show(keyword, cfg),
        Commands::Remind {
            buffer,
            cache: cache_path,
        } => cmd_remind(buffer, cache_path),
        Commands::Import { source } => cmd_import(source),
        Commands::Export { config: cfg } => cmd_export(cfg),
        Commands::ListKeywords { config: cfg } => cmd_list_keywords(cfg),
        Commands::ServeEnabled { config: cfg } => cmd_serve_enabled(cfg),
        Commands::History { action } => cmd_history(action),
        Commands::Serve { socket, cache, config } => match socket {
            Some(sock_path) => serve::run_socket(sock_path, cache, config),
            None => serve::run(cache, config),
        },
    }
}

fn resolve_config_path(config: Option<PathBuf>) -> Result<PathBuf> {
    match config {
        Some(p) => Ok(p),
        None => config::default_config_path(),
    }
}

fn resolve_cache_path(cache: Option<PathBuf>) -> Result<PathBuf> {
    match cache {
        Some(p) => Ok(p),
        None => config::default_cache_path(),
    }
}

fn cmd_compile(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    let cache_path = resolve_cache_path(None)?;
    require_config(&config_path)?;

    let result = compiler::compile(&config_path, &cache_path)?;

    eprintln!(
        "✓ compiled {} abbreviation(s) → {}",
        result.abbr_count,
        cache_path.display()
    );

    Ok(())
}

fn cmd_expand(
    lbuffer: String,
    rbuffer: String,
    cache_path: Option<PathBuf>,
    cfg: Option<PathBuf>,
) -> Result<()> {
    let cache_file = resolve_cache_path(cache_path)?;
    let config_path = resolve_config_path(cfg)?;

    // Load cache
    let compiled = match cache::read(&cache_file) {
        Ok(c) => c,
        Err(_) => {
            println!("{}", output::ExpandOutput::StaleCache);
            return Ok(());
        }
    };

    // Freshness check
    if config_path.exists() {
        if let Ok(fresh) = cache::is_fresh(&compiled, &config_path) {
            if !fresh {
                println!("{}", output::ExpandOutput::StaleCache);
                return Ok(());
            }
        }
    }

    let regex_cache = context::RegexCache::new();
    let input = expand::ExpandInput { lbuffer, rbuffer };
    let result = expand::expand(&input, &compiled.matcher, &compiled.settings.prefixes, &regex_cache);
    println!("{}", result);

    Ok(())
}

fn cmd_next_placeholder(lbuffer: String, rbuffer: String) -> Result<()> {
    let full_buffer = format!("{}{}", lbuffer, rbuffer);
    let cursor = lbuffer.len();

    match placeholder::find_next_placeholder(&full_buffer, cursor) {
        Some((start, end)) => {
            // Remove placeholder and move cursor there
            let mut new_buffer = String::with_capacity(full_buffer.len() - (end - start));
            new_buffer.push_str(&full_buffer[..start]);
            new_buffer.push_str(&full_buffer[end..]);

            println!(
                "{}",
                output::PlaceholderOutput::Success {
                    buffer: new_buffer,
                    cursor: start,
                }
            );
        }
        None => {
            println!("{}", output::PlaceholderOutput::NoPlaceholder);
        }
    }

    Ok(())
}

fn cmd_list(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    let config = config::load(&config_path)?;

    if config.abbr.is_empty() {
        println!("(no abbreviations registered)");
        return Ok(());
    }

    // Header
    println!("{:<15} {:<6} {:<40}", "KEYWORD", "TYPE", "EXPANSION");
    println!("{}", "-".repeat(65));

    for abbr in &config.abbr {
        let abbr_type = if abbr.context.is_some() {
            "ctx"
        } else if abbr.command.is_some() {
            "cmd"
        } else if abbr.global {
            "global"
        } else if abbr.regex {
            "regex"
        } else {
            "reg"
        };

        let expansion = if abbr.expansion.chars().count() > 38 {
            let truncated: String = abbr.expansion.chars().take(35).collect();
            format!("{}...", truncated)
        } else {
            abbr.expansion.clone()
        };

        let mut flags = Vec::new();
        if abbr.evaluate {
            flags.push("eval");
        }
        if abbr.function {
            flags.push("func");
        }
        if abbr.allow_conflict {
            flags.push("allow");
        }
        if let Some(ref cmd) = abbr.command {
            flags.push(cmd);
        }
        let flag_str = if flags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", flags.join(","))
        };

        println!(
            "{:<15} {:<6} {}{}",
            abbr.keyword, abbr_type, expansion, flag_str
        );
    }

    println!("\nTotal: {}", config.abbr.len());
    Ok(())
}

fn cmd_check(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    let count = compiler::check(&config_path)?;
    eprintln!("✓ config is valid ({} abbreviation(s))", count);
    Ok(())
}

fn cmd_add(
    keyword: Option<String>,
    expansion: Option<String>,
    global: bool,
    evaluate: bool,
    function: bool,
    regex: bool,
    command: Option<String>,
    allow_conflict: bool,
    context_lbuffer: Option<String>,
    context_rbuffer: Option<String>,
    cfg: Option<PathBuf>,
) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    let params = match (keyword, expansion) {
        (Some(kw), Some(exp)) => add::AddParams {
            keyword: kw,
            expansion: exp,
            global,
            evaluate,
            function,
            regex,
            command,
            allow_conflict,
            context_lbuffer,
            context_rbuffer,
        },
        (None, None) => {
            eprintln!("abbrs add - interactive mode\n");
            add::interactive_prompt()?
        }
        _ => {
            anyhow::bail!("both KEYWORD and EXPANSION are required for non-interactive mode\nusage: abbrs add <KEYWORD> <EXPANSION> [OPTIONS]\n       abbrs add  (interactive mode)");
        }
    };

    add::append_to_config(&config_path, &params)?;
    eprintln!("✓ added: {} → {}", params.keyword, params.expansion);

    Ok(())
}

fn cmd_remind(buffer: String, cache_path: Option<PathBuf>) -> Result<()> {
    let cache_file = resolve_cache_path(cache_path)?;

    let compiled = match cache::read(&cache_file) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    if !compiled.settings.remind {
        return Ok(());
    }

    if let Some((keyword, expansion)) = expand::check_remind(&buffer, &compiled.matcher) {
        // Output reminder to stderr (shown via zle -M in widget)
        println!("abbrs: you could have used \"{}\" instead of \"{}\"", keyword, expansion);
    }

    Ok(())
}

fn cmd_erase(
    keyword: String,
    command: Option<String>,
    global: bool,
    cfg: Option<PathBuf>,
) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    if manage::erase(&config_path, &keyword, command.as_deref(), global)? {
        eprintln!("✓ erased: {}", keyword);
    } else {
        anyhow::bail!("abbreviation \"{}\" not found", keyword);
    }

    Ok(())
}

fn cmd_rename(
    old: String,
    new: String,
    command: Option<String>,
    global: bool,
    cfg: Option<PathBuf>,
) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    if manage::rename(&config_path, &old, &new, command.as_deref(), global)? {
        eprintln!("✓ renamed: {} → {}", old, new);
    } else {
        anyhow::bail!("abbreviation \"{}\" not found", old);
    }

    Ok(())
}

fn cmd_query(
    keyword: String,
    command: Option<String>,
    global: bool,
    cfg: Option<PathBuf>,
) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    if manage::query(&config_path, &keyword, command.as_deref(), global)? {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

fn cmd_show(keyword: Option<String>, cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    let lines = manage::show(&config_path, keyword.as_deref())?;
    for line in lines {
        println!("{}", line);
    }
    Ok(())
}

fn print_import_result(result: &import::ImportResult) {
    eprintln!("✓ imported {} abbreviation(s)", result.imported);
    if result.evaluate_count > 0 {
        eprintln!(
            "  ⚠ {} abbreviation(s) use evaluate mode (shell command execution on expand)",
            result.evaluate_count
        );
    }
    if result.function_count > 0 {
        eprintln!(
            "  ⚠ {} abbreviation(s) use function mode (shell function call on expand)",
            result.function_count
        );
    }
    for s in &result.skipped {
        eprintln!("  ⚠ skipped: {}", s);
    }
}

fn cmd_import(source: ImportSource) -> Result<()> {
    match source {
        ImportSource::Aliases { config: cfg } => {
            let config_path = resolve_config_path(cfg)?;
            require_config(&config_path)?;

            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;

            let result = import::import_aliases(&input, &config_path)?;
            print_import_result(&result);
            Ok(())
        }
        ImportSource::Fish {
            file,
            config: cfg,
        } => {
            let config_path = resolve_config_path(cfg)?;
            require_config(&config_path)?;

            let content = match file {
                Some(path) => std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read file: {}", path.display()))?,
                None => {
                    let mut input = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
                    input
                }
            };

            let result = import::import_fish(&content, &config_path)?;
            print_import_result(&result);
            Ok(())
        }
        ImportSource::GitAliases { config: cfg } => {
            let config_path = resolve_config_path(cfg)?;
            require_config(&config_path)?;

            let output = std::process::Command::new("git")
                .args(["config", "--get-regexp", "^alias\\."])
                .output()
                .context("failed to run git config")?;

            if !output.status.success() {
                if output.status.code() == Some(1) {
                    eprintln!("no git aliases found");
                    return Ok(());
                }
                anyhow::bail!(
                    "git config failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let git_output = String::from_utf8_lossy(&output.stdout);
            let result = import::import_git_aliases(&git_output, &config_path)?;
            print_import_result(&result);
            Ok(())
        }
    }
}

fn cmd_list_keywords(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    let config = config::load(&config_path)?;
    for abbr in &config.abbr {
        // Escape colons so zsh _describe doesn't misinterpret them as separators
        let keyword = abbr.keyword.replace(':', "\\:");
        let expansion = abbr.expansion.replace(':', "\\:");
        println!("{}:{}", keyword, expansion);
    }
    Ok(())
}

fn cmd_serve_enabled(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    if !config_path.exists() {
        // No config file — default is serve enabled
        std::process::exit(0);
    }
    let cfg = config::load(&config_path)?;
    if cfg.settings.serve {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

fn cmd_export(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    require_config(&config_path)?;

    let lines = import::export(&config_path)?;
    for line in lines {
        println!("{}", line);
    }
    Ok(())
}

fn require_config(config_path: &std::path::Path) -> Result<()> {
    if !config_path.exists() {
        anyhow::bail!(
            "config file not found: {}\nrun `abbrs init config` to generate a template",
            config_path.display()
        );
    }
    Ok(())
}

fn cmd_init(target: InitTarget) -> Result<()> {
    match target {
        InitTarget::Zsh => cmd_init_zsh(),
        InitTarget::Config => cmd_init_config(),
    }
}

fn cmd_init_zsh() -> Result<()> {
    let bin_path = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("abbrs"))
        .to_string_lossy()
        .into_owned();
    let script = include_str!("../shells/zsh/abbrs.zsh").replace("__ABBRS_BIN__", &bin_path);
    print!("{}", script);
    Ok(())
}

fn cmd_init_config() -> Result<()> {
    let config_path = config::default_config_path()?;

    if config_path.exists() {
        anyhow::bail!(
            "config file already exists: {}",
            config_path.display()
        );
    }

    // Create parent directories
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let template = r#"# abbrs - abbreviation configuration
# See: https://github.com/ushironoko/abbrs

[settings]
# serve = true  # enable daemon mode for sub-millisecond latency (default: true)
# prefixes = ["sudo", "doas"]  # commands that preserve command position
# remind = false  # remind when abbreviation could have been used

# Regular abbreviation (expand only at command position)
[[abbr]]
keyword = "g"
expansion = "git"

[[abbr]]
keyword = "gc"
expansion = "git commit -m '{{message}}'"

[[abbr]]
keyword = "gp"
expansion = "git push"

# Global abbreviation (expand at any position)
# [[abbr]]
# keyword = "NE"
# expansion = "2>/dev/null"
# global = true

# Command-scoped abbreviation (expand only after specific command)
# [[abbr]]
# keyword = "co"
# expansion = "checkout"
# command = "git"

# Context abbreviation (with context condition)
# [[abbr]]
# keyword = "main"
# expansion = "main --branch"
# context.lbuffer = "^git (checkout|switch)"

# Command evaluation (expand with command output)
# [[abbr]]
# keyword = "TODAY"
# expansion = "date +%Y-%m-%d"
# evaluate = true
# global = true
"#;

    std::fs::write(&config_path, template)?;
    eprintln!("✓ generated config file: {}", config_path.display());
    Ok(())
}

fn cmd_history(action: HistoryAction) -> Result<()> {
    let history_path = history::default_history_path()?;

    match action {
        HistoryAction::List { limit, config: cfg } => {
            let config_path = resolve_config_path(cfg)?;
            let max_limit = if config_path.exists() {
                config::load(&config_path)
                    .map(|c| c.settings.history_limit)
                    .unwrap_or(500)
            } else {
                500
            };
            let effective_limit = limit.min(max_limit);

            let entries = history::load(&history_path, effective_limit)?;

            if entries.is_empty() {
                eprintln!("(no expansion history)");
                return Ok(());
            }

            println!("{:<20} {:<15} {}", "TIMESTAMP", "KEYWORD", "EXPANSION");
            println!("{}", "-".repeat(60));

            for entry in &entries {
                let datetime = format_timestamp(entry.timestamp);
                println!("{:<20} {:<15} {}", datetime, entry.keyword, entry.expansion);
            }

            eprintln!("\n{} entries", entries.len());
            Ok(())
        }
        HistoryAction::Clear => {
            history::clear(&history_path)?;
            eprintln!("✓ expansion history cleared");
            Ok(())
        }
    }
}

fn format_timestamp(ts: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(ts);
    // Format as local time using chrono-free approach
    match dt.elapsed() {
        Ok(elapsed) => {
            let secs = elapsed.as_secs();
            if secs < 60 {
                "just now".to_string()
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        }
        Err(_) => ts.to_string(),
    }
}
