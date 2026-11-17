mod action;
mod app;
mod archive;
mod backup;
mod cli;
mod drift;
mod event;
mod find;
mod github;
mod http;
mod onboarding;
mod shimmer;
mod source_error;
mod theme;
mod transfer;
mod tui;
mod tukui;
mod update;
mod update_targets;
mod wago;
mod wowinterface;

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use lemonup_core::binding::{BindingCheck, check_addon_dir_binding};
use lemonup_core::{
    AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE, GameFlavor, ImportAnalysis, LemonupError,
    ProfileRoots, ProfileSummary, ScanSummary, StateDatabase, UpdateStatus, V1Location,
    V1MigrationOutcome, analyze_import, default_transfer_path, export_addons, list_profiles,
    migrate_v1_if_needed, parse_import_file, remember_last_used, scan_addons_dir_report,
    tui_profile, validate_addons_path,
};
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::{EnvFilter, fmt};

use crate::app::{App, AppRuntime};
use crate::backup::{
    PreUpdateBackupSettings, adopt_legacy_backups, backup_root, client_backup_dir, list_backups,
    restore_wtf_backup, run_pre_update_backup,
};
use crate::cli::{Cli, Commands};
use crate::event::EventHandler;
use crate::github::install_github_addon;
use crate::transfer::{collect_existing_import_folders, install_transfer_addons};
use crate::tui::Tui;
use crate::tukui::install_tukui_addon;
#[cfg(test)]
use crate::update::refresh_managed_update_state;
use crate::update::{
    LiveUpdateResult, LiveUpdateStatus, apply_live_updates, refresh_live_update_checks,
    serialize_live_update_status, serialize_update_status,
};
use crate::wago::{WagoStability, install_wago_addon, resolve_wago_api_key};
use crate::wowinterface::install_wowinterface_addon;

/// How a command that ran to completion maps to the process exit status.
///
/// Fatal errors never reach this type: they come back as `Err` from `run` and exit 1. This
/// covers the other case, where the command finished and printed its normal output but some
/// addon/target errored (v1 parity: any failed addon exits 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandOutcome {
    Success,
    /// Finished, but this many addons/targets errored.
    Failures(usize),
    /// The command could not run (no addon dir, no Wago API key) and printed a skip message.
    /// Scripts must see this as a failure, so it exits 1.
    Skipped,
}

impl CommandOutcome {
    fn from_error_count(errors: usize) -> Self {
        if errors == 0 {
            Self::Success
        } else {
            Self::Failures(errors)
        }
    }

    fn exit_status(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failures(_) | Self::Skipped => 1,
        }
    }
}

/// Number of checks that ended in an error status. `update_available` is not an error.
fn count_check_errors(checks: &[crate::update::CheckResult]) -> usize {
    checks
        .iter()
        .filter(|check| check.status == UpdateStatus::Error)
        .count()
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(outcome) => ExitCode::from(outcome.exit_status()),
        Err(error) => {
            eprintln!("{}", error_text(error.as_ref()));
            ExitCode::FAILURE
        }
    }
}

/// What an error that ends a command prints. A refusal already says what happened and what to
/// do next, so it prints as it is; anything else is labeled as an error.
fn error_text(error: &(dyn std::error::Error + 'static)) -> String {
    match error.downcast_ref::<LemonupError>() {
        Some(LemonupError::Refused(message)) => message.clone(),
        _ => format!("Error: {error}"),
    }
}

async fn run() -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Tui);
    let roots = ProfileRoots::discover()?;
    let profile = command_profile(&roots, cli.profile, &command)?;
    let runtime = AppRuntime::new(
        profile.clone(),
        cli.addon_dir.clone(),
        load_guarded_addon_dirs(&profile)?,
    )
    .with_display(theme::DisplayEnv::detect());
    let paths = roots.paths(&profile)?;
    paths.ensure()?;

    configure_tracing(log_destination(&command, &paths));

    let startup_notice = run_v1_migration(&paths, V1Location::discover());
    if let Some(notice) = startup_notice.as_ref()
        && !matches!(command, Commands::Tui)
    {
        notice.print();
    }

    let outcome = match command {
        Commands::Tui => {
            run_tui(paths, runtime, startup_notice).await?;
            CommandOutcome::Success
        }
        Commands::Sync => run_sync(paths, runtime)?,
        Commands::Profiles => {
            for line in profile_lines(&list_profiles(&paths.roots)?) {
                println!("{line}");
            }
            CommandOutcome::Success
        }
        Commands::Check { addons } => run_check(paths, runtime, addons).await?,
        Commands::Update {
            addons,
            force,
            dry_run,
        } => run_update(paths, runtime, addons, force, dry_run).await?,
        Commands::UpdateAll { force, dry_run } => {
            run_update(paths, runtime, Vec::new(), force, dry_run).await?
        }
        // Install commands target a single addon: a failed install is a fatal `Err` (exit 1).
        Commands::InstallWago {
            addon,
            stability,
            dry_run,
        } => run_install_wago(paths, runtime, &addon, stability, dry_run).await?,
        Commands::InstallTukui { addon, dry_run } => {
            run_install_tukui(paths, runtime, &addon, dry_run).await?
        }
        Commands::InstallWowinterface { addon, dry_run } => {
            run_install_wowinterface(paths, runtime, &addon, dry_run).await?
        }
        Commands::InstallGithub { addon, dry_run } => {
            run_install_github(paths, runtime, &addon, dry_run).await?
        }
        Commands::ExportAddons { output } => {
            run_export_addons(paths, output)?;
            CommandOutcome::Success
        }
        Commands::ImportAddons { input, dry_run } => {
            run_import_addons(paths, runtime, input, dry_run).await?
        }
        Commands::RestoreBackup { archive } => {
            run_restore_backup(paths, runtime, archive)?;
            CommandOutcome::Success
        }
        Commands::Toc {
            addon,
            field,
            value,
            clear,
            yes,
        } => run_toc(paths, runtime, &addon, field, value, clear, yes)?,
        Commands::Find { link, yes, plain } => run_find(paths, runtime, link, yes, plain).await?,
    };

    Ok(outcome)
}

/// A command's config: the profile's own settings with the ones all games share.
fn load_profile_config(paths: &AppPaths) -> lemonup_core::Result<(ConfigStore, ConfigLoad)> {
    let config_store = ConfigStore::for_profile(paths);
    let config_state = config_store.load()?;
    Ok((config_store, config_state))
}

/// The profile a run uses: `--profile` when given; else the TUI opens the game used last,
/// and every other command uses the default profile, so scripts keep acting on it.
fn command_profile(
    roots: &ProfileRoots,
    requested: Option<String>,
    command: &Commands,
) -> lemonup_core::Result<String> {
    match requested {
        Some(profile) => Ok(profile),
        None if matches!(command, Commands::Tui) => tui_profile(roots, None),
        None => Ok(DEFAULT_PROFILE.to_string()),
    }
}

/// `lemonup profiles`: a summary line, then one line per profile.
fn profile_lines(profiles: &[ProfileSummary]) -> Vec<String> {
    let mut lines = vec![format!("profiles summary: count={}", profiles.len())];
    for profile in profiles {
        let mut line = format!(
            "profile: name={}, game={}, game_version={}, addon_dir={}, addons={}",
            profile.name,
            profile.display_name,
            profile
                .flavor
                .map(lemonup_core::profiles::profile_name_for)
                .unwrap_or("<unknown>"),
            profile
                .addon_dir
                .as_ref()
                .map(|dir| dir.display().to_string())
                .unwrap_or_else(|| "<unconfigured>".to_string()),
            profile
                .addon_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "<unknown>".to_string()),
        );
        if let Some(problem) = &profile.problem {
            line.push_str(&format!(", problem={problem}"));
        }
        lines.push(line);
    }
    lines
}

/// `lemonup toc`: prints the addon's Title, Notes and Category, or changes one. The CLI never
/// asks, so changing a "Not linked" addon needs `--yes`.
fn run_toc(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    field: Option<lemonup_core::TocField>,
    value: Option<String>,
    clear: bool,
    yes: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let change = match (field, value, clear) {
        (None, _, _) => None,
        (Some(field), Some(value), false) => Some((field, Some(value))),
        (Some(field), None, true) => Some((field, None)),
        (Some(field), _, _) => {
            return Err(Box::new(LemonupError::InvalidArgument(format!(
                "give a new {} value, or --clear to restore the addon's own",
                field.name()
            ))));
        }
    };
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let Some(addon_dir) = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone())
    else {
        println!(
            "toc skipped: profile={}, addon_dir=<unconfigured>",
            runtime.profile_name
        );
        return Ok(CommandOutcome::Skipped);
    };
    if runtime.is_guarded_path(&addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            addon_dir.display()
        ))));
    }
    validate_addons_path(&addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;
    let mut database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        change.is_some(),
    )?;
    for line in toc_command(&mut database, &addon_dir, addon, change, yes)? {
        println!("{line}");
    }
    Ok(CommandOutcome::Success)
}

/// `lemonup find`: lists where the Not linked addons come from, grouped by label, and changes
/// nothing. `--link` links the exact matches (no download); the CLI never asks, so matches that
/// take over folders beyond their own need `--yes`. `--plain` prints one key=value line per
/// result instead of the readable report. Exits 1 when a source failed.
async fn run_find(
    paths: AppPaths,
    runtime: AppRuntime,
    link: bool,
    yes: bool,
    plain: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let Some(addon_dir) = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone())
    else {
        println!(
            "find skipped: profile={}, addon_dir=<unconfigured>",
            runtime.profile_name
        );
        return Ok(CommandOutcome::Skipped);
    };
    if runtime.is_guarded_path(&addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            addon_dir.display()
        ))));
    }
    validate_addons_path(&addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;
    let mut database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        link,
    )?;

    let flavor = GameFlavor::for_addons_dir(&addon_dir);
    if link {
        crate::find::scan_before_linking(&mut database, &addon_dir, flavor)?;
    }
    let rows = database.list_addons()?;
    let inputs = lemonup_core::find::FindInputs::read(&addon_dir, flavor)?;
    let wago_lookups = inputs.wago_lookups(&rows, None);
    let api_key = resolve_wago_api_key(&config_state);
    let sends_to_wago = api_key.is_some() && !wago_lookups.is_empty();
    if plain && sends_to_wago {
        println!(
            "find note: sending {} folder names and file hashes to Wago.io",
            wago_lookups.len()
        );
    }
    let names = crate::find::LocalNames::new(&rows, &inputs);
    let lookups =
        crate::find::lookup_sources(&wago_lookups, flavor, api_key.as_deref(), &paths.cache_dir)
            .await;
    let report = lemonup_core::find::match_addons(&inputs, &rows, None, &lookups.lookups);
    let mut link_now =
        || crate::find::link_exact_matches(&mut database, &addon_dir, flavor, &report, yes);
    if plain {
        for line in crate::find::report_lines(&report, &lookups.problems, inputs.folders.len()) {
            println!("{line}");
        }
        let outcome = if link { Some(link_now()?) } else { None };
        for line in crate::find::plain_closing(&report, outcome)? {
            println!("{line}");
        }
    } else {
        let wago_folders = if sends_to_wago { wago_lookups.len() } else { 0 };
        // Colors only on a terminal, never in a pipe; the refusal goes to stderr, so it asks
        // about stderr.
        let theme_config = match &config_state {
            ConfigLoad::Loaded(config) => config.clone(),
            ConfigLoad::Missing(_) => lemonup_core::AppConfig::new_unconfigured(),
        };
        let paint = crate::find::Paint::for_stdout(&theme_config);
        for line in
            crate::find::human_report(&report, &lookups.problems, &names, wago_folders, paint)
        {
            println!("{line}");
        }
        println!();
        let outcome = if link { Some(link_now()?) } else { None };
        // A refusal leaves the report on screen and ends the run with what happened.
        let error_paint = crate::find::Paint::for_stderr(&theme_config);
        for line in crate::find::human_closing(&report, &names, outcome, paint, error_paint)? {
            println!("{line}");
        }
    }
    Ok(CommandOutcome::from_error_count(lookups.problems.len()))
}

