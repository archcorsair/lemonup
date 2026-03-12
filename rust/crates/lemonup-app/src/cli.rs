use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "lemonup", about = "Rust rewrite of the LemonUp addon manager")]
pub struct Cli {
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
