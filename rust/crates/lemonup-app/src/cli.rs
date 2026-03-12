use std::path::PathBuf;

use clap::{Parser, Subcommand};
use lemonup_core::DEFAULT_PROFILE;

#[derive(Debug, Parser)]
#[command(name = "lemonup", about = "Rust rewrite of the LemonUp addon manager")]
pub struct Cli {
    #[arg(long, default_value = DEFAULT_PROFILE)]
    pub profile: String,
    #[arg(long)]
    pub addon_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Launch the Ratatui interface
    Tui,
    /// Run the basic non-interactive updater path
    Update {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
}