/// What `lemonup toc` prints, after changing a field when `change` asks for it (`None` as the
/// value restores the addon's own). A change rewrites the addon's .toc files and reads the
/// AddOns folder again, so the stored name follows a new Title.
fn toc_command(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selector: &str,
    change: Option<(lemonup_core::TocField, Option<String>)>,
    yes: bool,
) -> Result<Vec<String>, LemonupError> {
    use lemonup_core::toc_edit;

    let installed = database.list_addons()?;
    let record = crate::update::resolve_selected_addons(&installed, &[selector.to_string()])
        .ok()
        .and_then(|selected| selected.first().map(|record| (*record).clone()))
        .ok_or_else(|| {
            LemonupError::InvalidArgument(format!(
                "No addon matches {selector}. Use the exact addon name or folder name; \
                 `lemonup sync` reads your AddOns folder first."
            ))
        })?;
    let flavor = GameFlavor::for_addons_dir(addon_dir);

    let Some((field, value)) = change else {
        let effective = lemonup_core::read_addon_toc_fields(addon_dir, &record.folder, flavor)?
            .unwrap_or_default();
        return Ok(lemonup_core::TocField::ALL
            .into_iter()
            .map(|field| {
                let value = effective.get(field);
                format!(
                    "toc field={} value={} source={}",
                    field.name(),
                    value.unwrap_or("<none>"),
                    toc_edit::value_source(field, &record.toc_overrides, value).cli_name()
                )
            })
            .collect());
    };

    let mut overrides = record.toc_overrides.clone();
    toc_edit::set_override(&mut overrides, field, value.as_deref())?;
    if record.source == lemonup_core::SourceKind::Manual && !yes {
        return Err(LemonupError::Refused(format!(
            "{} is Not linked: LemonUp didn't install it, so changing its .toc files needs \
             --yes. Its own lines are kept, and --clear puts them back. Nothing was changed.",
            record.name
        )));
    }
    let folders = crate::update_targets::apply_toc_overrides_now(
        database,
        addon_dir,
        &record.folder,
        &overrides,
    )
    .map_err(|error| LemonupError::InvalidArgument(error.to_string()))?;
    // The list catches up on the next scan if this read fails.
    let rescan = database.list_addons().and_then(|tracked| {
        let report = scan_addons_dir_report(addon_dir, flavor, &tracked)?;
        database.reconcile_scan(&report)
    });
    if let Err(error) = rescan {
        tracing::warn!(%error, "could not read the AddOns folder after a .toc change");
    }
    let folders = folders.join("|");
    Ok(match value {
        Some(_) => vec![
            format!(
                "toc saved: addon={}, field={}, value={}, folders={folders}",
                record.name,
                field.name(),
                overrides.get(field.name()).map_or("", String::as_str)
            ),
            "toc note: /reload in the game to see it".to_string(),
        ],
        None => vec![format!(
            "toc restored: addon={}, field={}, folders={folders}",
            record.name,
            field.name()
        )],
    })
}

/// Log filter used when `RUST_LOG` is unset or invalid: quiet (`warn`) for
/// release builds so CLI runs and cron jobs stay clean, verbose for debug builds.
fn default_log_filter(debug_build: bool) -> &'static str {
    if debug_build {
        "info,lemonup_app=debug,lemonup_core=debug"
    } else {
        "warn"
    }
}

/// Where tracing output goes for a given command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LogDestination {
    /// CLI commands: stderr, so logs never mix with the `key=value` result lines on stdout.
    Stderr,
    /// The TUI owns the terminal (alternate screen), so logs go to this file instead.
    File(PathBuf),
}

/// File the TUI appends its logs to: `<data_dir>/logs/lemonup.log` for the active profile.
fn tui_log_file(paths: &AppPaths) -> PathBuf {
    paths.log_dir.join("lemonup.log")
}

fn log_destination(command: &Commands, paths: &AppPaths) -> LogDestination {
    match command {
        Commands::Tui => LogDestination::File(tui_log_file(paths)),
        _ => LogDestination::Stderr,
    }
}

/// Opens (creating if needed) the log file in append mode. `None` when it cannot be opened.
fn open_log_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok()?;
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Writer for a destination. A log file that cannot be opened silences logging for the TUI
/// rather than falling back to stderr, which would draw over the alternate screen.
fn log_writer(destination: &LogDestination) -> BoxMakeWriter {
    match destination {
        LogDestination::Stderr => BoxMakeWriter::new(std::io::stderr),
        LogDestination::File(path) => match open_log_file(path) {
            Some(file) => BoxMakeWriter::new(std::sync::Mutex::new(file)),
            None => BoxMakeWriter::new(std::io::sink),
        },
    }
}

fn configure_tracing(destination: LogDestination) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_log_filter(cfg!(debug_assertions))));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(log_writer(&destination))
        .json()
        .init();
}

/// The command that opens the TUI for `profile`. Global options such as
/// `--profile` must come before the subcommand.
fn tui_command_hint(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "lemonup tui".to_string()
    } else {
        format!("lemonup --profile {profile} tui")
    }
}

/// Result of the one-time v1 migration, shown once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupNotice {
    text: String,
    is_error: bool,
    /// Extra lines printed by CLI commands only (for example skipped addons).
    details: Vec<String>,
}

impl StartupNotice {
    /// CLI output goes to stderr so it never mixes with `key=value` result lines.
    fn print(&self) {
        eprintln!("{}", self.text);
        for line in &self.details {
            eprintln!("  {line}");
        }
    }
}

/// Runs the v1 migration for the default profile when no v2 config exists.
/// Never fails startup: errors become a one-line notice and onboarding runs.
fn run_v1_migration(paths: &AppPaths, v1: Option<V1Location>) -> Option<StartupNotice> {
    let v1 = v1?;
    match migrate_v1_if_needed(paths, &v1) {
        Ok(V1MigrationOutcome::NotApplicable) => None,
        Ok(V1MigrationOutcome::Migrated(report)) => {
            tracing::info!(
                v1_dir = %report.v1_dir.display(),
                addon_dir = ?report.addon_dir,
                addon_dir_rejected = ?report.addon_dir_rejected,
                database_found = report.database_found,
                migrated = ?report.migrated_addons,
                skipped = ?report.skipped_addons,
                already_tracked = ?report.already_tracked,
                owned_by_parent = ?report.owned_by_parent,
                addons_not_imported = ?report.addons_not_imported,
                "migrated LemonUp v1 config"
            );
            let mut details = Vec::new();
            if let Some(reason) = report.addon_dir_rejected.as_ref() {
                details.push(format!("v1 AddOns path not migrated: {reason}"));
            }
            if !report.skipped_addons.is_empty() {
                details.push(format!(
                    "v1 addons skipped (folder not found or not a plain folder name): {}",
                    report.skipped_addons.join(", ")
                ));
            }
            if !report.already_tracked.is_empty() {
                details.push(format!(
                    "v1 addons already in LemonUp (kept the existing data): {}",
                    report.already_tracked.join(", ")
                ));
            }
            if report.addons_not_imported.is_some() {
                details.push(
                    "to bring addons over: export in v1, then run `lemonup import-addons`"
                        .to_string(),
                );
            }
            Some(StartupNotice {
                text: report.summary_line(),
                is_error: report.addons_not_imported.is_some(),
                details,
            })
        }
        Err(error) => {
            tracing::warn!(%error, v1_dir = %v1.dir.display(), "LemonUp v1 migration failed");
            Some(StartupNotice {
                text: format!(
                    "LemonUp v1 migration failed: {error} | continuing with setup, v1 files untouched"
                ),
                is_error: true,
                details: Vec::new(),
            })
        }
    }
}

async fn run_tui(
    paths: AppPaths,
    runtime: AppRuntime,
    startup_notice: Option<StartupNotice>,
) -> Result<(), Box<dyn std::error::Error>> {
    let roots = paths.roots.clone();
    let profile = paths.profile.clone();
    let mut app = App::bootstrap(paths, runtime)?;
    // Plain `lemonup` opens this game next time. Not being able to save that is no reason
    // to stop.
    if let Err(error) = remember_last_used(&roots, &profile) {
        tracing::warn!(%error, "couldn't remember the last used game");
    }
    if let Some(notice) = startup_notice {
        app.show_startup_notice(&notice.text, notice.is_error);
    }
    let mut tui = Tui::enter()?;
    let events = EventHandler::new(Duration::from_millis(100));
    let result = app.run(tui.terminal_mut(), events).await;
    tui.exit()?;
    result?;
    Ok(())
}

async fn run_check(
    paths: AppPaths,
    runtime: AppRuntime,
    addons: Vec<String>,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (config_store, config_state) = load_profile_config(&paths)?;
    let mut database = StateDatabase::open(paths.state_db_file)?;
    let show_details = !addons.is_empty();
    let api_key = resolve_wago_api_key(&config_store, &config_state)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());
    // `check` is read-only toward the folder: against a folder this profile is not bound
    // to it still reports, but on a throwaway copy so no row is written.
    let mut scratch = None;
    if let Some(addon_dir) = effective_addon_dir.as_deref()
        && let BindingCheck::Mismatch(mismatch) = check_addon_dir_binding(
            &database,
            &runtime.profile_name,
            configured_addon_dir.as_deref(),
            addon_dir,
            false,
        )?
    {
        eprintln!("check warning: {mismatch} | running read-only, results are not saved");
        let dir = tempfile::tempdir()?;
        let copy = dir.path().join("state.sqlite");
        database.copy_to(&copy)?;
        database = StateDatabase::open(&copy)?;
        scratch = Some(dir);
    }
    let checks = refresh_live_update_checks(
        &mut database,
        &addons,
        api_key.as_deref(),
        effective_addon_dir.as_deref(),
    )
    .await
    .map_err(Box::<dyn std::error::Error>::from)?;
    drop(database);
    drop(scratch);

    let up_to_date = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::UpToDate)
        .count();
    let update_available = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::UpdateAvailable)
        .count();
    let remote_older = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::RemoteOlder)
        .count();
    let unknown = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::Unknown)
        .count();
    let errors = count_check_errors(&checks);

    println!(
        "check summary: targets={}, checked={}, up_to_date={}, update_available={}, remote_older={}, unknown={}, errors={}",
        if addons.is_empty() {
            "all".to_string()
        } else {
            addons.join("|")
        },
        checks.len(),
        up_to_date,
        update_available,
        remote_older,
        unknown,
        errors
    );

    if show_details {
        for check in checks {
            println!(
                "check result: addon={}, status={}, remote_version={}, message={}",
                check.addon_name,
                serialize_update_status(check.status),
                check.remote_version.as_deref().unwrap_or("<unknown>"),
                check.message.as_deref().unwrap_or("<none>")
            );
        }
    } else {
        println!(
            "check detail: pass one or more exact addon names or folders to inspect individual results"
        );
    }

    Ok(CommandOutcome::from_error_count(errors))
}

