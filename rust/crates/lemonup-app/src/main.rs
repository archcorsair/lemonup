mod action;
mod app;
mod cli;
mod event;
mod onboarding;
mod tui;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use lemonup_core::{
    AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE, LemonupError, StateDatabase,
    validate_addons_path,
};
use tracing_subscriber::{EnvFilter, fmt};

use crate::app::{App, AppRuntime};
use crate::cli::{Cli, Commands};
use crate::event::EventHandler;
use crate::tui::Tui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let runtime = AppRuntime::new(
        cli.profile.clone(),
        cli.addon_dir.clone(),
        load_guarded_addon_dir(&cli.profile)?,
    );
    let paths = AppPaths::discover(cli.profile)?;
    paths.ensure()?;

    configure_tracing();

    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Tui => run_tui(paths, runtime).await?,
        Commands::Update { force, dry_run } => run_update(paths, runtime, force, dry_run)?,
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

async fn run_tui(paths: AppPaths, runtime: AppRuntime) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::bootstrap(paths, runtime)?;
    let mut tui = Tui::enter()?;
    let events = EventHandler::new(Duration::from_millis(250));
    let result = app.run(tui.terminal_mut(), events).await;
    tui.exit()?;
    result?;
    Ok(())
}

fn run_update(
    paths: AppPaths,
    runtime: AppRuntime,
    force: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
    let database = StateDatabase::open(paths.state_db_file)?;
    let installed = database.list_addons()?;

    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());

    if let Some(path) = &effective_addon_dir {
        if runtime.is_guarded_path(path) {
            return Err(Box::new(LemonupError::InvalidArgument(format!(
                "profile '{}' refuses to target the default profile addon directory: {}",
                runtime.profile_name,
                path.display()
            ))));
        }

        validate_addons_path(path).map_err(|error| {
            Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
        })?;
    }

    match config_state {
        ConfigLoad::Missing(path) if runtime.addon_dir_override.is_none() => {
            println!(
                "config missing at {}. run `lemonup tui --profile {}` to initialize v2 first.",
                path.display(),
                runtime.profile_name
            );
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) => {
            println!(
                "update foundation ready: profile={}, tracked addons={}, addon_dir={}, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                effective_addon_dir
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

fn load_guarded_addon_dir(profile: &str) -> lemonup_core::Result<Option<PathBuf>> {
    if profile == DEFAULT_PROFILE {
        return Ok(None);
    }

    let default_paths = AppPaths::discover(DEFAULT_PROFILE)?;
    let config_store = ConfigStore::new(default_paths.config_file);
    match config_store.load()? {
        ConfigLoad::Missing(_) => Ok(None),
        ConfigLoad::Loaded(config) => Ok(config.addon_dir),
    }
}
