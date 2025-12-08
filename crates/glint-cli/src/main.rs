//! # Glint CLI
//!
//! Command-line interface for the Glint file search tool.
//!
//! ## Commands
//!
//! - `glint index` - Build or rebuild the file index
//! - `glint query <pattern>` - Search for files matching a pattern
//! - `glint interactive` - Start interactive TUI mode
//! - `glint status` - Show index status and statistics
//!
//! ## Example Usage
//!
//! ```bash
//! # Build the initial index (requires admin for full MFT access)
//! glint index
//!
//! # Search for Rust files
//! glint query "*.rs"
//!
//! # Interactive search
//! glint interactive
//! ```

mod app;
mod commands;
mod tui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Glint - Extremely fast file search
#[derive(Parser)]
#[command(name = "glint")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to configuration file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Verbosity level (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build or rebuild the file index
    Index {
        /// Force a full re-index even if the index exists
        #[arg(short, long)]
        force: bool,

        /// Only index specific volumes (e.g., "C:" "D:")
        #[arg(short = 'V', long)]
        volumes: Vec<String>,
    },

    /// Search for files matching a pattern
    Query {
        /// Search pattern (supports wildcards and regex with r/pattern/)
        pattern: String,

        /// Maximum number of results to show
        #[arg(short, long, default_value = "100")]
        limit: usize,

        /// Only show files (not directories)
        #[arg(short, long)]
        files_only: bool,

        /// Only show directories
        #[arg(short, long)]
        dirs_only: bool,

        /// Filter by extension (can be used multiple times)
        #[arg(short, long)]
        ext: Vec<String>,

        /// Search in full paths, not just filenames
        #[arg(short, long)]
        path: bool,

        /// Output format (text, json)
        #[arg(short, long, default_value = "text")]
        output: OutputFormat,
    },

    /// Start interactive TUI mode
    #[command(alias = "i")]
    Interactive,

    /// Show index status and statistics
    Status,

    /// Start watching for file changes (requires the index to exist)
    Watch {
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
    },

    /// Clear the index and all data
    Clear {
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!("Unknown output format: {}", s)),
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)))
        .init();

    // Load configuration
    let config = match &cli.config {
        Some(path) => glint_core::Config::load_from(path)?,
        None => glint_core::Config::load()?,
    };

    // Execute command
    match cli.command {
        Commands::Index { force, volumes } => commands::index::run(config, force, volumes),
        Commands::Query {
            pattern,
            limit,
            files_only,
            dirs_only,
            ext,
            path,
            output,
        } => commands::query::run(
            config, &pattern, limit, files_only, dirs_only, ext, path, output,
        ),
        Commands::Interactive => tui::run(config),
        Commands::Status => commands::status::run(config),
        Commands::Watch { foreground } => commands::watch::run(config, foreground),
        Commands::Clear { yes } => commands::clear::run(config, yes),
    }
}