async fn run_update(
    paths: AppPaths,
    runtime: AppRuntime,
    addons: Vec<String>,
    force: bool,
    dry_run: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (config_store, config_state) = load_profile_config(&paths)?;
    let mut database = StateDatabase::open(&paths.state_db_file)?;
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
        ensure_bound_addon_dir(
            &database,
            &paths,
            &runtime,
            configured_addon_dir.as_deref(),
            path,
            !dry_run,
        )?;
    }

    let outcome = match config_state {
        ConfigLoad::Missing(path) if runtime.addon_dir_override.is_none() => {
            println!(
                "config missing at {}. run `{}` to initialize v2 first.",
                path.display(),
                tui_command_hint(&runtime.profile_name)
            );
            CommandOutcome::Skipped
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) if effective_addon_dir.is_none() => {
            println!(
                "update skipped: profile={}, tracked_addons={}, targets={}, addon_dir=<unconfigured>, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                render_target_list(&addons),
                force,
                dry_run
            );
            CommandOutcome::Skipped
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) => {
            let addon_dir = effective_addon_dir.expect("checked above");
            let api_key = resolve_wago_api_key(&config_store, &config_state)?;
            if !dry_run {
                let backup_settings =
                    pre_update_backup_settings(&config_state, &paths.data_dir, &addon_dir);
                let backup = run_pre_update_backup(
                    &backup_settings,
                    &addon_dir,
                    time::OffsetDateTime::now_utc(),
                );
                if let Some(line) = backup.cli_line() {
                    println!("{line}");
                }
            }
            let run = apply_live_updates(
                &mut database,
                &addon_dir,
                &addons,
                api_key.as_deref(),
                force,
                dry_run,
                |_, _, _| {},
            )
            .await?;
            let summary = run.summary;
            println!(
                "update summary: profile={}, tracked_addons={}, targets={}, updated={}, up_to_date={}, remote_older={}, skipped_manual={}, skipped_no_api_key={}, errors={}, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                render_target_list(&addons),
                summary.updated_addons,
                summary.up_to_date,
                summary.remote_older,
                summary.skipped_manual,
                summary.skipped_no_api_key,
                summary.errors,
                force,
                dry_run
            );
            print_update_details(&addons, &run.results);
            CommandOutcome::from_error_count(summary.errors)
        }
    };

    Ok(outcome)
}

/// Backup settings from the loaded config; a missing config uses the defaults.
/// The backup folder is `addon_dir`'s client folder, so the 60-minute throttle and
/// retention never count another client's archives.
fn pre_update_backup_settings(
    config_state: &ConfigLoad,
    data_dir: &std::path::Path,
    addon_dir: &std::path::Path,
) -> PreUpdateBackupSettings {
    let defaults;
    let config = match config_state {
        ConfigLoad::Loaded(config) => config,
        ConfigLoad::Missing(_) => {
            defaults = lemonup_core::AppConfig::default();
            &defaults
        }
    };
    PreUpdateBackupSettings {
        enabled: config.backup_wtf,
        retention: config.backup_retention,
        backup_dir: client_backup_dir(&backup_root(data_dir), addon_dir),
    }
}

fn run_sync(
    paths: AppPaths,
    runtime: AppRuntime,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let mut outcome = CommandOutcome::Success;
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());

    match config_state {
        ConfigLoad::Missing(path) if runtime.addon_dir_override.is_none() => {
            println!(
                "sync skipped: profile={}, addon_dir=<unconfigured>, config={}",
                runtime.profile_name,
                path.display()
            );
            outcome = CommandOutcome::Skipped;
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) if effective_addon_dir.is_none() => {
            println!(
                "sync skipped: profile={}, addon_dir=<unconfigured>",
                runtime.profile_name
            );
            outcome = CommandOutcome::Skipped;
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) => {
            let addon_dir = effective_addon_dir.expect("checked above");
            if runtime.is_guarded_path(&addon_dir) {
                return Err(Box::new(LemonupError::InvalidArgument(format!(
                    "profile '{}' refuses to target the default profile addon directory: {}",
                    runtime.profile_name,
                    addon_dir.display()
                ))));
            }

            validate_addons_path(&addon_dir).map_err(|error| {
                Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
            })?;

            let mut database = StateDatabase::open(&paths.state_db_file)?;
            ensure_bound_addon_dir(
                &database,
                &paths,
                &runtime,
                configured_addon_dir.as_deref(),
                &addon_dir,
                true,
            )?;
            let tracked = database.list_addons()?;
            let report = scan_addons_dir_report(
                &addon_dir,
                GameFlavor::for_addons_dir(&addon_dir),
                &tracked,
            )?;
            let summary = database.reconcile_scan(&report)?;

            println!(
                "{}",
                format_sync_summary(&runtime.profile_name, &addon_dir, &summary)
            );
            if let Some(detail) = format_skipped_detail(&summary) {
                println!("{detail}");
            }
            for warning in leftover_staging_warnings(&addon_dir, &summary) {
                eprintln!("{warning}");
            }
        }
    }

    Ok(outcome)
}

/// The `sync summary:` line. New fields are appended at the end so parsers that read the
/// earlier `key=value` pairs keep working.
fn format_sync_summary(profile: &str, addon_dir: &Path, summary: &ScanSummary) -> String {
    format!(
        "sync summary: profile={}, addon_dir={}, scanned_addons={}, upserted_addons={}, removed_addons={}, skipped={}",
        profile,
        addon_dir.display(),
        summary.scanned_addons,
        summary.upserted_addons,
        summary.removed_addons,
        summary.skipped_folders.len()
    )
}

/// Names the addon folders the scan could not read (they were left as they were).
fn format_skipped_detail(summary: &ScanSummary) -> Option<String> {
    (!summary.skipped_folders.is_empty()).then(|| {
        format!(
            "sync detail: skipped (unreadable): {}",
            summary.skipped_folders.join(", ")
        )
    })
}

/// One warning per `.lemonup-staging-*` folder an earlier update left in the AddOns folder.
fn leftover_staging_warnings(addon_dir: &Path, summary: &ScanSummary) -> Vec<String> {
    summary
        .leftover_staging
        .iter()
        .map(|name| {
            format!(
                "warning: leftover update staging folder {} may contain previous addon versions; see Troubleshooting",
                addon_dir.join(name).display()
            )
        })
        .collect()
}

fn render_target_list(addons: &[String]) -> String {
    if addons.is_empty() {
        "all".to_string()
    } else {
        addons.join("|")
    }
}

fn run_export_addons(
    paths: AppPaths,
    output: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let database = StateDatabase::open(paths.state_db_file)?;
    let addons = database.list_addons()?;
    let output_path = output.unwrap_or(default_transfer_path()?);
    let export = export_addons(&addons, &output_path)?;
    println!(
        "export summary: output={}, addons={}",
        output_path.display(),
        export.addons.len()
    );
    for addon in &export.addons {
        if let Some(source) = addon.unknown_source() {
            println!(
                "export warning: {}: linked to '{source}' by a newer version of LemonUp; saved, but only a newer version can reinstall it",
                addon.name
            );
        }
    }
    Ok(())
}

async fn run_import_addons(
    paths: AppPaths,
    runtime: AppRuntime,
    input: Option<PathBuf>,
    dry_run: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let input_path = input.unwrap_or(default_transfer_path()?);
    let transfer = parse_import_file(&input_path)?;

    let (config_store, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());

    let mut database = StateDatabase::open(paths.state_db_file.clone())?;
    // Same order as every other command: guard, validate, and only then bind. A dry run
    // checks the folder against the binding but never adopts one (and moves no backups),
    // so previewing an import cannot bind the profile to a folder it has not used yet.
    if let Some(addon_dir) = effective_addon_dir.as_deref() {
        if runtime.is_guarded_path(addon_dir) {
            return Err(Box::new(LemonupError::InvalidArgument(format!(
                "profile '{}' refuses to target the default profile addon directory: {}",
                runtime.profile_name,
                addon_dir.display()
            ))));
        }

        validate_addons_path(addon_dir).map_err(|error| {
            Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
        })?;
        ensure_bound_addon_dir(
            &database,
            &paths,
            &runtime,
            configured_addon_dir.as_deref(),
            addon_dir,
            !dry_run,
        )?;
    }
    let tracked = database.list_addons()?;
    let existing_folders =
        collect_existing_import_folders(effective_addon_dir.as_deref(), &tracked).map_err(
            |error| Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>,
        )?;
    let analysis = analyze_import(&transfer, &existing_folders);
    print_import_analysis(&input_path, &analysis);

    if dry_run {
        return Ok(CommandOutcome::Success);
    }

    let Some(addon_dir) = effective_addon_dir else {
        return Err(Box::new(LemonupError::InvalidArgument(
            "cannot import without a configured addon directory".to_string(),
        )));
    };

    let wago_api_key = resolve_wago_api_key(&config_store, &config_state)?;
    let summary = install_transfer_addons(
        &mut database,
        &addon_dir,
        &analysis,
        wago_api_key.as_deref(),
    )
    .await;

    println!(
        "import summary: input={}, installed={}, already_installed={}, manual_skipped={}, duplicate_skipped={}, unsupported_skipped={}, errors={}",
        input_path.display(),
        summary.installed,
        analysis.already_installed.len(),
        analysis.manual_addons.len(),
        analysis.duplicate_addons.len(),
        analysis.unsupported_addons.len(),
        summary.errors.len()
    );
    let error_count = summary.errors.len();
    for error in summary.errors {
        println!("import error: {error}");
    }
    for warning in summary.warnings {
        println!("import warning: {warning}");
    }
    Ok(CommandOutcome::from_error_count(error_count))
}

fn run_restore_backup(
    paths: AppPaths,
    runtime: AppRuntime,
    archive: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone())
        .ok_or_else(|| {
            Box::new(LemonupError::InvalidArgument(
                "cannot restore backup without a configured addon directory".to_string(),
            )) as Box<dyn std::error::Error>
        })?;

    if runtime.is_guarded_path(&effective_addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            effective_addon_dir.display()
        ))));
    }

    validate_addons_path(&effective_addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;

    let database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &effective_addon_dir,
        true,
    )?;
    drop(database);

    let backup_dir = client_backup_dir(&backup_root(&paths.data_dir), &effective_addon_dir);
    let backup_path = match archive {
        Some(path) => path,
        None => list_backups(&backup_dir)?
            .into_iter()
            .next()
            .map(|entry| entry.path)
            .ok_or_else(|| {
                Box::new(LemonupError::InvalidArgument(
                    "no backups found to restore".to_string(),
                )) as Box<dyn std::error::Error>
            })?,
    };

    restore_wtf_backup(&effective_addon_dir, &backup_dir, &backup_path)
        .map_err(Box::<dyn std::error::Error>::from)?;
    println!("restore summary: archive={}", backup_path.display());
    Ok(())
}

fn print_import_analysis(input_path: &std::path::Path, analysis: &ImportAnalysis) {
    println!(
        "import analysis: input={}, install={}, already_installed={}, manual={}, duplicates={}, unsupported={}",
        input_path.display(),
        analysis.to_install.len(),
        analysis.already_installed.len(),
        analysis.manual_addons.len(),
        analysis.duplicate_addons.len(),
        analysis.unsupported_addons.len()
    );
    for warning in analysis
        .unsupported_addons
        .iter()
        .filter_map(lemonup_core::TransferAddon::unknown_source_warning)
    {
        println!("import warning: {warning}");
    }
}

