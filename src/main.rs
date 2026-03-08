use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use brv::{cache, compiler, config, expand, output, placeholder};

#[derive(Parser, Debug)]
#[command(name = "brv")]
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
        /// Treat suffix conflicts as errors
        #[arg(long, default_value = "false")]
        strict: bool,

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

    /// Generate config file template
    Init,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Compile { strict, config: cfg } => cmd_compile(strict, cfg),
        Commands::Expand {
            lbuffer,
            rbuffer,
            cache: cache_path,
            config: cfg,
        } => cmd_expand(lbuffer, rbuffer, cache_path, cfg),
        Commands::NextPlaceholder { lbuffer, rbuffer } => cmd_next_placeholder(lbuffer, rbuffer),
        Commands::List { config: cfg } => cmd_list(cfg),
        Commands::Check { config: cfg } => cmd_check(cfg),
        Commands::Init => cmd_init(),
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

fn cmd_compile(strict: bool, cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;
    let cache_path = resolve_cache_path(None)?;

    if !config_path.exists() {
        anyhow::bail!(
            "config file not found: {}\nrun `brv init` to generate a template",
            config_path.display()
        );
    }

    let result = compiler::compile(&config_path, &cache_path, strict)?;

    // Print warnings
    for warning in &result.warnings {
        eprintln!("  ⚠ {}", warning);
    }

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

    let input = expand::ExpandInput { lbuffer, rbuffer };
    let result = expand::expand(&input, &compiled.matcher);
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

    if !config_path.exists() {
        anyhow::bail!(
            "config file not found: {}\nrun `brv init` to generate a template",
            config_path.display()
        );
    }

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
        } else if abbr.global {
            "global"
        } else {
            "reg"
        };

        let expansion = if abbr.expansion.len() > 38 {
            format!("{}...", &abbr.expansion[..35])
        } else {
            abbr.expansion.clone()
        };

        let mut flags = Vec::new();
        if abbr.evaluate {
            flags.push("eval");
        }
        if abbr.allow_conflict {
            flags.push("allow");
        }
        let flag_str = if flags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", flags.join(","))
        };

        println!("{:<15} {:<6} {}{}", abbr.keyword, abbr_type, expansion, flag_str);
    }

    println!("\nTotal: {}", config.abbr.len());
    Ok(())
}

fn cmd_check(cfg: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config_path(cfg)?;

    if !config_path.exists() {
        anyhow::bail!(
            "config file not found: {}\nrun `brv init` to generate a template",
            config_path.display()
        );
    }

    let count = compiler::check(&config_path)?;
    eprintln!("✓ config is valid ({} abbreviation(s))", count);
    Ok(())
}

fn cmd_init() -> Result<()> {
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

    let template = r#"# brv - abbreviation configuration
# See: https://github.com/ushironoko/brv

[settings]
strict = false  # true: treat suffix conflicts as errors

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
