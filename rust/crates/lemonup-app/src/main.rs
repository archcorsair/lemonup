mod action;
mod app;
mod cli;
mod event;
mod onboarding;
mod tui;

use std::time::Duration;

use clap::Parser;
use lemonup_core::{AppPaths, ConfigLoad, ConfigStore, StateDatabase};
use tracing_subscriber::{EnvFilter, fmt};

use crate::app::App;
use crate::cli::{Cli, Commands};
use crate::event::EventHandler;
use crate::tui::Tui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = AppPaths::discover()?;
    paths.ensure()?;

    configure_tracing();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Tui => run_tui(paths).await?,
        Commands::Update { force, dry_run } => run_update(paths, force, dry_run)?,
    }

    Ok(())
}

fn configure_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,lemonup_app=debug,lemonup_core=debug"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .json()
        .init();
}

async fn run_tui(paths: AppPaths) -> Result<(), Box<dyn std::error::Error>> {
    let mut tui = Tui::enter()?;
    let events = EventHandler::new(Duration::from_millis(250));
    let mut app = App::bootstrap(paths)?;
    let result = app.run(tui.terminal_mut(), events).await;
    tui.exit()?;
    result?;
    Ok(())
}

fn run_update(
    paths: AppPaths,
    force: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
    let database = StateDatabase::open(paths.state_db_file)?;
    let installed = database.list_addons()?;

    match config_state {
        ConfigLoad::Missing(path) => {
            println!(
                "config missing at {}. run `lemonup tui` to initialize v2 first.",
                path.display()
            );
        }
        ConfigLoad::Loaded(config) => {
            println!(
                "update foundation ready: {} tracked addons, addon_dir={}, force={}, dry_run={}",
                installed.len(),
                config
                    .addon_dir
                    .as_ref()
                    .map(|value| value.display().to_string())
                    .unwrap_or_else(|| "<unconfigured>".to_string()),
                force,
                dry_run
            );
        }
    }

    Ok(())
}