fn print_update_details(addons: &[String], results: &[LiveUpdateResult]) {
    for line in build_update_detail_lines(addons, results) {
        println!("{line}");
    }
}

fn build_update_detail_lines(addons: &[String], results: &[LiveUpdateResult]) -> Vec<String> {
    if !addons.is_empty() {
        return results.iter().map(format_update_result_line).collect();
    }

    let skipped_manual = results
        .iter()
        .filter(|result| result.status == LiveUpdateStatus::SkippedManual)
        .count();
    let skipped_no_api_key = results
        .iter()
        .filter(|result| result.status == LiveUpdateStatus::SkippedNoApiKey)
        .count();
    // Errors, held-back older remotes, plus updates that succeeded but carry a warning (for
    // example the WoWInterface Forever notice), stay visible in the aggregated view. Up-to-date
    // results carry a boilerplate message and are intentionally not listed.
    let attention_lines = results
        .iter()
        .filter(|result| {
            result.status == LiveUpdateStatus::Error
                || result.status == LiveUpdateStatus::RemoteOlder
                || (result.status == LiveUpdateStatus::Updated && result.message.is_some())
        })
        .map(format_update_result_line)
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    if skipped_manual > 0 || skipped_no_api_key > 0 {
        lines.push(format!(
            "update detail: skipped manual={}, no_api_key={} | rerun `lemonup update <addon...>` for per-addon detail",
            skipped_manual, skipped_no_api_key
        ));
    }

    if lines.is_empty() && attention_lines.is_empty() {
        lines.push(
            "update detail: all processed targets were updated or already up to date".to_string(),
        );
        return lines;
    }

    lines.extend(attention_lines);
    lines
}

fn format_update_result_line(result: &LiveUpdateResult) -> String {
    format!(
        "update result: addon={}, source={}, status={}, previous_version={}, remote_version={}, message={}",
        result.addon_name,
        render_source_kind(result.source),
        serialize_live_update_status(result.status),
        result.previous_version.as_deref().unwrap_or("<unknown>"),
        result.remote_version.as_deref().unwrap_or("<unknown>"),
        result.message.as_deref().unwrap_or("<none>")
    )
}

fn render_source_kind(source: lemonup_core::SourceKind) -> &'static str {
    match source {
        lemonup_core::SourceKind::GitHub => "GitHub",
        lemonup_core::SourceKind::Tukui => "TukUI",
        lemonup_core::SourceKind::WowInterface => "WoWInterface",
        lemonup_core::SourceKind::Wago => "Wago",
        lemonup_core::SourceKind::Manual => "manual",
    }
}

async fn run_install_wago(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    stability: WagoStability,
    dry_run: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (config_store, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());
    let Some(addon_dir) = effective_addon_dir else {
        println!(
            "install skipped: profile={}, addon={}, addon_dir=<unconfigured>, dry_run={}",
            runtime.profile_name, addon, dry_run
        );
        return Ok(CommandOutcome::Skipped);
    };

    if runtime.is_guarded_path(&addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            addon_dir.display()
        ))));
    }

    validate_addons_path(&addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;

    let mut database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        !dry_run,
    )?;

    let Some(api_key) = resolve_wago_api_key(&config_store, &config_state)? else {
        println!(
            "install skipped: profile={}, addon={}, reason=no Wago API key configured",
            runtime.profile_name, addon
        );
        return Ok(CommandOutcome::Skipped);
    };

    let summary = install_wago_addon(
        &mut database,
        &addon_dir,
        addon,
        &api_key,
        stability,
        dry_run,
    )
    .await
    .map_err(Box::<dyn std::error::Error>::from)?;

    println!(
        "wago install: profile={}, addon_id={}, addon_name={}, parent={}, folders={}, stability={}, version={}, dry_run={}",
        runtime.profile_name,
        summary.addon_id,
        summary.addon_name,
        summary.parent_folder,
        summary.installed_folders.join("|"),
        summary.stability.as_str(),
        summary.version.as_deref().unwrap_or("<unknown>"),
        summary.dry_run
    );

    Ok(CommandOutcome::Success)
}

async fn run_install_tukui(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    dry_run: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());
    let Some(addon_dir) = effective_addon_dir else {
        println!(
            "install skipped: profile={}, addon={}, addon_dir=<unconfigured>, dry_run={}",
            runtime.profile_name, addon, dry_run
        );
        return Ok(CommandOutcome::Skipped);
    };

    if runtime.is_guarded_path(&addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            addon_dir.display()
        ))));
    }

    validate_addons_path(&addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;

    let mut database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        !dry_run,
    )?;

    let summary = install_tukui_addon(&mut database, &addon_dir, addon, dry_run)
        .await
        .map_err(Box::<dyn std::error::Error>::from)?;

    println!(
        "tukui install: profile={}, addon_slug={}, addon_name={}, parent={}, folders={}, version={}, dry_run={}",
        runtime.profile_name,
        summary.addon_slug,
        summary.addon_name,
        summary.parent_folder,
        summary.installed_folders.join("|"),
        summary.version.as_deref().unwrap_or("<unknown>"),
        summary.dry_run
    );

    Ok(CommandOutcome::Success)
}

async fn run_install_wowinterface(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    dry_run: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());
    let Some(addon_dir) = effective_addon_dir else {
        println!(
            "install skipped: profile={}, addon={}, addon_dir=<unconfigured>, dry_run={}",
            runtime.profile_name, addon, dry_run
        );
        return Ok(CommandOutcome::Skipped);
    };

    if runtime.is_guarded_path(&addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            addon_dir.display()
        ))));
    }

    validate_addons_path(&addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;

    let mut database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        !dry_run,
    )?;

    let summary = install_wowinterface_addon(&mut database, &addon_dir, addon, dry_run)
        .await
        .map_err(Box::<dyn std::error::Error>::from)?;

    println!(
        "wowinterface install: profile={}, addon_id={}, addon_name={}, parent={}, folders={}, version={}, dry_run={}",
        runtime.profile_name,
        summary.addon_id,
        summary.addon_name,
        summary.parent_folder,
        summary.installed_folders.join("|"),
        summary.version.as_deref().unwrap_or("<unknown>"),
        summary.dry_run
    );
    if let Some(warning) = &summary.warning {
        println!("wowinterface warning: {warning}");
    }

    Ok(CommandOutcome::Success)
}

async fn run_install_github(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    dry_run: bool,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let effective_addon_dir = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone());
    let Some(addon_dir) = effective_addon_dir else {
        println!(
            "install skipped: profile={}, addon={}, addon_dir=<unconfigured>, dry_run={}",
            runtime.profile_name, addon, dry_run
        );
        return Ok(CommandOutcome::Skipped);
    };

    if runtime.is_guarded_path(&addon_dir) {
        return Err(Box::new(LemonupError::InvalidArgument(format!(
            "profile '{}' refuses to target the default profile addon directory: {}",
            runtime.profile_name,
            addon_dir.display()
        ))));
    }

    validate_addons_path(&addon_dir).map_err(|error| {
        Box::new(LemonupError::InvalidArgument(error)) as Box<dyn std::error::Error>
    })?;

    let mut database = StateDatabase::open(&paths.state_db_file)?;
    ensure_bound_addon_dir(
        &database,
        &paths,
        &runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        !dry_run,
    )?;

    let summary = install_github_addon(&mut database, &addon_dir, addon, dry_run)
        .await
        .map_err(Box::<dyn std::error::Error>::from)?;

    println!(
        "github install: profile={}, repo_url={}, repo_name={}, parent={}, folders={}, version={}, commit={}, dry_run={}",
        runtime.profile_name,
        summary.repo_url,
        summary.repo_name,
        summary.parent_folder,
        summary.installed_folders.join("|"),
        summary.version.as_deref().unwrap_or("<unknown>"),
        summary.git_commit,
        summary.dry_run
    );

    Ok(CommandOutcome::Success)
}

/// Refuses to let `addon_dir` touch this profile's tracked state or backups unless it is
/// the folder that state is bound to. A database from before binding existed adopts the
/// profile's configured folder (or `addon_dir` when none is configured) first. On a match,
/// archives in the legacy flat backup root move into the bound client's backup folder.
///
/// Callers validate `addon_dir` (default-profile guard, `validate_addons_path`) first so an
/// unvalidated path can never become a binding. `adopt` is false for dry runs: the check
/// still refuses a different folder, but nothing is bound and no backups are moved.
fn ensure_bound_addon_dir(
    database: &StateDatabase,
    paths: &AppPaths,
    runtime: &AppRuntime,
    configured_addon_dir: Option<&Path>,
    addon_dir: &Path,
    adopt: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match check_addon_dir_binding(
        database,
        &runtime.profile_name,
        configured_addon_dir,
        addon_dir,
        adopt,
    )? {
        BindingCheck::Bound => {
            if adopt
                && let Err(error) = adopt_legacy_backups(&backup_root(&paths.data_dir), addon_dir)
            {
                tracing::warn!(%error, "could not move legacy WTF backups into the client folder");
            }
            Ok(())
        }
        BindingCheck::Mismatch(mismatch) => Err(Box::new(LemonupError::InvalidArgument(
            mismatch.to_string(),
        ))),
    }
}

/// Folders a non-default profile must never target: the default profile's configured
/// AddOns folder and the folder its state database is bound to. A default profile that was
/// only ever pointed at its folder with `--addon-dir` has a binding but no config entry.
/// The default profile's files are only read, never created or modified.
fn load_guarded_addon_dirs(profile: &str) -> lemonup_core::Result<Vec<PathBuf>> {
    if profile == DEFAULT_PROFILE {
        return Ok(Vec::new());
    }

    guarded_dirs_of(&AppPaths::discover(DEFAULT_PROFILE)?)
}

fn guarded_dirs_of(default_paths: &AppPaths) -> lemonup_core::Result<Vec<PathBuf>> {
    let mut guarded = Vec::new();
    let config_store = ConfigStore::new(default_paths.config_file.clone());
    if let ConfigLoad::Loaded(config) = config_store.load()?
        && let Some(addon_dir) = config.addon_dir
    {
        guarded.push(addon_dir);
    }
    if let Some(bound) = StateDatabase::read_bound_addon_dir(&default_paths.state_db_file)? {
        guarded.push(bound);
    }
    Ok(guarded)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        CommandOutcome, LogDestination, build_update_detail_lines, count_check_errors,
        default_log_filter, log_destination, log_writer, open_log_file, pre_update_backup_settings,
        refresh_managed_update_state, tui_command_hint,
    };
    use crate::update::{
        CheckResult, LiveUpdateResult, LiveUpdateStatus, build_managed_update_record,
        build_update_checks, determine_update_status,
    };
    use lemonup_core::{
        AddonKind, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase, UpdateStatus,
    };

    #[test]
    fn default_log_filter_is_quiet_in_release_and_verbose_in_debug() {
        assert_eq!(default_log_filter(false), "warn");
        assert_eq!(
            default_log_filter(true),
            "info,lemonup_app=debug,lemonup_core=debug"
        );
        // Both defaults must parse as valid filter directives.
        assert!(tracing_subscriber::EnvFilter::try_new(default_log_filter(false)).is_ok());
        assert!(tracing_subscriber::EnvFilter::try_new(default_log_filter(true)).is_ok());
    }

    fn log_test_paths(root: &std::path::Path) -> lemonup_core::AppPaths {
        lemonup_core::AppPaths {
            profile: lemonup_core::DEFAULT_PROFILE.to_string(),
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            log_dir: root.join("data").join("logs"),
            config_file: root.join("config").join("config.toml"),
            state_db_file: root.join("data").join("state.sqlite"),
            roots: lemonup_core::ProfileRoots::new(
                root.join("config"),
                root.join("data"),
                root.join("cache"),
            ),
        }
    }

    #[test]
    fn log_destination_is_a_file_for_tui_and_stderr_for_cli_commands() {
        use crate::cli::Commands;

        let temp = tempdir().expect("tempdir");
        let paths = log_test_paths(temp.path());
        let expected = LogDestination::File(paths.data_dir.join("logs").join("lemonup.log"));

        assert_eq!(log_destination(&Commands::Tui, &paths), expected);
        for command in [
            Commands::Sync,
            Commands::Check { addons: Vec::new() },
            Commands::UpdateAll {
                force: false,
                dry_run: false,
            },
            Commands::ExportAddons { output: None },
        ] {
            assert_eq!(
                log_destination(&command, &paths),
                LogDestination::Stderr,
                "{command:?}"
            );
        }
    }

    #[test]
    fn tui_log_file_is_scoped_to_the_profile_data_dir() {
        let temp = tempdir().expect("tempdir");
        let mut paths = log_test_paths(temp.path());
        paths.profile = "dev".to_string();
        paths.data_dir = temp.path().join("data").join("profiles").join("dev");
        paths.log_dir = paths.data_dir.join("logs");

        assert_eq!(
            super::tui_log_file(&paths),
            temp.path()
                .join("data")
                .join("profiles")
                .join("dev")
                .join("logs")
                .join("lemonup.log")
        );
    }

    #[test]
    fn open_log_file_creates_dirs_and_file_and_appends() {
        use std::io::Write;

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("nested").join("logs").join("lemonup.log");

        let mut file = open_log_file(&path).expect("creates log file");
        writeln!(file, "first").expect("write");
        drop(file);
        let mut file = open_log_file(&path).expect("reopens log file");
        writeln!(file, "second").expect("write");
        drop(file);

        assert_eq!(
            std::fs::read_to_string(&path).expect("read log"),
            "first\nsecond\n"
        );
    }

    #[test]
    fn unopenable_log_file_falls_back_to_a_silent_writer() {
        use std::io::Write;
        use tracing_subscriber::fmt::MakeWriter;

        let temp = tempdir().expect("tempdir");
        // A regular file where the log directory should be makes the path unopenable.
        let blocker = temp.path().join("logs");
        std::fs::write(&blocker, "not a directory").expect("blocker");
        let path = blocker.join("lemonup.log");
        assert!(open_log_file(&path).is_none());

        let writer = log_writer(&LogDestination::File(path));
        let mut sink = writer.make_writer();
        sink.write_all(b"dropped")
            .expect("silent writer accepts writes");
    }

    #[test]
    fn log_writer_writes_json_lines_to_the_file() {
        use tracing_subscriber::fmt;

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("logs").join("lemonup.log");
        let subscriber = fmt()
            .with_target(false)
            .with_writer(log_writer(&LogDestination::File(path.clone())))
            .json()
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("tui log line");
        });

        let contents = std::fs::read_to_string(&path).expect("read log");
        assert!(
            contents.contains("\"message\":\"tui log line\""),
            "{contents}"
        );
    }

    #[test]
    fn tui_command_hint_puts_global_profile_option_before_subcommand() {
        assert_eq!(
            tui_command_hint(lemonup_core::DEFAULT_PROFILE),
            "lemonup tui"
        );
        assert_eq!(
            tui_command_hint("manual-smoke"),
            "lemonup --profile manual-smoke tui"
        );
    }

    fn write_addon(root: &std::path::Path, folder: &str, toc_body: &str) {
        let addon_dir = root.join(folder);
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        std::fs::write(addon_dir.join(format!("{folder}.toc")), toc_body).expect("write toc");
    }

    fn check_result(addon_name: &str, status: UpdateStatus) -> CheckResult {
        CheckResult {
            addon_name: addon_name.to_string(),
            status,
            remote_version: None,
            message: None,
            error_kind: None,
        }
    }

    #[test]
    fn zero_errors_exits_zero_and_any_error_exits_one() {
        assert_eq!(CommandOutcome::from_error_count(0), CommandOutcome::Success);
        assert_eq!(CommandOutcome::from_error_count(0).exit_status(), 0);
        assert_eq!(
            CommandOutcome::from_error_count(1),
            CommandOutcome::Failures(1)
        );
        assert_eq!(CommandOutcome::from_error_count(1).exit_status(), 1);
        assert_eq!(CommandOutcome::from_error_count(7).exit_status(), 1);
    }

    #[test]
    fn skipped_install_outcome_exits_nonzero() {
        assert_eq!(CommandOutcome::Skipped.exit_status(), 1);
        assert_ne!(CommandOutcome::Skipped, CommandOutcome::Success);
        assert_eq!(CommandOutcome::Success.exit_status(), 0);
    }

    #[test]
    fn skipped_update_and_sync_exit_nonzero_while_dry_run_success_stays_zero() {
        // update/update-all/sync reuse `Skipped` for their skip paths; a completed run
        // (including --dry-run) is `from_error_count(0)`, which stays 0.
        assert_eq!(CommandOutcome::Skipped.exit_status(), 1);
        assert_eq!(CommandOutcome::from_error_count(0).exit_status(), 0);
    }

    #[test]
    fn pre_update_backup_settings_follow_config_and_default_when_missing() {
        let data_dir = std::path::Path::new("data");
        let forever = std::path::Path::new("WoW/_forever_/Interface/AddOns");

        let config = lemonup_core::AppConfig {
            backup_wtf: false,
            backup_retention: 9,
            ..lemonup_core::AppConfig::default()
        };
        let loaded = pre_update_backup_settings(
            &lemonup_core::ConfigLoad::Loaded(config),
            data_dir,
            forever,
        );
        assert!(!loaded.enabled);
        assert_eq!(loaded.retention, 9);
        assert_eq!(
            loaded.backup_dir,
            crate::backup::backup_root(data_dir).join("forever")
        );

        let missing = pre_update_backup_settings(
            &lemonup_core::ConfigLoad::Missing(std::path::PathBuf::from("config.toml")),
            data_dir,
            forever,
        );
        assert!(missing.enabled);
        assert_eq!(missing.retention, 5);
    }

    #[test]
    fn check_exit_status_counts_only_error_status() {
        let clean = [
            check_result("A", UpdateStatus::UpToDate),
            check_result("B", UpdateStatus::UpdateAvailable),
            check_result("C", UpdateStatus::Unknown),
        ];
        assert_eq!(count_check_errors(&clean), 0);
        assert_eq!(
            CommandOutcome::from_error_count(count_check_errors(&clean)).exit_status(),
            0
        );

        let failing = [
            check_result("A", UpdateStatus::UpdateAvailable),
            check_result("B", UpdateStatus::Error),
        ];
        assert_eq!(count_check_errors(&failing), 1);
        assert_eq!(
            CommandOutcome::from_error_count(count_check_errors(&failing)).exit_status(),
            1
        );
    }

    fn update_result(
        addon_name: &str,
        status: LiveUpdateStatus,
        source: SourceKind,
    ) -> LiveUpdateResult {
        LiveUpdateResult {
            addon_name: addon_name.to_string(),
            source,
            status,
            previous_version: Some("1.0.0".to_string()),
            remote_version: Some("1.1.0".to_string()),
            message: Some("detail".to_string()),
        }
    }

    #[test]
    fn refresh_managed_update_state_records_managed_rows() {
        let temp = tempdir().expect("tempdir");
        let addons_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addons_dir).expect("create addons dir");
        write_addon(
            &addons_dir,
            "DBM-Core",
            "## Title: Deadly Boss Mods\n## Version: 11.0.2\n## Author: MysticalOS\n## Interface: 110002\n",
        );
        write_addon(
            &addons_dir,
            "DBM-Naxx",
            "## Title: DBM Naxx\n## Version: 11.0.2\n## Author: MysticalOS\n## Interface: 110002\n",
        );
        write_addon(
            &addons_dir,
            "DBM-Ulduar",
            "## Title: DBM Ulduar\n## Version: 11.0.2\n## Author: MysticalOS\n## Interface: 110002\n",
        );

        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut parent = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        parent.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "DBM-Naxx".to_string(),
            },
            OwnedFolder {
                name: "DBM-Ulduar".to_string(),
            },
        ]);
        parent.remote_version = Some("11.0.2".to_string());
        database.record_managed_addon(&parent).expect("seed parent");
        database
            .upsert_addon(&lemonup_core::AddonRecord::new(
                "DBM Ulduar",
                "DBM-Ulduar",
                SourceKind::Manual,
            ))
            .expect("seed stale child row");

        let summary =
            refresh_managed_update_state(&mut database, &addons_dir, false).expect("refresh");

        assert_eq!(summary.target_addons, 2);
        assert_eq!(summary.scanned_addons, 3);
        assert_eq!(summary.refreshed_addons, 1);
        assert_eq!(summary.skipped_unmanaged, 0);
        assert_eq!(summary.missing_on_disk, 0);

        let parent = database
            .get_addon_by_folder("DBM-Core")
            .expect("get parent")
            .expect("parent exists");
        assert!(parent.has_authoritative_owned_folders());
        assert_eq!(parent.version.as_deref(), Some("11.0.2"));
        assert_eq!(parent.author.as_deref(), Some("MysticalOS"));
        assert!(
            database
                .get_addon_by_folder("DBM-Ulduar")
                .expect("get stale child")
                .is_none()
        );
    }

    #[test]
    fn refresh_managed_update_state_dry_run_does_not_mutate_state() {
        let temp = tempdir().expect("tempdir");
        let addons_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addons_dir).expect("create addons dir");
        write_addon(
            &addons_dir,
            "DBM-Core",
            "## Title: Deadly Boss Mods\n## Version: 11.0.2\n",
        );
        write_addon(
            &addons_dir,
            "DBM-Naxx",
            "## Title: DBM Naxx\n## Version: 11.0.2\n",
        );

        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut parent = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        parent.version = Some("11.0.1".to_string());
        parent.set_managed_owned_folders(vec![OwnedFolder {
            name: "DBM-Naxx".to_string(),
        }]);
        database.record_managed_addon(&parent).expect("seed parent");

        let summary =
            refresh_managed_update_state(&mut database, &addons_dir, true).expect("dry-run");

        assert_eq!(summary.target_addons, 1);
        assert_eq!(summary.refreshed_addons, 1);
        let parent = database
            .get_addon_by_folder("DBM-Core")
            .expect("get parent")
            .expect("parent exists");
        assert_eq!(parent.version.as_deref(), Some("11.0.1"));
    }

    #[test]
    fn build_managed_update_record_preserves_kind_override_and_owned_folders() {
        let mut existing = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        existing.kind = AddonKind::Library;
        existing.kind_override = true;
        existing.set_managed_owned_folders(vec![OwnedFolder {
            name: "DBM-Naxx".to_string(),
        }]);

        let scanned = ScannedAddon {
            name: "Deadly Boss Mods".to_string(),
            folder: "DBM-Core".to_string(),
            owned_folders: Vec::new(),
            kind: AddonKind::Addon,
            flavor: GameFlavor::Retail,
            version: Some("11.0.2".to_string()),
            git_commit: None,
            author: Some("MysticalOS".to_string()),
            interface: Some("110002".to_string()),
            source: SourceKind::Manual,
            required_deps: vec!["Ace3".to_string()],
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
            notes: None,
            category: None,
            toc_title: None,
        };

        let refreshed = build_managed_update_record(&existing, &scanned);
        assert_eq!(refreshed.kind, AddonKind::Library);
        assert!(refreshed.kind_override);
        assert!(refreshed.has_authoritative_owned_folders());
        assert_eq!(refreshed.owned_folders.len(), 1);
        assert_eq!(refreshed.owned_folders[0].name, "DBM-Naxx");
        assert_eq!(refreshed.required_deps, vec!["Ace3"]);
    }

    #[test]
    fn build_update_checks_filters_targets_and_reports_statuses() {
        let mut dbm = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        dbm.version = Some("11.0.0".to_string());
        dbm.remote_version = Some("11.0.2".to_string());

        let mut details =
            lemonup_core::AddonRecord::new("Details! Damage Meter", "Details", SourceKind::Wago);
        details.version = Some("11.0.2".to_string());
        details.remote_version = Some("11.0.2".to_string());

        let manual = lemonup_core::AddonRecord::new("Scratch", "Scratch", SourceKind::Manual);

        let checks = build_update_checks(
            &[dbm, details, manual],
            &["dbm-core".to_string(), "Details! Damage Meter".to_string()],
        )
        .expect("build checks");

        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].addon_name, "DBM-Core");
        assert_eq!(checks[0].status, UpdateStatus::UpdateAvailable);
        assert_eq!(checks[1].addon_name, "Details");
        assert_eq!(checks[1].status, UpdateStatus::UpToDate);
    }

    #[test]
    fn build_update_checks_rejects_unknown_selectors() {
        let addon = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        let error = build_update_checks(&[addon], &["Unknown".to_string()]).expect_err("missing");
        assert!(
            error
                .to_string()
                .contains("No linked addon matches: Unknown")
        );
    }

    #[test]
    fn determine_update_status_handles_manual_and_missing_versions() {
        let manual = lemonup_core::AddonRecord::new("Scratch", "Scratch", SourceKind::Manual);
        let (status, message) = determine_update_status(&manual);
        assert_eq!(status, UpdateStatus::Unknown);
        assert_eq!(
            message.as_deref(),
            Some("addons that are not linked cannot be checked yet")
        );

        let tracked = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        let (status, message) = determine_update_status(&tracked);
        assert_eq!(status, UpdateStatus::Unknown);
        assert_eq!(message.as_deref(), Some("not checked yet"));
    }

    #[test]
    fn update_all_detail_lines_aggregate_skip_noise() {
        let lines = build_update_detail_lines(
            &[],
            &[
                update_result(
                    "HandyNotes",
                    LiveUpdateStatus::SkippedManual,
                    SourceKind::Manual,
                ),
                update_result("Pawn", LiveUpdateStatus::SkippedManual, SourceKind::Manual),
            ],
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            "update detail: skipped manual=2, no_api_key=0 | rerun `lemonup update <addon...>` for per-addon detail"
        );
    }

    #[test]
    fn update_all_detail_lines_keep_error_rows_actionable() {
        let lines = build_update_detail_lines(
            &[],
            &[update_result(
                "WeakAuras",
                LiveUpdateStatus::Error,
                SourceKind::Wago,
            )],
        );

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("update result: addon=WeakAuras"));
        assert!(lines[0].contains("status=error"));
    }

    #[test]
    fn update_all_detail_lines_show_warnings_on_updated_results() {
        let mut warned = update_result(
            "Details",
            LiveUpdateStatus::Updated,
            SourceKind::WowInterface,
        );
        warned.message = Some("WoWInterface may not support WoW Forever".to_string());
        let mut quiet = update_result("Pawn", LiveUpdateStatus::Updated, SourceKind::WowInterface);
        quiet.message = None;
        let up_to_date = update_result("Bagnon", LiveUpdateStatus::UpToDate, SourceKind::Wago);

        let lines = build_update_detail_lines(&[], &[warned, quiet, up_to_date]);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("update result: addon=Details"));
        assert!(lines[0].contains("status=updated"));
        assert!(lines[0].contains("message=WoWInterface may not support WoW Forever"));
    }

    #[test]
    fn update_all_detail_lines_list_remote_older_and_count_missing_key_skips() {
        let mut older = update_result(
            "RareScanner",
            LiveUpdateStatus::RemoteOlder,
            SourceKind::WowInterface,
        );
        older.message = Some(
            "remote version 9.2.5.1 is older than installed 12.1.5.3; use --force to install it anyway"
                .to_string(),
        );
        let lines = build_update_detail_lines(
            &[],
            &[
                older,
                update_result("Pawn", LiveUpdateStatus::SkippedNoApiKey, SourceKind::Wago),
            ],
        );

        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "update detail: skipped manual=0, no_api_key=1 | rerun `lemonup update <addon...>` for per-addon detail"
        );
        assert!(lines[1].contains("addon=RareScanner"));
        assert!(lines[1].contains("status=remote_older"));
    }

    #[test]
    fn remote_older_and_missing_key_skips_do_not_fail_the_command() {
        let checks = vec![crate::update::CheckResult {
            addon_name: "Postal".to_string(),
            status: UpdateStatus::RemoteOlder,
            remote_version: Some("v3.6.6".to_string()),
            message: None,
            error_kind: None,
        }];
        assert_eq!(count_check_errors(&checks), 0);
        assert_eq!(
            CommandOutcome::from_error_count(count_check_errors(&checks)).exit_status(),
            0
        );
    }

    #[test]
    fn targeted_update_detail_lines_keep_per_addon_rows() {
        let lines = build_update_detail_lines(
            &["WeakAuras".to_string()],
            &[update_result(
                "WeakAuras",
                LiveUpdateStatus::SkippedManual,
                SourceKind::Manual,
            )],
        );

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("update result: addon=WeakAuras"));
        assert!(lines[0].contains("status=skipped_manual"));
    }
}

#[cfg(test)]
mod v1_migration_tests {
    use tempfile::tempdir;

    use super::run_v1_migration;
    use crate::app::{App, AppRuntime};
    use lemonup_core::{AppPaths, DEFAULT_PROFILE, V1Location};

    fn default_paths(root: &std::path::Path) -> AppPaths {
        AppPaths {
            profile: DEFAULT_PROFILE.to_string(),
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            log_dir: root.join("data").join("logs"),
            config_file: root.join("config").join("config.toml"),
            state_db_file: root.join("data").join("state.sqlite"),
            roots: lemonup_core::ProfileRoots::new(
                root.join("config"),
                root.join("data"),
                root.join("cache"),
            ),
        }
    }

    #[test]
    fn corrupt_v1_config_yields_notice_and_startup_continues() {
        let temp = tempdir().expect("tempdir");
        let v1_dir = temp.path().join("v1");
        std::fs::create_dir_all(&v1_dir).expect("v1 dir");
        std::fs::write(v1_dir.join("config.json"), "{ broken").expect("corrupt config");
        let paths = default_paths(&temp.path().join("v2"));

        let notice =
            run_v1_migration(&paths, Some(V1Location::from_dir(&v1_dir))).expect("failure notice");
        assert!(notice.is_error);
        assert!(notice.text.contains("v1 migration failed"));
        assert!(!paths.config_file.exists());

        let mut app = App::bootstrap(
            paths,
            AppRuntime::new(DEFAULT_PROFILE.to_string(), None, Vec::new()),
        )
        .expect("bootstrap continues to onboarding");
        app.show_startup_notice(&notice.text, notice.is_error);
    }

    #[test]
    fn missing_v1_location_is_silent() {
        let temp = tempdir().expect("tempdir");
        let paths = default_paths(temp.path());
        assert!(run_v1_migration(&paths, None).is_none());
        assert!(
            run_v1_migration(&paths, Some(V1Location::from_dir(temp.path().join("none"))))
                .is_none()
        );
    }

    #[test]
    fn config_only_migration_prints_summary() {
        let temp = tempdir().expect("tempdir");
        let v1_dir = temp.path().join("v1");
        std::fs::create_dir_all(&v1_dir).expect("v1 dir");
        std::fs::write(
            v1_dir.join("config.json"),
            r#"{"destDir":"NOT_CONFIGURED","theme":"light","wagoApiKey":"key"}"#,
        )
        .expect("v1 config");
        let paths = default_paths(&temp.path().join("v2"));

        let notice =
            run_v1_migration(&paths, Some(V1Location::from_dir(&v1_dir))).expect("success notice");
        assert!(!notice.is_error);
        assert!(notice.text.starts_with("Migrated LemonUp v1 settings"));
        assert!(notice.text.contains("v1 files left untouched"));
        assert!(paths.config_file.exists());
        assert!(run_v1_migration(&paths, Some(V1Location::from_dir(&v1_dir))).is_none());
    }
}

#[cfg(test)]
mod addon_dir_binding_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use lemonup_core::{
        AddonRecord, AppConfig, AppPaths, ConfigStore, DEFAULT_PROFILE, LemonupError, SourceKind,
        StateDatabase,
    };
    use tempfile::tempdir;

    use super::{
        error_text, format_skipped_detail, format_sync_summary, guarded_dirs_of,
        leftover_staging_warnings, run_import_addons, run_restore_backup, run_sync, run_update,
        toc_command,
    };
    use crate::app::AppRuntime;
    use crate::backup::{backup_root, create_wtf_backup, list_backups};

    struct Sandbox {
        _temp: tempfile::TempDir,
        paths: AppPaths,
        retail: PathBuf,
        forever: PathBuf,
    }

    fn client(root: &Path, name: &str) -> PathBuf {
        let addons = root.join(name).join("Interface").join("AddOns");
        fs::create_dir_all(addons.join("SharedAddon")).expect("addon folder");
        fs::write(
            addons.join("SharedAddon").join("SharedAddon.toc"),
            "## Title: SharedAddon\n## Version: 1.0\n",
        )
        .expect("toc");
        fs::create_dir_all(root.join(name).join("WTF")).expect("wtf");
        fs::write(root.join(name).join("WTF").join("Config.wtf"), name).expect("wtf file");
        fs::write(root.join(name).join("Wow.exe"), "").expect("exe");
        addons
    }

    /// Default profile configured for retail, tracking a GitHub addon in it.
    fn sandbox() -> Sandbox {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("v2");
        let paths = AppPaths {
            profile: DEFAULT_PROFILE.to_string(),
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            log_dir: root.join("data").join("logs"),
            config_file: root.join("config").join("config.toml"),
            state_db_file: root.join("data").join("state.sqlite"),
            roots: lemonup_core::ProfileRoots::new(
                root.join("config"),
                root.join("data"),
                root.join("cache"),
            ),
        };
        paths.ensure().expect("ensure paths");
        let wow = temp.path().join("World of Warcraft");
        fs::create_dir_all(wow.join("Data")).expect("data");
        fs::write(wow.join(".build.info"), "").expect("build info");
        let retail = client(&wow, "_retail_");
        let forever = client(&wow, "_forever_");
        fs::create_dir_all(retail.join("GitAddon")).expect("git addon");
        fs::write(
            retail.join("GitAddon").join("GitAddon.toc"),
            "## Title: GitAddon\n## Version: 2.0\n",
        )
        .expect("toc");

        let config = AppConfig {
            addon_dir: Some(retail.clone()),
            ..AppConfig::default()
        };
        ConfigStore::for_profile(&paths)
            .write_new_config(&config)
            .expect("write config");
        let database = StateDatabase::open(&paths.state_db_file).expect("open db");
        let mut tracked = AddonRecord::new("GitAddon", "GitAddon", SourceKind::GitHub);
        tracked.source_url = Some("https://github.com/example/GitAddon".to_string());
        tracked.git_commit = Some("abc123".to_string());
        database.upsert_addon(&tracked).expect("seed tracked addon");

        Sandbox {
            _temp: temp,
            paths,
            retail,
            forever,
        }
    }

    fn runtime(addon_dir: Option<&Path>) -> AppRuntime {
        AppRuntime::new(
            DEFAULT_PROFILE.to_string(),
            addon_dir.map(Path::to_path_buf),
            Vec::new(),
        )
    }

    fn tracked_github(paths: &AppPaths) -> AddonRecord {
        StateDatabase::open(&paths.state_db_file)
            .expect("open db")
            .get_addon_by_folder("GitAddon")
            .expect("query")
            .expect("GitAddon still tracked")
    }

    #[test]
    fn mismatched_addon_dir_refuses_sync_and_keeps_tracking() {
        let sandbox = sandbox();

        let error = run_sync(sandbox.paths.clone(), runtime(Some(&sandbox.forever)))
            .expect_err("sync against forever must be refused");
        let message = error.to_string();
        assert!(
            message.contains("is bound to the AddOns folder"),
            "{message}"
        );
        assert!(message.contains("--profile forever"), "{message}");

        let addon = tracked_github(&sandbox.paths);
        assert_eq!(addon.source, SourceKind::GitHub);
        assert_eq!(addon.git_commit.as_deref(), Some("abc123"));
        let database = StateDatabase::open(&sandbox.paths.state_db_file).expect("open");
        assert_eq!(database.list_addons().expect("list").len(), 1);
        assert_eq!(
            database.bound_addon_dir().expect("binding"),
            Some(sandbox.retail.clone()),
            "a legacy database adopts the configured folder"
        );
    }

    #[test]
    fn matching_addon_dir_syncs() {
        let sandbox = sandbox();
        run_sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("sync retail");
        run_sync(sandbox.paths.clone(), runtime(None)).expect("sync configured retail");
        let addon = tracked_github(&sandbox.paths);
        assert_eq!(addon.source, SourceKind::GitHub);
        let database = StateDatabase::open(&sandbox.paths.state_db_file).expect("open");
        assert!(
            database
                .get_addon_by_folder("SharedAddon")
                .expect("query")
                .is_some()
        );
    }

    #[tokio::test]
    async fn mismatched_addon_dir_refuses_update_before_any_backup() {
        let sandbox = sandbox();
        let error = run_update(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.forever)),
            Vec::new(),
            false,
            false,
        )
        .await
        .expect_err("update against forever must be refused");
        assert!(error.to_string().contains("is bound to the AddOns folder"));
        assert!(
            !backup_root(&sandbox.paths.data_dir)
                .join("forever")
                .exists()
        );
        assert_eq!(tracked_github(&sandbox.paths).source, SourceKind::GitHub);
    }

    #[test]
    fn mismatched_addon_dir_refuses_restore_and_leaves_wtf_alone() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        create_wtf_backup(&sandbox.retail, &root.join("retail"), 5).expect("retail backup");
        let forever_wtf = sandbox.forever.join("../../WTF/Config.wtf");

        run_restore_backup(sandbox.paths.clone(), runtime(Some(&sandbox.forever)), None)
            .expect_err("restore into forever must be refused");
        assert_eq!(fs::read_to_string(&forever_wtf).expect("wtf"), "_forever_");
    }

    #[test]
    fn restore_without_archive_picks_the_bound_clients_newest_backup() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        create_wtf_backup(&sandbox.retail, &root.join("retail"), 5).expect("retail backup");
        let retail_wtf = sandbox.retail.join("../../WTF/Config.wtf");
        fs::write(&retail_wtf, "changed").expect("mutate");
        // A newer Forever archive in the same store must not be picked for retail.
        create_wtf_backup(&sandbox.forever, &root.join("forever"), 5).expect("forever backup");

        run_restore_backup(sandbox.paths.clone(), runtime(None), None).expect("restore retail");
        assert_eq!(fs::read_to_string(&retail_wtf).expect("wtf"), "_retail_");
    }

    #[test]
    fn explicit_archive_from_another_client_is_refused() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        let forever_backup =
            create_wtf_backup(&sandbox.forever, &root.join("forever"), 5).expect("backup");

        let error = run_restore_backup(
            sandbox.paths.clone(),
            runtime(None),
            Some(forever_backup.backup.path),
        )
        .expect_err("forever archive into retail must be refused");
        assert!(error.to_string().contains("'forever' client"), "{error}");
        let retail_wtf = sandbox.retail.join("../../WTF/Config.wtf");
        assert_eq!(fs::read_to_string(&retail_wtf).expect("wtf"), "_retail_");
    }

    #[test]
    fn legacy_flat_backups_move_to_the_bound_client_and_stay_restorable() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        // Written by an older build straight into the store root.
        let legacy = create_wtf_backup(&sandbox.retail, &root, 5).expect("legacy backup");
        let retail_wtf = sandbox.retail.join("../../WTF/Config.wtf");
        fs::write(&retail_wtf, "changed").expect("mutate");

        run_restore_backup(sandbox.paths.clone(), runtime(None), None).expect("restore");
        assert_eq!(fs::read_to_string(&retail_wtf).expect("wtf"), "_retail_");
        assert!(!legacy.backup.path.exists());
        let moved = list_backups(&root.join("retail")).expect("list");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].file_name, legacy.backup.file_name);
    }

    fn empty_import_file(sandbox: &Sandbox) -> PathBuf {
        let path = sandbox.paths.data_dir.join("import.json");
        lemonup_core::export_addons(&[], &path).expect("write import file");
        path
    }

    #[tokio::test]
    async fn import_dry_run_validates_the_path_first_and_never_binds() {
        let sandbox = sandbox();
        let input = empty_import_file(&sandbox);
        let bound = || {
            StateDatabase::open(&sandbox.paths.state_db_file)
                .expect("open")
                .bound_addon_dir()
                .expect("binding")
        };
        assert_eq!(bound(), None);

        // A path that is not an AddOns folder is refused and must not become the binding.
        let nowhere = sandbox.paths.data_dir.join("nonexistent").join("x");
        let error = run_import_addons(
            sandbox.paths.clone(),
            runtime(Some(&nowhere)),
            Some(input.clone()),
            true,
        )
        .await
        .expect_err("invalid folder must be refused");
        assert!(!error.to_string().contains("bound"), "{error}");
        assert_eq!(bound(), None, "an unvalidated path was bound");

        // A valid folder is accepted read-only: a dry run never adopts a binding.
        run_import_addons(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.retail)),
            Some(input.clone()),
            true,
        )
        .await
        .expect("dry run on a valid folder");
        assert_eq!(bound(), None, "a dry run must not bind");

        // The folder the profile is configured for is still protected during a dry run.
        run_import_addons(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.forever)),
            Some(input),
            true,
        )
        .await
        .expect_err("dry run against a different folder is refused");
        assert_eq!(bound(), None);

        // A following real sync on the valid folder binds it as usual.
        run_sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("sync retail");
        assert_eq!(bound(), Some(sandbox.retail.clone()));
    }

    #[test]
    fn non_default_profile_guard_includes_the_default_profiles_bound_folder() {
        let sandbox = sandbox();
        // Default profile pointed at retail with --addon-dir only: no addon_dir in its config.
        fs::remove_file(&sandbox.paths.config_file).expect("remove config");
        run_sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("default sync");
        let db_before = fs::read(&sandbox.paths.state_db_file).expect("db bytes");

        let guarded = guarded_dirs_of(&sandbox.paths).expect("guard");
        assert_eq!(guarded, vec![sandbox.retail.clone()]);
        // Reading the guard leaves the default profile's database untouched.
        assert_eq!(
            fs::read(&sandbox.paths.state_db_file).expect("db bytes"),
            db_before
        );

        let other_root = sandbox.paths.data_dir.parent().expect("root").join("other");
        let other_paths = AppPaths {
            profile: "other".to_string(),
            config_dir: other_root.join("config"),
            data_dir: other_root.join("data"),
            cache_dir: other_root.join("cache"),
            log_dir: other_root.join("data").join("logs"),
            config_file: other_root.join("config").join("config.toml"),
            state_db_file: other_root.join("data").join("state.sqlite"),
            roots: lemonup_core::ProfileRoots::new(
                other_root.join("config"),
                other_root.join("data"),
                other_root.join("cache"),
            ),
        };
        other_paths.ensure().expect("ensure other");
        let error = run_sync(
            other_paths.clone(),
            AppRuntime::new("other".to_string(), Some(sandbox.retail.clone()), guarded),
        )
        .expect_err("other profile must not take the default profile's folder");
        assert!(
            error
                .to_string()
                .contains("refuses to target the default profile addon directory"),
            "{error}"
        );
        assert_eq!(
            StateDatabase::open(&other_paths.state_db_file)
                .expect("open other")
                .bound_addon_dir()
                .expect("binding"),
            None
        );
    }

    #[test]
    fn guard_reads_a_missing_default_profile_without_creating_it() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("fresh");
        let paths = AppPaths {
            profile: DEFAULT_PROFILE.to_string(),
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            log_dir: root.join("data").join("logs"),
            config_file: root.join("config").join("config.toml"),
            state_db_file: root.join("data").join("state.sqlite"),
            roots: lemonup_core::ProfileRoots::new(
                root.join("config"),
                root.join("data"),
                root.join("cache"),
            ),
        };
        assert_eq!(
            guarded_dirs_of(&paths).expect("guard"),
            Vec::<PathBuf>::new()
        );
        assert!(!root.exists(), "the default profile must not be created");
    }

    #[test]
    fn sync_summary_appends_skipped_and_lists_unreadable_folders() {
        let mut summary = lemonup_core::ScanSummary {
            scanned_addons: 3,
            upserted_addons: 2,
            removed_addons: 1,
            ..Default::default()
        };
        let dir = Path::new("/wow/AddOns");
        assert_eq!(
            format_sync_summary("default", dir, &summary),
            "sync summary: profile=default, addon_dir=/wow/AddOns, scanned_addons=3, upserted_addons=2, removed_addons=1, skipped=0"
        );
        assert_eq!(format_skipped_detail(&summary), None);

        summary.skipped_folders = vec!["Locked".to_string(), "Pawn".to_string()];
        assert!(format_sync_summary("default", dir, &summary).ends_with(", skipped=2"));
        assert_eq!(
            format_skipped_detail(&summary).as_deref(),
            Some("sync detail: skipped (unreadable): Locked, Pawn")
        );
    }

    #[test]
    fn leftover_staging_folders_get_one_warning_each() {
        let summary = lemonup_core::ScanSummary {
            leftover_staging: vec![
                ".lemonup-staging-1-2-0".to_string(),
                ".lemonup-staging-3-4-0".to_string(),
            ],
            ..Default::default()
        };
        let warnings = leftover_staging_warnings(Path::new("/wow/AddOns"), &summary);
        assert_eq!(
            warnings,
            vec![
                "warning: leftover update staging folder /wow/AddOns/.lemonup-staging-1-2-0 may contain previous addon versions; see Troubleshooting".to_string(),
                "warning: leftover update staging folder /wow/AddOns/.lemonup-staging-3-4-0 may contain previous addon versions; see Troubleshooting".to_string(),
            ]
        );
        assert!(leftover_staging_warnings(Path::new("/x"), &Default::default()).is_empty());
    }

    #[test]
    fn sync_with_a_leftover_staging_folder_succeeds_and_does_not_track_it() {
        let sandbox = sandbox();
        let staging = sandbox.retail.join(".lemonup-staging-9-9-0").join("old");
        fs::create_dir_all(staging.join("Old")).expect("staging");
        fs::write(staging.join("Old").join("Old.toc"), "## Title: Old\n").expect("toc");

        run_sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("sync");
        let database = StateDatabase::open(&sandbox.paths.state_db_file).expect("open");
        assert!(
            database
                .get_addon_by_folder("Old")
                .expect("query")
                .is_none()
        );
        let report = lemonup_core::scan_addons_dir_report(
            &sandbox.retail,
            lemonup_core::GameFlavor::Retail,
            &[],
        )
        .expect("scan");
        assert_eq!(report.leftover_staging, vec![".lemonup-staging-9-9-0"]);
    }

    /// An AddOns folder with Foo (linked, from GitHub) and Bar (Not linked), each with a
    /// Category, and the database that knows them.
    fn toc_fixture(root: &std::path::Path) -> (std::path::PathBuf, StateDatabase) {
        let addon_dir = root.join("AddOns");
        for name in ["Foo", "Bar"] {
            std::fs::create_dir_all(addon_dir.join(name)).expect("folder");
            std::fs::write(
                addon_dir.join(name).join(format!("{name}.toc")),
                format!("## Interface: 110005\n## Title: {name}\n## Category: Bags\n"),
            )
            .expect("toc");
        }
        let database = StateDatabase::open(root.join("state.sqlite")).expect("db");
        database
            .upsert_addon(&lemonup_core::AddonRecord::new(
                "Foo",
                "Foo",
                SourceKind::GitHub,
            ))
            .expect("foo");
        database
            .upsert_addon(&lemonup_core::AddonRecord::new(
                "Bar",
                "Bar",
                SourceKind::Manual,
            ))
            .expect("bar");
        (addon_dir, database)
    }

    #[test]
    fn toc_command_shows_sets_and_clears_a_field() {
        use lemonup_core::TocField;
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = toc_fixture(temp.path());

        let shown = toc_command(&mut database, &addon_dir, "foo", None, false).expect("show");
        assert_eq!(
            shown,
            vec![
                "toc field=Title value=Foo source=addon",
                "toc field=Notes value=<none> source=addon",
                "toc field=Category value=Bags source=addon",
            ]
        );

        let saved = toc_command(
            &mut database,
            &addon_dir,
            "Foo",
            Some((TocField::Category, Some(" My Bags ".to_string()))),
            false,
        )
        .expect("set");
        assert_eq!(
            saved[0],
            "toc saved: addon=Foo, field=Category, value=My Bags, folders=Foo"
        );
        let toc = std::fs::read_to_string(addon_dir.join("Foo/Foo.toc")).expect("toc");
        assert!(toc.contains("## Category: My Bags"), "{toc}");
        assert!(toc.contains("# LemonUp: ## Category: Bags"), "{toc}");
        let shown = toc_command(&mut database, &addon_dir, "Foo", None, false).expect("show");
        assert_eq!(shown[2], "toc field=Category value=My Bags source=override");

        // Another program puts the addon's own line back.
        std::fs::write(
            addon_dir.join("Foo/Foo.toc"),
            "## Interface: 110005\n## Title: Foo\n## Category: Bags\n",
        )
        .expect("toc");
        let shown = toc_command(&mut database, &addon_dir, "Foo", None, false).expect("show");
        assert_eq!(
            shown[2],
            "toc field=Category value=Bags source=override-not-applied"
        );

        let cleared = toc_command(
            &mut database,
            &addon_dir,
            "Foo",
            Some((TocField::Category, None)),
            false,
        )
        .expect("clear");
        assert_eq!(
            cleared,
            vec!["toc restored: addon=Foo, field=Category, folders=Foo"]
        );
        let stored = database
            .get_addon_by_folder("Foo")
            .expect("read")
            .expect("row");
        assert!(stored.toc_overrides.is_empty());
    }

    #[test]
    fn only_a_refusal_prints_without_the_error_label() {
        let refusal: Box<dyn std::error::Error> =
            Box::new(LemonupError::Refused("Nothing was linked.".to_string()));
        let failure: Box<dyn std::error::Error> =
            Box::new(LemonupError::InvalidArgument("bad".to_string()));

        assert_eq!(error_text(refusal.as_ref()), "Nothing was linked.");
        assert_eq!(error_text(failure.as_ref()), "Error: invalid argument: bad");
    }

    #[test]
    fn toc_command_needs_yes_for_a_not_linked_addon_and_a_known_addon() {
        use lemonup_core::TocField;
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = toc_fixture(temp.path());
        let change = || Some((TocField::Notes, Some("Mine".to_string())));

        let refusal = toc_command(&mut database, &addon_dir, "Bar", change(), false)
            .expect_err("needs --yes");
        assert!(matches!(refusal, LemonupError::Refused(_)), "{refusal:?}");
        // The run ends with the refusal as it is, not an "Error: invalid argument" label.
        let printed = error_text(&refusal);
        assert!(!printed.contains("invalid argument"), "{printed}");
        assert!(!printed.starts_with("Error"), "{printed}");
        assert!(printed.ends_with("Nothing was changed."), "{printed}");
        let error = refusal.to_string();
        assert!(error.contains("Bar is Not linked"), "{error}");
        assert!(error.contains("--yes"), "{error}");
        let toc = std::fs::read_to_string(addon_dir.join("Bar/Bar.toc")).expect("toc");
        assert!(!toc.contains("LemonUp"), "{toc}");
        assert!(
            database
                .get_addon_by_folder("Bar")
                .expect("read")
                .expect("row")
                .toc_overrides
                .is_empty()
        );

        toc_command(&mut database, &addon_dir, "Bar", change(), true).expect("with --yes");
        let toc = std::fs::read_to_string(addon_dir.join("Bar/Bar.toc")).expect("toc");
        assert!(toc.contains("## Notes: Mine"), "{toc}");

        // An unknown addon is named, and nothing says "linked".
        let error = toc_command(&mut database, &addon_dir, "Nope", None, false)
            .expect_err("unknown addon")
            .to_string();
        assert!(error.contains("No addon matches Nope"), "{error}");
        assert!(!error.contains("linked"), "{error}");

        // A value the game can't read is refused before anything changes.
        let error = toc_command(
            &mut database,
            &addon_dir,
            "Foo",
            Some((TocField::Title, Some("two\nlines".to_string()))),
            false,
        )
        .expect_err("refused");
        assert!(error.to_string().contains("line breaks"), "{error}");
    }
}

#[cfg(test)]
mod profile_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use lemonup_core::{
        AddonRecord, AppConfig, ConfigLoad, ConfigStore, DEFAULT_PROFILE, ProfileRoots,
        SharedSettings, SharedSettingsStore, SourceKind, StateDatabase, ThemeMode, create_profile,
        list_profiles, remember_last_used,
    };
    use tempfile::tempdir;

    use super::{command_profile, load_profile_config, profile_lines};
    use crate::cli::Commands;

    fn test_roots(base: &Path) -> ProfileRoots {
        ProfileRoots::new(base.join("config"), base.join("data"), base.join("cache"))
    }

    fn install(base: &Path, client: &str) -> PathBuf {
        let root = base.join("wow");
        let dir = root.join(client).join("Interface").join("AddOns");
        fs::create_dir_all(&dir).expect("addons dir");
        fs::create_dir_all(root.join("Data")).expect("data dir");
        fs::write(root.join(".build.info"), "build").expect("build info");
        dir
    }

    #[test]
    fn commands_read_the_settings_all_games_share() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        let mut own = AppConfig::new_unconfigured();
        own.check_interval_secs = 60;
        ConfigStore::new(paths.config_file.clone())
            .write_config(&own)
            .expect("write own");
        let shared = SharedSettings {
            check_interval_secs: 1800,
            theme: ThemeMode::Light,
            ..SharedSettings::default()
        };
        SharedSettingsStore::for_roots(&roots)
            .write(&shared)
            .expect("write shared");

        let (_, state) = load_profile_config(&paths).expect("load");
        let ConfigLoad::Loaded(config) = state else {
            panic!("config exists");
        };
        assert_eq!(config.check_interval_secs, 1800);
        assert_eq!(config.theme, ThemeMode::Light);
    }

    #[test]
    fn only_the_tui_opens_the_last_used_game() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        create_profile(&roots, &install(temp.path(), "_forever_"), None).expect("create");
        remember_last_used(&roots, "forever").expect("remember");

        let profile = |requested: Option<&str>, command: Commands| {
            command_profile(&roots, requested.map(str::to_string), &command).expect("profile")
        };
        assert_eq!(profile(None, Commands::Tui), "forever");
        assert_eq!(profile(None, Commands::Sync), DEFAULT_PROFILE);
        assert_eq!(profile(None, Commands::Profiles), DEFAULT_PROFILE);
        assert_eq!(
            profile(Some(DEFAULT_PROFILE), Commands::Tui),
            DEFAULT_PROFILE
        );
        assert_eq!(profile(Some("forever"), Commands::Sync), "forever");
    }

    #[test]
    fn profiles_command_lists_each_game() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let retail = install(temp.path(), "_retail_");
        let forever = install(temp.path(), "_forever_");
        let default = roots.paths(DEFAULT_PROFILE).expect("paths");
        let config = AppConfig {
            addon_dir: Some(retail.clone()),
            ..AppConfig::default()
        };
        ConfigStore::for_profile(&default)
            .write_config(&config)
            .expect("write default");
        let database = StateDatabase::open(&default.state_db_file).expect("open");
        database.bind_addon_dir(&retail).expect("bind");
        for name in ["One", "Two"] {
            database
                .upsert_addon(&AddonRecord::new(name, name, SourceKind::Manual))
                .expect("upsert");
        }
        create_profile(&roots, &forever, None).expect("create forever");
        let broken = roots.paths("broken").expect("paths");
        fs::create_dir_all(&broken.config_dir).expect("dir");
        fs::write(&broken.config_file, "addon_dir = [").expect("broken");

        let lines = profile_lines(&list_profiles(&roots).expect("list"));
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert_eq!(lines[0], "profiles summary: count=3");
        assert_eq!(
            lines[1],
            format!(
                "profile: name=default, game=Retail, game_version=retail, addon_dir={}, addons=2",
                retail.display()
            )
        );
        assert!(
            lines[2].starts_with(
                "profile: name=broken, game=broken, game_version=<unknown>, addon_dir=<unconfigured>, addons=0, problem=couldn't read its settings:"
            ),
            "{}",
            lines[2]
        );
        assert_eq!(
            lines[3],
            format!(
                "profile: name=forever, game=WoW Forever, game_version=forever, addon_dir={}, addons=0",
                forever.display()
            )
        );
    }
}
