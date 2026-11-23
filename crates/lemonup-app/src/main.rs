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
mod lemonup_release;
mod onboarding;
mod output;
mod shimmer;
mod source_error;
mod source_limits;
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
    AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE, GameFlavor, LemonupError, ProfileRoots,
    StateDatabase, TuiProfile, UpdateStatus, V1Location, V1MigrationOutcome, analyze_import,
    default_transfer_path, export_addons, list_profiles, migrate_v1_if_needed, parse_import_file,
    remember_last_used, scan_addons_dir_report, tui_profile, validate_addons_path,
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
use crate::update::{LiveUpdateSummary, apply_live_updates, refresh_live_update_checks};
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
    /// The command failed or refused and said so in its own last line.
    Failed,
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
            Self::Failures(_) | Self::Skipped | Self::Failed => 1,
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
            let paint = output::Paint::for_stderr(
                &lemonup_core::AppConfig::default(),
                theme::DisplayEnv::detect(),
            );
            eprintln!("{}", error_text(error.as_ref(), paint));
            ExitCode::FAILURE
        }
    }
}

/// What an error that ends a command prints. A refusal already says what happened and what to
/// do next, so it has no label; anything else is labeled as an error, in `paint`. Both can
/// name an addon, a folder or a source's own words, so control characters are removed.
fn error_text(error: &(dyn std::error::Error + 'static), paint: output::Paint) -> String {
    match error.downcast_ref::<LemonupError>() {
        Some(LemonupError::Refused(message)) => paint.commands(message, output::Paint::text),
        _ => format!(
            "{} {}",
            output::error_label(paint),
            paint.commands(&error.to_string(), output::Paint::text)
        ),
    }
}

async fn run() -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    // Before anything reads or writes a folder: a refused run changes nothing.
    if cli.json
        && let Some(refusal) = cli::json_refusal(cli.command.as_ref())
    {
        return Err(Box::new(LemonupError::Refused(refusal)));
    }
    let command = cli.command.unwrap_or(Commands::Tui);
    let roots = ProfileRoots::discover()?;
    let TuiProfile {
        name: profile,
        notice: profile_notice,
    } = command_profile(&roots, cli.profile, &command)?;
    let runtime = AppRuntime::new(
        profile.clone(),
        cli.addon_dir.clone(),
        load_guarded_addon_dirs(&profile)?,
    )
    .with_display(theme::DisplayEnv::detect());
    let paths = roots.paths(&profile)?;
    paths.ensure()?;
    let json = cli.json;

    configure_tracing(log_destination(&command, &paths));

    if let Some(notice) = &profile_notice {
        tracing::warn!(notice, "the last used game didn't open");
    }
    let startup_notice = run_v1_migration(&paths, V1Location::discover()).or_else(|| {
        profile_notice.map(|text| StartupNotice {
            text,
            is_error: true,
            details: Vec::new(),
        })
    });
    if let Some(notice) = startup_notice.as_ref()
        && !matches!(command, Commands::Tui)
    {
        // Only the colors matter here, and they don't depend on the settings.
        notice.print(output::Paint::for_stderr(
            &lemonup_core::AppConfig::default(),
            runtime.display,
        ));
    }

    let outcome = match command {
        Commands::Tui => {
            run_tui(paths, runtime, startup_notice).await?;
            CommandOutcome::Success
        }
        Commands::Sync => {
            let mut output = command_output(&paths, &runtime, json);
            run_sync(paths, runtime, &mut output)?
        }
        Commands::Profiles => {
            let mut output = command_output(&paths, &runtime, json);
            output.print(
                &output::ProfilesReport::new(&list_profiles(&paths.roots)?),
                None,
            )?;
            CommandOutcome::Success
        }
        Commands::Check { addons } => {
            let mut output = command_output(&paths, &runtime, json);
            run_check(paths, runtime, addons, &mut output).await?
        }
        Commands::Update {
            addons,
            force,
            dry_run,
            replace,
        } => {
            let mut output = command_output(&paths, &runtime, json);
            run_update(paths, runtime, addons, force, dry_run, replace, &mut output).await?
        }
        Commands::UpdateAll {
            force,
            dry_run,
            replace,
        } => {
            let mut output = command_output(&paths, &runtime, json);
            run_update(
                paths,
                runtime,
                Vec::new(),
                force,
                dry_run,
                replace,
                &mut output,
            )
            .await?
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
        Commands::ExportAddons { output: file } => {
            let mut output = command_output(&paths, &runtime, json);
            run_export_addons(paths, file, &mut output)?;
            CommandOutcome::Success
        }
        Commands::ImportAddons { input, dry_run } => {
            let mut output = command_output(&paths, &runtime, json);
            run_import_addons(paths, runtime, input, dry_run, &mut output).await?
        }
        Commands::RestoreBackup { archive } => {
            let mut output = command_output(&paths, &runtime, json);
            run_restore_backup(paths, runtime, archive, &mut output)?;
            CommandOutcome::Success
        }
        Commands::Toc {
            addon,
            field,
            value,
            clear,
            yes,
        } => {
            let change = toc_change(field, value, clear)?;
            let mut output = command_output(&paths, &runtime, json);
            run_toc(paths, runtime, &addon, change, yes, &mut output)?
        }
        Commands::Find {
            link,
            yes,
            plain,
            source,
        } => run_find(paths, runtime, link, yes, plain, &source).await?,
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
) -> lemonup_core::Result<TuiProfile> {
    let plain = |name: String| TuiProfile { name, notice: None };
    match requested {
        Some(profile) => Ok(plain(profile)),
        None if matches!(command, Commands::Tui) => tui_profile(roots, None),
        None => Ok(plain(DEFAULT_PROFILE.to_string())),
    }
}

/// Where a command that prints through `output` prints, for the run's profile. Built per
/// command while only some print through it, so the others start as before.
fn command_output(paths: &AppPaths, runtime: &AppRuntime, json: bool) -> output::Output {
    output::Output::stdio(
        output::OutputMode::from_json_flag(json),
        &output_config(paths),
        runtime.display,
        &runtime.profile_name,
    )
}

/// The game a report is about: the game of `addon_dir` as `profiles` names it.
fn report_target(
    paths: &AppPaths,
    runtime: &AppRuntime,
    addon_dir: &Path,
) -> crate::find::FindTarget {
    crate::find::FindTarget::new(
        &runtime.profile_name,
        addon_dir,
        GameFlavor::for_addons_dir(addon_dir),
        &list_profiles(&paths.roots).unwrap_or_default(),
    )
}

/// The refusal of a command that needs the game's AddOns folder when none is set.
fn no_addon_dir_refusal(profile: &str) -> LemonupError {
    let command = if profile == DEFAULT_PROFILE {
        "lemonup".to_string()
    } else {
        tui_command_hint(profile)
    };
    LemonupError::Refused(format!(
        "No AddOns folder is set for this game. Run `{command}` to set it up."
    ))
}

/// The settings CLI output is drawn with (only its symbols): the profile's, read without
/// writing anything. Settings that can't be read give the defaults; the command itself reports
/// them when it reads them.
fn output_config(paths: &AppPaths) -> lemonup_core::AppConfig {
    let store = ConfigStore::for_profile(paths);
    match store.load_without_writing() {
        Ok(ConfigLoad::Loaded(config)) => config,
        _ => {
            let mut config = lemonup_core::AppConfig::new_unconfigured();
            let _ = store.refresh_shared(&mut config);
            config
        }
    }
}

/// A field and the change `lemonup toc` makes to it: a new value, or `None` to restore the
/// addon's own.
type TocChangeArg = (lemonup_core::TocField, Option<String>);

/// The change `lemonup toc` was asked for, if any: a field needs a new value or `--clear`.
fn toc_change(
    field: Option<lemonup_core::TocField>,
    value: Option<String>,
    clear: bool,
) -> Result<Option<TocChangeArg>, LemonupError> {
    match (field, value, clear) {
        (None, _, _) => Ok(None),
        (Some(field), Some(value), false) => Ok(Some((field, Some(value)))),
        (Some(field), None, true) => Ok(Some((field, None))),
        (Some(field), _, _) => Err(LemonupError::InvalidArgument(format!(
            "give a new {} value, or `--clear` to restore the addon's own",
            field.name()
        ))),
    }
}

/// `lemonup toc`: prints the addon's Title, Notes and Category, or changes one. The CLI never
/// asks, so changing a "Not linked" addon needs `--yes`.
fn run_toc<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    change: Option<TocChangeArg>,
    yes: bool,
    output: &mut output::Output<O, E>,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
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
        return Err(Box::new(no_addon_dir_refusal(&runtime.profile_name)));
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
    let report = toc_command(&mut database, &addon_dir, addon, change, yes)?;
    for warning in report.warnings() {
        output.warn(&warning)?;
    }
    output.print(&report, Some(&addon_dir))?;
    Ok(CommandOutcome::Success)
}

/// `lemonup find`: lists where the Not linked addons come from, grouped by label, and changes
/// nothing. `--link` links the exact matches (no download); the CLI never asks, so matches that
/// take over folders beyond their own need `--yes`. `--plain` prints one key=value line per
/// result instead of the readable report. `--source` limits the sources asked (all without it;
/// the choice the TUI remembers is not used). Exits 1 when a source failed.
async fn run_find(
    paths: AppPaths,
    runtime: AppRuntime,
    link: bool,
    yes: bool,
    plain: bool,
    sources: &[lemonup_core::find::FindSource],
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (config_store, config_state) = load_profile_config(&paths)?;
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
    let target = crate::find::FindTarget::new(
        &runtime.profile_name,
        &addon_dir,
        flavor,
        &list_profiles(&paths.roots).unwrap_or_default(),
    );
    if link {
        crate::find::scan_before_linking(&mut database, &addon_dir, flavor)?;
    }
    let rows = database.list_addons()?;
    let inputs = lemonup_core::find::FindInputs::read(&addon_dir, flavor)?;
    let choice = crate::find::cli_choice(sources);
    let wago_lookups = inputs.wago_lookups(&rows, None);
    let api_key = resolve_wago_api_key(&config_store, &config_state)?;
    let sends_to_wago = choice.wants(lemonup_core::find::FindSource::Wago)
        && api_key.is_some()
        && !wago_lookups.is_empty();
    if plain {
        println!("{}", crate::find::target_plain_line(&target));
    }
    if plain && sends_to_wago {
        println!(
            "find note: sending {} folder names and file hashes to Wago.io",
            wago_lookups.len()
        );
    }
    let names = crate::find::LocalNames::new(&rows, &inputs);
    let lookups = crate::find::lookup_sources(
        &wago_lookups,
        flavor,
        api_key.as_deref(),
        &paths.cache_dir,
        choice,
    )
    .await;
    let report = lemonup_core::find::match_addons(&inputs, &rows, None, &lookups.lookups);
    let curseforge_files = lemonup_core::curseforge::record_files();
    let mut link_now = || {
        crate::find::link_exact_matches(
            &mut database,
            &addon_dir,
            flavor,
            &report,
            yes,
            &curseforge_files,
        )
    };
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
        let theme_config = config_store.effective(&config_state)?;
        let paint = crate::output::Paint::for_stdout(&theme_config, runtime.display);
        println!("{}", crate::find::target_line(&target, paint));
        for line in
            crate::find::human_report(&report, &lookups.problems, &names, wago_folders, paint)
        {
            println!("{line}");
        }
        println!();
        let outcome = if link { Some(link_now()?) } else { None };
        // A refusal leaves the report on screen and ends the run with what happened, bold
        // like the closing line it takes the place of.
        match crate::find::human_closing(&report, &names, outcome, paint) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(LemonupError::Refused(message)) => {
                let error_paint = crate::output::Paint::for_stderr(&theme_config, runtime.display);
                eprintln!("{}", crate::find::human_refusal(&message, error_paint));
                return Ok(CommandOutcome::Failed);
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
    Ok(CommandOutcome::from_error_count(lookups.problems.len()))
}

/// The fields of the addon in `folder` as its .toc files have them, and where each comes from
/// given the stored `overrides`.
fn toc_rows(
    addon_dir: &Path,
    folder: &str,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<output::TocRow>, LemonupError> {
    let flavor = GameFlavor::for_addons_dir(addon_dir);
    let effective =
        lemonup_core::read_addon_toc_fields(addon_dir, folder, flavor)?.unwrap_or_default();
    Ok(lemonup_core::TocField::ALL
        .into_iter()
        .map(|field| {
            let value = effective.get(field);
            output::TocRow {
                field: field.name(),
                value: value.map(str::to_string),
                source: lemonup_core::toc_edit::value_source(field, overrides, value),
            }
        })
        .collect())
}

/// What `lemonup toc` prints, after changing a field when `change` asks for it (`None` as the
/// value restores the addon's own). A change rewrites the addon's .toc files and reads the
/// AddOns folder again, so the stored name follows a new Title.
fn toc_command(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selector: &str,
    change: Option<TocChangeArg>,
    yes: bool,
) -> Result<output::TocReport, LemonupError> {
    use lemonup_core::toc_edit;

    let installed = database.list_addons()?;
    // A name more than one addon has is refused rather than guessed.
    let record = crate::update::resolve_one_addon(&installed, selector)?
        .cloned()
        .ok_or_else(|| {
            LemonupError::InvalidArgument(format!(
                "No addon matches {selector}. Use the exact addon name or folder name; \
                 `lemonup sync` reads your AddOns folder first."
            ))
        })?;
    let flavor = GameFlavor::for_addons_dir(addon_dir);

    let Some((field, value)) = change else {
        let rows = toc_rows(addon_dir, &record.folder, &record.toc_overrides)?;
        return Ok(output::TocReport::new(
            record.display_name(),
            &record.folder,
            rows,
            None,
        ));
    };

    let mut overrides = record.toc_overrides.clone();
    toc_edit::set_override(&mut overrides, field, value.as_deref())?;
    if record.source == lemonup_core::SourceKind::Manual && !yes {
        return Err(LemonupError::Refused(format!(
            "{} is Not linked: LemonUp didn't install it, so changing its .toc files needs \
             `--yes`. Its own lines are kept, and `--clear` puts them back. Nothing was changed.",
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
    // The change is saved by now. If the files can't be read again, the report has no fields
    // rather than failing a run that did what it was asked.
    let rows = toc_rows(addon_dir, &record.folder, &overrides).map_err(|error| {
        tracing::warn!(%error, "could not read the .toc fields after a change");
    });
    let unread = rows.is_err();
    let changed = output::TocChange {
        field: field.name(),
        value: value.and_then(|_| overrides.get(field.name()).cloned()),
        folders,
    };
    let report = output::TocReport::new(
        record.display_name(),
        &record.folder,
        rows.unwrap_or_default(),
        Some(changed),
    );
    Ok(if unread {
        report.with_fields_unread()
    } else {
        report
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
    /// CLI output goes to stderr so it never mixes with the result on stdout.
    fn print(&self, paint: output::Paint) {
        for line in self.lines(paint) {
            eprintln!("{line}");
        }
    }

    /// The lines a CLI command prints, with the commands they name drawn as commands.
    fn lines(&self, paint: output::Paint) -> Vec<String> {
        std::iter::once(self.text.clone())
            .chain(self.details.iter().map(|line| format!("  {line}")))
            .map(|line| paint.commands(&line, output::Paint::text))
            .collect()
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

/// How the check report names the command that installs the updates. `update` refuses to run
/// with no AddOns folder, on a folder guarded for the default profile, on a folder that isn't
/// a valid AddOns folder, and on one the profile isn't bound to (`mismatch`): no command is
/// named then. A run that was given `--addon-dir` needs it again.
fn check_update_hint(
    runtime: &AppRuntime,
    addon_dir: Option<&Path>,
    mismatch: bool,
) -> output::UpdateHint {
    let refused = match addon_dir {
        None => true,
        Some(dir) => mismatch || runtime.is_guarded_path(dir) || validate_addons_path(dir).is_err(),
    };
    if refused {
        output::UpdateHint::None
    } else if runtime.addon_dir_override.is_some() {
        output::UpdateHint::WithAddonDir
    } else {
        output::UpdateHint::Plain
    }
}

async fn run_check<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    runtime: AppRuntime,
    addons: Vec<String>,
    output: &mut output::Output<O, E>,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let (config_store, config_state) = load_profile_config(&paths)?;
    let mut database = StateDatabase::open(&paths.state_db_file)?;
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
        output.warn(&format!(
            "{mismatch}. Running read-only, results are not saved."
        ))?;
        let dir = tempfile::tempdir()?;
        let copy = dir.path().join("state.sqlite");
        database.copy_to(&copy)?;
        database = StateDatabase::open(&copy)?;
        scratch = Some(dir);
    }
    let hint = check_update_hint(&runtime, effective_addon_dir.as_deref(), scratch.is_some());
    // Names, sources and installed versions for the report. Read before the check, so a read
    // that fails stops the run before anything is saved.
    let rows_before = database.list_addons()?;
    let checks = refresh_live_update_checks(
        &mut database,
        &addons,
        api_key.as_deref(),
        effective_addon_dir.as_deref(),
        |_| {},
    )
    .await
    .map_err(Box::<dyn std::error::Error>::from)?;
    // A check can save an addon's name at its source, so the rows are read again. The check
    // is saved by now: if this read fails, the report still prints from the earlier rows.
    let rows = database.list_addons().unwrap_or(rows_before);
    drop(database);
    drop(scratch);

    let target = effective_addon_dir
        .as_deref()
        .map(|addon_dir| report_target(&paths, &runtime, addon_dir));
    let report = output::CheckReport::new(
        target,
        &runtime.profile_name,
        !addons.is_empty(),
        &checks,
        &rows,
    )
    .with_hint(hint);
    output.print(&report, effective_addon_dir.as_deref())?;
    Ok(CommandOutcome::from_error_count(count_check_errors(
        &checks,
    )))
}

async fn run_update<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    runtime: AppRuntime,
    addons: Vec<String>,
    force: bool,
    dry_run: bool,
    replace: bool,
    output: &mut output::Output<O, E>,
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

    // A missing config without `--addon-dir` has no folder either.
    let Some(addon_dir) = effective_addon_dir else {
        return Err(Box::new(no_addon_dir_refusal(&runtime.profile_name)));
    };
    let api_key = resolve_wago_api_key(&config_store, &config_state)?;
    let backup = if dry_run {
        None
    } else {
        let backup_settings =
            pre_update_backup_settings(&config_state, &paths.data_dir, &addon_dir);
        Some(run_pre_update_backup(
            &backup_settings,
            &addon_dir,
            time::OffsetDateTime::now_utc(),
        ))
    };
    let backup = output::BackupLine::of(backup.as_ref());
    if let Some(warning) = backup.warning() {
        output.warn(&warning)?;
    }
    let run = match apply_live_updates(
        &mut database,
        &addon_dir,
        &addons,
        api_key.as_deref(),
        force,
        dry_run,
        replace,
        |_| {},
    )
    .await
    {
        Ok(run) => run,
        Err(error) => {
            // No report will print, so say here that a backup was made before the failure.
            if let (output::BackupLine::Created { .. }, Some(line)) =
                (&backup, backup.text(output::Paint::PLAIN))
            {
                output.note(&line)?;
            }
            return Err(error.into());
        }
    };
    // Held packages are removed when `run` drops; the CLI never asks.
    let report = output::UpdateReport::new(
        Some(report_target(&paths, &runtime, &addon_dir)),
        output::UpdateFlags {
            named: !addons.is_empty(),
            force,
            dry_run,
        },
        backup,
        &run.summary,
        &run.results,
        &installed,
    );
    output.print(&report, Some(&addon_dir))?;
    Ok(update_outcome(&run.summary))
}

/// A run that held addons for a yes did not finish them, so it counts them with the errors.
fn update_outcome(summary: &LiveUpdateSummary) -> CommandOutcome {
    CommandOutcome::from_error_count(summary.errors + summary.held)
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

fn is_refusal(error: &(dyn std::error::Error + 'static)) -> bool {
    matches!(
        error.downcast_ref::<LemonupError>(),
        Some(LemonupError::Refused(_))
    )
}

/// Why a command failed, without the name of the kind of error in front.
fn failure_reason(error: &(dyn std::error::Error + 'static)) -> String {
    match error.downcast_ref::<LemonupError>() {
        Some(LemonupError::InvalidArgument(reason)) => reason.clone(),
        _ => error.to_string(),
    }
}

/// `sync`, with a failure said the way its success is: a mark, then what happened. A refusal
/// already says what to do next, so it stays as it is. Only the sync itself is called a
/// failed sync: a report that can't be written is an error of its own, because by then
/// LemonUp's list is saved.
fn run_sync<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    runtime: AppRuntime,
    output: &mut output::Output<O, E>,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    let report = match sync_report(&paths, &runtime) {
        Ok(report) => report,
        Err(error) if is_refusal(error.as_ref()) => return Err(error),
        Err(error) => {
            output.fail(&format!("Sync failed - {}", failure_reason(error.as_ref())))?;
            return Ok(CommandOutcome::Failed);
        }
    };
    for warning in report.warnings() {
        output.warn(&warning)?;
    }
    output.print(&report, Some(report.addon_dir()))?;
    Ok(CommandOutcome::Success)
}

/// Reads the AddOns folder into LemonUp's list and says what it found. Prints nothing.
fn sync_report(
    paths: &AppPaths,
    runtime: &AppRuntime,
) -> Result<output::SyncReport, Box<dyn std::error::Error>> {
    let (_, config_state) = load_profile_config(paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    // A missing config without `--addon-dir` has no folder either.
    let Some(addon_dir) = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone())
    else {
        return Err(Box::new(no_addon_dir_refusal(&runtime.profile_name)));
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
        paths,
        runtime,
        configured_addon_dir.as_deref(),
        &addon_dir,
        true,
    )?;
    let tracked = database.list_addons()?;
    let report =
        scan_addons_dir_report(&addon_dir, GameFlavor::for_addons_dir(&addon_dir), &tracked)?;
    let summary = database.reconcile_scan(&report)?;

    Ok(output::SyncReport::new(
        report_target(paths, runtime, &addon_dir),
        &summary,
    ))
}

fn run_export_addons<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    file: Option<PathBuf>,
    output: &mut output::Output<O, E>,
) -> Result<(), Box<dyn std::error::Error>> {
    let database = StateDatabase::open(paths.state_db_file)?;
    let addons = database.list_addons()?;
    let output_path = file.unwrap_or(default_transfer_path()?);
    let export = export_addons(&addons, &output_path)?;
    let file = std::path::absolute(&output_path).unwrap_or(output_path);
    let report = output::ExportReport::new(file, &export.addons);
    for warning in report.warnings() {
        output.warn(&warning)?;
    }
    output.print(&report, None)?;
    Ok(())
}

async fn run_import_addons<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    runtime: AppRuntime,
    input: Option<PathBuf>,
    dry_run: bool,
    output: &mut output::Output<O, E>,
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
    // A dry run only reads, so it reports without a folder too; installing needs one. Refused
    // before the list is opened, so a refused run changes nothing.
    if !dry_run && effective_addon_dir.is_none() {
        return Err(Box::new(no_addon_dir_refusal(&runtime.profile_name)));
    }

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
    for warning in analysis
        .unsupported_addons
        .iter()
        .filter_map(lemonup_core::TransferAddon::unknown_source_warning)
    {
        output.warn(&warning)?;
    }
    let file = std::path::absolute(&input_path).unwrap_or(input_path);
    let target = effective_addon_dir
        .as_deref()
        .map(|addon_dir| report_target(&paths, &runtime, addon_dir));

    let outcomes = match effective_addon_dir.as_deref() {
        Some(addon_dir) if !dry_run => {
            let wago_api_key = resolve_wago_api_key(&config_store, &config_state)?;
            let summary = install_transfer_addons(
                &mut database,
                addon_dir,
                &analysis,
                wago_api_key.as_deref(),
            )
            .await;
            for warning in &summary.warnings {
                output.warn(warning)?;
            }
            summary.outcomes
        }
        _ => Vec::new(),
    };
    let report = output::ImportReport::new(target, file, dry_run, &analysis, &outcomes);
    output.print(&report, effective_addon_dir.as_deref())?;
    let error_count = outcomes
        .iter()
        .filter(|outcome| outcome.result.is_err())
        .count();
    Ok(CommandOutcome::from_error_count(error_count))
}

fn run_restore_backup<O: std::io::Write, E: std::io::Write>(
    paths: AppPaths,
    runtime: AppRuntime,
    archive: Option<PathBuf>,
    output: &mut output::Output<O, E>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, config_state) = load_profile_config(&paths)?;
    let configured_addon_dir = match &config_state {
        ConfigLoad::Loaded(config) => config.addon_dir.clone(),
        ConfigLoad::Missing(_) => None,
    };
    let Some(effective_addon_dir) = runtime
        .addon_dir_override
        .clone()
        .or(configured_addon_dir.clone())
    else {
        return Err(Box::new(no_addon_dir_refusal(&runtime.profile_name)));
    };

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
                Box::new(LemonupError::Refused(
                    "There is no backup of your game settings to restore.".to_string(),
                )) as Box<dyn std::error::Error>
            })?,
    };

    restore_wtf_backup(&effective_addon_dir, &backup_dir, &backup_path)
        .map_err(Box::<dyn std::error::Error>::from)?;
    let archive = std::path::absolute(&backup_path).unwrap_or(backup_path);
    output.print(
        &output::RestoreReport::new(archive),
        Some(&effective_addon_dir),
    )?;
    Ok(())
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
        "github install: profile={}, repo_url={}, repo_name={}, parent={}, folders={}, version={}, commit={}, release={}, dry_run={}",
        runtime.profile_name,
        summary.repo_url,
        summary.repo_name,
        summary.parent_folder,
        summary.installed_folders.join("|"),
        summary.version.as_deref().unwrap_or("<unknown>"),
        summary.git_commit.as_deref().unwrap_or("<none>"),
        summary.release_tag.as_deref().unwrap_or("<none>"),
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
        CommandOutcome, LogDestination, count_check_errors, default_log_filter, log_destination,
        log_writer, open_log_file, pre_update_backup_settings, refresh_managed_update_state,
        tui_command_hint, update_outcome,
    };
    use crate::update::{
        CheckResult, LiveUpdateSummary, build_managed_update_record, build_update_checks,
        determine_update_status,
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
                replace: false,
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
    fn an_update_run_with_held_addons_exits_one() {
        let clean = LiveUpdateSummary {
            updated_addons: 2,
            ..LiveUpdateSummary::default()
        };
        assert_eq!(update_outcome(&clean).exit_status(), 0);
        let held = LiveUpdateSummary {
            updated_addons: 1,
            held: 1,
            ..LiveUpdateSummary::default()
        };
        assert_eq!(update_outcome(&held), CommandOutcome::Failures(1));
        assert_eq!(update_outcome(&held).exit_status(), 1);
        let both = LiveUpdateSummary {
            held: 1,
            errors: 2,
            ..LiveUpdateSummary::default()
        };
        assert_eq!(update_outcome(&both), CommandOutcome::Failures(3));
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
        CommandOutcome, check_update_hint, error_text, guarded_dirs_of, run_check,
        run_export_addons, run_import_addons, run_restore_backup, run_sync, run_toc, run_update,
        toc_command,
    };
    use crate::app::AppRuntime;
    use crate::backup::{backup_root, create_wtf_backup, list_backups};
    use crate::output::{Output, OutputMode, Paint, Report};

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

    /// The work of `lemonup sync`, with the error it ends in. `run_sync` says a failure in
    /// its own last line.
    fn sync(
        paths: AppPaths,
        runtime: AppRuntime,
    ) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
        super::sync_report(&paths, &runtime).map(|_| CommandOutcome::Success)
    }

    /// `lemonup restore-backup`, printing into a buffer nobody reads.
    fn restore(
        paths: AppPaths,
        runtime: AppRuntime,
        archive: Option<PathBuf>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        run_restore_backup(paths, runtime, archive, &mut capture(OutputMode::Human))
    }

    fn tracked_github(paths: &AppPaths) -> AddonRecord {
        StateDatabase::open(&paths.state_db_file)
            .expect("open db")
            .get_addon_by_folder("GitAddon")
            .expect("query")
            .expect("GitAddon still tracked")
    }

    #[test]
    fn the_check_names_no_update_command_that_would_be_refused() {
        use crate::output::UpdateHint;
        let sandbox = sandbox();
        // The profile's own folder: plain. Given again with --addon-dir: the update needs it too.
        assert_eq!(
            check_update_hint(&runtime(None), Some(&sandbox.retail), false),
            UpdateHint::Plain
        );
        assert_eq!(
            check_update_hint(
                &runtime(Some(&sandbox.retail)),
                Some(&sandbox.retail),
                false
            ),
            UpdateHint::WithAddonDir
        );
        // No folder, a folder the profile isn't bound to, and a folder that isn't an AddOns
        // folder are all refused by `update`.
        assert_eq!(
            check_update_hint(&runtime(None), None, false),
            UpdateHint::None
        );
        assert_eq!(
            check_update_hint(
                &runtime(Some(&sandbox.forever)),
                Some(&sandbox.forever),
                true
            ),
            UpdateHint::None
        );
        let not_addons = sandbox.retail.join("NoSuchFolder");
        assert_eq!(
            check_update_hint(&runtime(Some(&not_addons)), Some(&not_addons), false),
            UpdateHint::None
        );
        // A folder guarded for the default profile, seen from another profile.
        let other = AppRuntime::new(
            "forever".to_string(),
            Some(sandbox.retail.clone()),
            vec![sandbox.retail.clone()],
        );
        assert_eq!(
            check_update_hint(&other, Some(&sandbox.retail), false),
            UpdateHint::None
        );
    }

    #[test]
    fn mismatched_addon_dir_refuses_sync_and_keeps_tracking() {
        let sandbox = sandbox();

        let error = sync(sandbox.paths.clone(), runtime(Some(&sandbox.forever)))
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
        sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("sync retail");
        sync(sandbox.paths.clone(), runtime(None)).expect("sync configured retail");
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
        let mut output = capture(OutputMode::Human);
        let error = run_update(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.forever)),
            Vec::new(),
            false,
            false,
            false,
            &mut output,
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
        let (stdout, _) = output.into_writers();
        assert!(stdout.is_empty());
    }

    type Captured = Output<Vec<u8>, Vec<u8>>;

    fn capture(mode: OutputMode) -> Captured {
        Output::new(
            mode,
            Paint::PLAIN,
            Paint::PLAIN,
            DEFAULT_PROFILE,
            Vec::new(),
            Vec::new(),
        )
    }

    fn streams(output: Captured) -> (String, String) {
        let (stdout, stderr) = output.into_writers();
        (
            String::from_utf8(stdout).expect("utf-8"),
            String::from_utf8(stderr).expect("utf-8"),
        )
    }

    /// The sandbox's GitHub addon as one that is not linked, so a run asks no source.
    fn unlink_tracked(paths: &AppPaths) {
        let mut addon = tracked_github(paths);
        addon.source = SourceKind::Manual;
        addon.source_url = None;
        StateDatabase::open(&paths.state_db_file)
            .expect("open db")
            .upsert_addon(&addon)
            .expect("unlink");
    }

    #[tokio::test]
    async fn update_without_an_addons_folder_refuses_and_prints_nothing() {
        let sandbox = sandbox();
        for (profile, command) in [
            (DEFAULT_PROFILE, "`lemonup`"),
            ("forever", "`lemonup --profile forever tui`"),
        ] {
            let runtime = AppRuntime::new(profile.to_string(), None, Vec::new());
            for config in [Some(AppConfig::default()), None] {
                match &config {
                    Some(config) => {
                        ConfigStore::for_profile(&sandbox.paths)
                            .write_config(config)
                            .expect("config without a folder");
                    }
                    None => {
                        let _ = fs::remove_file(&sandbox.paths.config_file);
                    }
                }
                for mode in [OutputMode::Human, OutputMode::Json] {
                    let mut output = capture(mode);
                    let error = run_update(
                        sandbox.paths.clone(),
                        runtime.clone(),
                        Vec::new(),
                        false,
                        false,
                        false,
                        &mut output,
                    )
                    .await
                    .expect_err("refused");
                    assert_eq!(
                        error_text(error.as_ref(), Paint::PLAIN),
                        format!(
                            "No AddOns folder is set for this game. Run {command} to set it up."
                        )
                    );
                    let (stdout, stderr) = streams(output);
                    assert_eq!(stdout, "", "{profile} {mode:?}");
                    assert_eq!(stderr, "", "{profile} {mode:?}");
                }
            }
        }
        assert!(!backup_root(&sandbox.paths.data_dir).join("retail").exists());
    }

    #[tokio::test]
    async fn check_against_another_folder_warns_that_it_runs_read_only() {
        let sandbox = sandbox();
        unlink_tracked(&sandbox.paths);
        let mut human = capture(OutputMode::Human);
        let outcome = run_check(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.forever)),
            Vec::new(),
            &mut human,
        )
        .await
        .expect("check");
        assert_eq!(outcome, CommandOutcome::Success);
        let (stdout, stderr) = streams(human);
        assert!(
            stderr.starts_with("warning: profile 'default' is bound to the AddOns folder"),
            "{stderr}"
        );
        assert!(
            stderr.ends_with(". Running read-only, results are not saved.\n"),
            "{stderr}"
        );
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(!stdout.contains("warning"), "{stdout}");
        assert!(
            stdout.ends_with("\n\nNo linked addons to check.\n"),
            "{stdout}"
        );

        let mut json = capture(OutputMode::Json);
        run_check(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.forever)),
            Vec::new(),
            &mut json,
        )
        .await
        .expect("check");
        let (stdout, stderr) = streams(json);
        // The warning is on stderr too, where it stays if the run fails before its document.
        assert!(
            stderr.ends_with(". Running read-only, results are not saved.\n"),
            "{stderr}"
        );
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(document["command"], "check");
        let warnings = document["warnings"].as_array().expect("warnings");
        assert_eq!(warnings.len(), 1, "{document}");
        assert!(
            warnings[0]
                .as_str()
                .expect("text")
                .ends_with(". Running read-only, results are not saved."),
            "{document}"
        );
        assert_eq!(document["addons"][0]["source"], "manual");
        assert_eq!(document["addons"][0]["status"], "unknown");
        // Nothing was written for the other folder.
        assert_eq!(
            StateDatabase::open(&sandbox.paths.state_db_file)
                .expect("open")
                .bound_addon_dir()
                .expect("binding"),
            None
        );
    }

    #[tokio::test]
    async fn update_all_prints_its_report_and_backs_up_first() {
        let sandbox = sandbox();
        unlink_tracked(&sandbox.paths);
        let mut output = capture(OutputMode::Human);
        let outcome = run_update(
            sandbox.paths.clone(),
            runtime(None),
            Vec::new(),
            false,
            false,
            false,
            &mut output,
        )
        .await
        .expect("update");
        assert_eq!(outcome, CommandOutcome::Success);
        let (stdout, stderr) = streams(output);
        assert_eq!(stderr, "");
        let lines = stdout.lines().collect::<Vec<_>>();
        assert!(lines[0].starts_with("Retail  "), "{stdout}");
        assert_eq!(lines[1], "");
        assert!(
            lines[2].starts_with("Made a backup of your game settings: "),
            "{stdout}"
        );
        assert_eq!(
            lines[3..],
            ["", "Not linked (1)", "", "No linked addons to update."]
        );

        // A dry run makes no backup, and JSON says so.
        let mut output = capture(OutputMode::Json);
        run_update(
            sandbox.paths.clone(),
            runtime(None),
            vec!["GitAddon".to_string()],
            false,
            true,
            false,
            &mut output,
        )
        .await
        .expect("dry run");
        let (stdout, _) = streams(output);
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(document["command"], "update");
        assert_eq!(document["dry_run"], true);
        assert_eq!(document["backup"]["status"], "not_run");
        assert_eq!(document["summary"]["skipped_manual"], 1);
        assert_eq!(document["addons"][0]["folder"], "GitAddon");
    }

    #[tokio::test]
    async fn an_update_that_fails_after_its_backup_still_names_the_backup() {
        let sandbox = sandbox();
        unlink_tracked(&sandbox.paths);
        let mut output = capture(OutputMode::Human);
        run_update(
            sandbox.paths.clone(),
            runtime(None),
            vec!["NoSuchAddon".to_string()],
            false,
            false,
            false,
            &mut output,
        )
        .await
        .expect_err("an unknown addon stops the run");
        let (stdout, stderr) = streams(output);
        assert_eq!(stdout, "");
        assert!(
            stderr.starts_with("Made a backup of your game settings: "),
            "{stderr}"
        );

        // The next failing run makes none (the last one is minutes old), so it names none.
        let mut output = capture(OutputMode::Human);
        run_update(
            sandbox.paths.clone(),
            runtime(None),
            vec!["NoSuchAddon".to_string()],
            false,
            false,
            false,
            &mut output,
        )
        .await
        .expect_err("an unknown addon stops the run");
        let (stdout, stderr) = streams(output);
        assert_eq!(stdout, "");
        assert_eq!(stderr, "");
    }

    #[test]
    fn mismatched_addon_dir_refuses_restore_and_leaves_wtf_alone() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        create_wtf_backup(&sandbox.retail, &root.join("retail"), 5).expect("retail backup");
        let forever_wtf = sandbox.forever.join("../../WTF/Config.wtf");

        restore(sandbox.paths.clone(), runtime(Some(&sandbox.forever)), None)
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

        restore(sandbox.paths.clone(), runtime(None), None).expect("restore retail");
        assert_eq!(fs::read_to_string(&retail_wtf).expect("wtf"), "_retail_");
    }

    #[test]
    fn explicit_archive_from_another_client_is_refused() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        let forever_backup =
            create_wtf_backup(&sandbox.forever, &root.join("forever"), 5).expect("backup");

        let error = restore(
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

        restore(sandbox.paths.clone(), runtime(None), None).expect("restore");
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
            &mut capture(OutputMode::Human),
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
            &mut capture(OutputMode::Human),
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
            &mut capture(OutputMode::Human),
        )
        .await
        .expect_err("dry run against a different folder is refused");
        assert_eq!(bound(), None);

        // A following real sync on the valid folder binds it as usual.
        sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("sync retail");
        assert_eq!(bound(), Some(sandbox.retail.clone()));
    }

    #[test]
    fn non_default_profile_guard_includes_the_default_profiles_bound_folder() {
        let sandbox = sandbox();
        // Default profile pointed at retail with --addon-dir only: no addon_dir in its config.
        fs::remove_file(&sandbox.paths.config_file).expect("remove config");
        sync(sandbox.paths.clone(), runtime(Some(&sandbox.retail))).expect("default sync");
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
        let error = sync(
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
    fn sync_with_a_leftover_staging_folder_succeeds_and_does_not_track_it() {
        let sandbox = sandbox();
        let staging = sandbox.retail.join(".lemonup-staging-9-9-0").join("old");
        fs::create_dir_all(staging.join("Old")).expect("staging");
        fs::write(staging.join("Old").join("Old.toc"), "## Title: Old\n").expect("toc");

        let mut output = capture(OutputMode::Human);
        let outcome = run_sync(
            sandbox.paths.clone(),
            runtime(Some(&sandbox.retail)),
            &mut output,
        )
        .expect("sync");
        assert_eq!(outcome, CommandOutcome::Success);
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

        // The report is on stdout and the warning on stderr.
        let staging_dir = sandbox.retail.join(".lemonup-staging-9-9-0");
        let (stdout, stderr) = streams(output);
        assert_eq!(
            stdout,
            format!(
                "Retail  {}\n\n✓ 2 addons found - sync complete\n",
                crate::app::friendly_path(&sandbox.retail)
            )
        );
        assert_eq!(
            stderr,
            format!(
                "warning: Leftover update staging folder {} may contain previous addon \
                 versions. See Troubleshooting.\n",
                staging_dir.display()
            )
        );

        // With --json the warning is in the document, and on stderr as well.
        let mut output = capture(OutputMode::Json);
        run_sync(sandbox.paths.clone(), runtime(None), &mut output).expect("sync");
        let (stdout, stderr) = streams(output);
        assert!(
            stderr.starts_with("warning: Leftover update staging folder "),
            "{stderr}"
        );
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(document["command"], "sync");
        assert_eq!(
            document["summary"],
            serde_json::json!({
                "scanned_addons": 2, "upserted_addons": 2, "removed_addons": 0, "skipped": 0
            })
        );
        let staging_path = staging_dir.display().to_string();
        assert_eq!(
            document["leftover_staging"],
            serde_json::json!([staging_path.clone()])
        );
        assert_eq!(
            document["warnings"],
            serde_json::json!([format!(
                "Leftover update staging folder {staging_path} may contain previous addon \
                 versions. See Troubleshooting."
            )])
        );
    }

    #[test]
    fn a_failed_sync_ends_with_the_failed_mark_and_why_and_a_refusal_stays_as_it_is() {
        let sandbox = sandbox();
        let gone = sandbox.retail.join("gone");
        for mode in [OutputMode::Human, OutputMode::Json] {
            let mut output = capture(mode);
            let outcome = run_sync(sandbox.paths.clone(), runtime(Some(&gone)), &mut output)
                .expect("the failure is said on stderr");
            assert_eq!(outcome, CommandOutcome::Failed);
            assert_eq!(outcome.exit_status(), 1);
            let (stdout, stderr) = streams(output);
            assert_eq!(stdout, "", "{mode:?}");
            assert_eq!(
                stderr,
                format!("✗ Sync failed - path does not exist: {}\n", gone.display()),
                "{mode:?}"
            );
        }

        // A sync that works is untouched.
        let mut output = capture(OutputMode::Human);
        let outcome = run_sync(sandbox.paths.clone(), runtime(None), &mut output).expect("sync");
        assert_eq!(outcome, CommandOutcome::Success);
        let (stdout, stderr) = streams(output);
        assert!(stdout.ends_with(" - sync complete\n"), "{stdout}");
        assert_eq!(stderr, "");

        // A report that can't be written is an error of its own: the sync itself worked.
        struct FullDisk;
        impl std::io::Write for FullDisk {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::StorageFull.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = Output::new(
            OutputMode::Human,
            Paint::PLAIN,
            Paint::PLAIN,
            DEFAULT_PROFILE,
            FullDisk,
            Vec::new(),
        );
        run_sync(sandbox.paths.clone(), runtime(None), &mut output).expect_err("not written");
        let (_, stderr) = output.into_writers();
        assert_eq!(String::from_utf8(stderr).expect("utf-8"), "");

        // A refusal says what to do next, so it is not called a failure.
        without_addon_dir(&sandbox, None);
        let mut output = capture(OutputMode::Human);
        let refusal =
            run_sync(sandbox.paths.clone(), runtime(None), &mut output).expect_err("refused");
        assert_eq!(
            refusal.to_string(),
            "No AddOns folder is set for this game. Run `lemonup` to set it up."
        );
        assert_eq!(streams(output), (String::new(), String::new()));
    }

    /// Writes the sandbox's config without an AddOns folder, or removes it.
    fn without_addon_dir(sandbox: &Sandbox, config: Option<AppConfig>) {
        match &config {
            Some(config) => {
                ConfigStore::for_profile(&sandbox.paths)
                    .write_config(config)
                    .expect("config without a folder");
            }
            None => {
                let _ = fs::remove_file(&sandbox.paths.config_file);
            }
        }
    }

    #[tokio::test]
    async fn sync_toc_import_and_restore_refuse_without_an_addons_folder_and_print_nothing() {
        let sandbox = sandbox();
        let input = empty_import_file(&sandbox);
        let expected = "No AddOns folder is set for this game. Run `lemonup` to set it up.";
        for config in [Some(AppConfig::default()), None] {
            without_addon_dir(&sandbox, config);
            for mode in [OutputMode::Human, OutputMode::Json] {
                let mut errors = Vec::new();
                let mut output = capture(mode);
                errors.push(
                    run_sync(sandbox.paths.clone(), runtime(None), &mut output).expect_err("sync"),
                );
                errors.push(
                    run_toc(
                        sandbox.paths.clone(),
                        runtime(None),
                        "GitAddon",
                        None,
                        false,
                        &mut output,
                    )
                    .expect_err("toc"),
                );
                errors.push(
                    run_import_addons(
                        sandbox.paths.clone(),
                        runtime(None),
                        Some(input.clone()),
                        false,
                        &mut output,
                    )
                    .await
                    .expect_err("import"),
                );
                errors.push(
                    run_restore_backup(sandbox.paths.clone(), runtime(None), None, &mut output)
                        .expect_err("restore"),
                );
                for error in errors {
                    assert_eq!(
                        error_text(error.as_ref(), Paint::PLAIN),
                        expected,
                        "{mode:?}"
                    );
                }
                let (stdout, stderr) = streams(output);
                assert_eq!(stdout, "", "{mode:?}");
                assert_eq!(stderr, "", "{mode:?}");
            }
        }
    }

    #[test]
    fn restore_without_a_backup_is_refused_and_prints_nothing() {
        let sandbox = sandbox();
        for mode in [OutputMode::Human, OutputMode::Json] {
            let mut output = capture(mode);
            let error = run_restore_backup(sandbox.paths.clone(), runtime(None), None, &mut output)
                .expect_err("no backup");
            assert_eq!(
                error_text(error.as_ref(), Paint::PLAIN),
                "There is no backup of your game settings to restore."
            );
            let (stdout, stderr) = streams(output);
            assert_eq!(stdout, "", "{mode:?}");
            assert_eq!(stderr, "", "{mode:?}");
        }
    }

    #[test]
    fn restore_prints_the_backup_it_put_back() {
        let sandbox = sandbox();
        let root = backup_root(&sandbox.paths.data_dir);
        let created = create_wtf_backup(&sandbox.retail, &root.join("retail"), 5).expect("backup");
        let mut output = capture(OutputMode::Human);
        run_restore_backup(sandbox.paths.clone(), runtime(None), None, &mut output)
            .expect("restore");
        let (stdout, stderr) = streams(output);
        assert_eq!(stderr, "");
        assert_eq!(
            stdout,
            format!(
                "✓ Restored your game settings from {}\n",
                crate::app::friendly_path(&created.backup.path)
            )
        );

        let mut output = capture(OutputMode::Json);
        run_restore_backup(sandbox.paths.clone(), runtime(None), None, &mut output)
            .expect("restore");
        let (stdout, _) = streams(output);
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(document["command"], "restore-backup");
        assert_eq!(
            document["archive"],
            created.backup.path.display().to_string()
        );
        assert_eq!(document["addon_dir"], sandbox.retail.display().to_string());
    }

    #[tokio::test]
    async fn export_then_a_dry_run_import_lists_every_addon_of_the_file() {
        let sandbox = sandbox();
        let mut other = AddonRecord::new("OtherAddon", "OtherAddon", SourceKind::GitHub);
        other.source_url = Some("https://github.com/example/OtherAddon".to_string());
        let mut not_linked = AddonRecord::new("HandMade", "HandMade", SourceKind::Manual);
        not_linked.source_url = None;
        let database = StateDatabase::open(&sandbox.paths.state_db_file).expect("open");
        database.upsert_addon(&other).expect("other");
        database.upsert_addon(&not_linked).expect("hand made");
        drop(database);

        let file = sandbox.paths.data_dir.join("list.json");
        let mut output = capture(OutputMode::Human);
        run_export_addons(sandbox.paths.clone(), Some(file.clone()), &mut output).expect("export");
        let (stdout, stderr) = streams(output);
        assert_eq!(stderr, "");
        assert_eq!(
            stdout,
            format!("✓ Saved 3 addons to {}\n", crate::app::friendly_path(&file))
        );
        let mut output = capture(OutputMode::Json);
        run_export_addons(sandbox.paths.clone(), Some(file.clone()), &mut output).expect("export");
        let (stdout, _) = streams(output);
        assert_eq!(
            stdout,
            format!(
                "{{\"schema\":1,\"command\":\"export-addons\",\"profile\":\"default\",\
                 \"addon_dir\":null,\"file\":\"{}\",\"count\":3,\"warnings\":[]}}\n",
                file.display()
            )
        );

        // OtherAddon has no folder on disk once LemonUp forgets it, so it would be installed.
        StateDatabase::open(&sandbox.paths.state_db_file)
            .expect("open")
            .remove_addon("OtherAddon")
            .expect("forget");
        let mut output = capture(OutputMode::Human);
        let outcome = run_import_addons(
            sandbox.paths.clone(),
            runtime(None),
            Some(file.clone()),
            true,
            &mut output,
        )
        .await
        .expect("dry run");
        assert_eq!(outcome, CommandOutcome::Success);
        let (stdout, stderr) = streams(output);
        assert_eq!(stderr, "");
        assert_eq!(
            stdout,
            format!(
                "Retail  {}\n\nWould install (1)\n  OtherAddon  GitHub\n\nAlready installed (1)\n\
                 Not linked, can't install (1)\n\nDry run - no changes\n",
                crate::app::friendly_path(&sandbox.retail)
            )
        );

        let mut output = capture(OutputMode::Json);
        run_import_addons(
            sandbox.paths.clone(),
            runtime(None),
            Some(file.clone()),
            true,
            &mut output,
        )
        .await
        .expect("dry run");
        let (stdout, _) = streams(output);
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(document["command"], "import-addons");
        assert_eq!(document["file"], file.display().to_string());
        assert_eq!(document["dry_run"], true);
        assert_eq!(
            document["addons"],
            serde_json::json!([
                {"name": "GitAddon", "source": "github", "status": "already_installed", "message": null},
                {"name": "HandMade", "source": "manual", "status": "skipped_manual", "message": null},
                {"name": "OtherAddon", "source": "github", "status": "would_install", "message": null},
            ])
        );
        // Nothing was installed.
        assert!(!sandbox.retail.join("OtherAddon").exists());
    }

    #[tokio::test]
    async fn a_dry_run_import_needs_no_addons_folder() {
        let sandbox = sandbox();
        let input = empty_import_file(&sandbox);
        without_addon_dir(&sandbox, None);
        let mut output = capture(OutputMode::Human);
        let outcome = run_import_addons(
            sandbox.paths.clone(),
            runtime(None),
            Some(input),
            true,
            &mut output,
        )
        .await
        .expect("a dry run only reads the file");
        assert_eq!(outcome, CommandOutcome::Success);
        let (stdout, stderr) = streams(output);
        assert_eq!(stderr, "");
        // No folder, so no game line; the report is the closing line alone.
        assert_eq!(stdout, "Dry run - no changes\n");
    }

    #[tokio::test]
    async fn an_import_that_fails_lists_the_addon_and_exits_one() {
        let sandbox = sandbox();
        // Not a GitHub address, so the install fails before any request is made.
        let mut bad = AddonRecord::new("BadLink", "BadLink", SourceKind::GitHub);
        bad.source_url = Some("https://example.com/owner/repo".to_string());
        let database = StateDatabase::open(&sandbox.paths.state_db_file).expect("open");
        database.upsert_addon(&bad).expect("bad link");
        drop(database);
        let file = sandbox.paths.data_dir.join("list.json");
        let mut output = capture(OutputMode::Human);
        run_export_addons(sandbox.paths.clone(), Some(file.clone()), &mut output).expect("export");
        StateDatabase::open(&sandbox.paths.state_db_file)
            .expect("open")
            .remove_addon("BadLink")
            .expect("forget");

        let mut output = capture(OutputMode::Human);
        let outcome = run_import_addons(
            sandbox.paths.clone(),
            runtime(None),
            Some(file.clone()),
            false,
            &mut output,
        )
        .await
        .expect("the run finishes");
        assert_eq!(outcome, CommandOutcome::Failures(1));
        assert_eq!(outcome.exit_status(), 1);
        let (stdout, _) = streams(output);
        let lines = stdout.lines().collect::<Vec<_>>();
        let failed = lines
            .iter()
            .position(|line| *line == "Failed (1)")
            .unwrap_or_else(|| panic!("no Failed section: {stdout}"));
        assert_eq!(lines[failed + 1], "  BadLink  GitHub");
        assert!(lines[failed + 2].starts_with("      "), "{stdout}");
        assert!(!stdout.contains("Installed ("), "{stdout}");
        assert_eq!(
            lines.last().copied(),
            Some(format!("1 failed, from {}", crate::app::friendly_path(&file)).as_str())
        );

        let mut output = capture(OutputMode::Json);
        let outcome = run_import_addons(
            sandbox.paths.clone(),
            runtime(None),
            Some(file),
            false,
            &mut output,
        )
        .await
        .expect("the run finishes");
        assert_eq!(outcome, CommandOutcome::Failures(1));
        let (stdout, _) = streams(output);
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(document["summary"]["errors"], 1);
        assert_eq!(document["summary"]["installed"], 0);
        let failed = document["addons"]
            .as_array()
            .expect("addons")
            .iter()
            .find(|addon| addon["name"] == "BadLink")
            .expect("bad link");
        assert_eq!(failed["status"], "error");
        assert!(failed["message"].is_string(), "{failed}");
    }

    #[test]
    fn toc_shows_sets_and_clears_a_field_through_the_report() {
        use lemonup_core::TocField;
        let sandbox = sandbox();
        let toc = |change: Option<(TocField, Option<String>)>, mode| {
            let mut output = capture(mode);
            run_toc(
                sandbox.paths.clone(),
                runtime(None),
                "GitAddon",
                change,
                false,
                &mut output,
            )
            .expect("toc");
            let (stdout, stderr) = streams(output);
            assert_eq!(stderr, "");
            stdout
        };
        assert_eq!(
            toc(None, OutputMode::Human),
            "Title     GitAddon\nNotes     none\nCategory  none\n"
        );
        assert_eq!(
            toc(
                Some((TocField::Category, Some("Bags".to_string()))),
                OutputMode::Human
            ),
            "✓ Saved Category for GitAddon: Bags\n/reload to take effect in game\n"
        );
        assert_eq!(
            toc(None, OutputMode::Human),
            "Title     GitAddon\nNotes     none\nCategory  Bags  (yours)\n"
        );
        let cleared: serde_json::Value =
            serde_json::from_str(&toc(Some((TocField::Category, None)), OutputMode::Json))
                .expect("json");
        assert_eq!(cleared["command"], "toc");
        assert_eq!(cleared["addon"], "GitAddon");
        assert_eq!(
            cleared["changed"],
            serde_json::json!({"field": "Category", "value": null, "folders": ["GitAddon"]})
        );
        // The fields after the change.
        assert_eq!(
            cleared["fields"][2],
            serde_json::json!({"field": "Category", "value": null, "source": "addon"})
        );
        assert_eq!(
            toc(None, OutputMode::Human),
            "Title     GitAddon\nNotes     none\nCategory  none\n"
        );
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

        let show = |database: &mut StateDatabase, selector: &str| {
            toc_command(database, &addon_dir, selector, None, false)
                .expect("show")
                .human(Paint::PLAIN)
        };
        assert_eq!(
            show(&mut database, "foo"),
            ["Title     Foo", "Notes     none", "Category  Bags",]
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
            saved.human(Paint::PLAIN),
            [
                "✓ Saved Category for Foo: My Bags",
                "/reload to take effect in game"
            ]
        );
        let toc = std::fs::read_to_string(addon_dir.join("Foo/Foo.toc")).expect("toc");
        assert!(toc.contains("## Category: My Bags"), "{toc}");
        assert!(toc.contains("# LemonUp: ## Category: Bags"), "{toc}");
        assert_eq!(show(&mut database, "Foo")[2], "Category  My Bags  (yours)");

        // Another program puts the addon's own line back.
        std::fs::write(
            addon_dir.join("Foo/Foo.toc"),
            "## Interface: 110005\n## Title: Foo\n## Category: Bags\n",
        )
        .expect("toc");
        assert_eq!(
            show(&mut database, "Foo")[2],
            "Category  Bags  (yours, not in the files)"
        );

        let cleared = toc_command(
            &mut database,
            &addon_dir,
            "Foo",
            Some((TocField::Category, None)),
            false,
        )
        .expect("clear");
        assert_eq!(cleared.human(Paint::PLAIN), ["✓ Restored Foo's Category"]);
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

        assert_eq!(
            error_text(refusal.as_ref(), Paint::PLAIN),
            "Nothing was linked."
        );
        assert_eq!(
            error_text(failure.as_ref(), Paint::PLAIN),
            "Error: invalid argument: bad"
        );
        // With color the label is bold red; a refusal still has no label and no color.
        assert_eq!(
            error_text(failure.as_ref(), Paint::terminal()),
            "\x1b[1m\x1b[38;5;1mError:\x1b[0m invalid argument: bad"
        );
        assert_eq!(
            error_text(refusal.as_ref(), Paint::terminal()),
            "Nothing was linked."
        );
    }

    #[test]
    fn a_refusal_and_an_error_lose_control_characters() {
        // A refusal can name an addon, whose title comes from its files.
        let refusal: Box<dyn std::error::Error> = Box::new(LemonupError::Refused(
            "Evil\x1b]0;x\x07 is Not linked, so it needs `--yes`.\u{9b}".to_string(),
        ));
        assert_eq!(
            error_text(refusal.as_ref(), Paint::PLAIN),
            "Evil ]0;x  is Not linked, so it needs `--yes`. "
        );
        assert_eq!(
            error_text(refusal.as_ref(), Paint::terminal()),
            "Evil ]0;x  is Not linked, so it needs \x1b[1m\x1b[38;5;6m--yes\x1b[0m. "
        );
        // Any other error can carry a source's own words.
        let failure: Box<dyn std::error::Error> = Box::new(LemonupError::InvalidArgument(
            "bad\x1b[31m name\x07".to_string(),
        ));
        assert_eq!(
            error_text(failure.as_ref(), Paint::PLAIN),
            "Error: invalid argument: bad [31m name "
        );
    }

    #[test]
    fn a_failure_gives_its_reason_without_the_kind_of_error() {
        let argument: Box<dyn std::error::Error> = Box::new(LemonupError::InvalidArgument(
            "path does not exist: /x".to_string(),
        ));
        assert_eq!(
            super::failure_reason(argument.as_ref()),
            "path does not exist: /x"
        );
        let other: Box<dyn std::error::Error> = Box::new(std::io::Error::other("disk on fire"));
        assert_eq!(super::failure_reason(other.as_ref()), "disk on fire");
        assert!(!super::is_refusal(other.as_ref()));
        let refusal: Box<dyn std::error::Error> = Box::new(LemonupError::Refused("no".to_string()));
        assert!(super::is_refusal(refusal.as_ref()));
    }

    #[test]
    fn an_error_and_a_refusal_draw_the_commands_they_name() {
        let command = |text: &str| format!("\x1b[1m\x1b[38;5;6m{text}\x1b[0m");
        let refusal: Box<dyn std::error::Error> = Box::new(super::no_addon_dir_refusal("forever"));
        assert_eq!(
            error_text(refusal.as_ref(), Paint::terminal()),
            format!(
                "No AddOns folder is set for this game. Run {} to set it up.",
                command("lemonup --profile forever tui")
            )
        );
        assert_eq!(
            error_text(refusal.as_ref(), Paint::PLAIN),
            "No AddOns folder is set for this game. Run `lemonup --profile forever tui` to set it up."
        );
        let failure: Box<dyn std::error::Error> = Box::new(LemonupError::InvalidArgument(
            "no match, run `lemonup check`".to_string(),
        ));
        assert_eq!(
            error_text(failure.as_ref(), Paint::terminal()),
            format!(
                "\x1b[1m\x1b[38;5;1mError:\x1b[0m invalid argument: no match, run {}",
                command("lemonup check")
            )
        );
    }

    #[test]
    fn a_startup_notice_draws_the_command_it_names() {
        let notice = super::StartupNotice {
            text: "Moved your v1 settings over".to_string(),
            is_error: true,
            details: vec![
                "to bring addons over: export in v1, then run `lemonup import-addons`".to_string(),
            ],
        };
        assert_eq!(
            notice.lines(Paint::PLAIN),
            [
                "Moved your v1 settings over",
                "  to bring addons over: export in v1, then run `lemonup import-addons`",
            ]
        );
        assert_eq!(
            notice.lines(Paint::terminal())[1],
            "  to bring addons over: export in v1, then run \x1b[1m\x1b[38;5;6mlemonup import-addons\x1b[0m"
        );
    }

    #[test]
    fn toc_command_prefers_a_folder_and_refuses_a_name_two_addons_have() {
        use lemonup_core::TocField;
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = toc_fixture(temp.path());
        // A plugin whose Wago.io name is the other addon's folder.
        let mut plugin =
            lemonup_core::AddonRecord::new("Foo Plugin", "FooPlugin", SourceKind::Wago);
        plugin.source_name = Some("Foo".to_string());
        database.upsert_addon(&plugin).expect("plugin");
        let shown = toc_command(&mut database, &addon_dir, "foo", None, false)
            .expect("show")
            .human(Paint::PLAIN);
        assert_eq!(shown[0], "Title     Foo");

        let mut one = lemonup_core::AddonRecord::new("Twin", "One", SourceKind::Manual);
        one.source_name = None;
        let mut two = lemonup_core::AddonRecord::new("Two Toc", "Two", SourceKind::Wago);
        two.source_name = Some("Twin".to_string());
        database.upsert_addon(&one).expect("one");
        database.upsert_addon(&two).expect("two");
        let error = toc_command(
            &mut database,
            &addon_dir,
            "twin",
            Some((TocField::Category, Some("Mine".to_string()))),
            true,
        )
        .expect_err("ambiguous");
        let message = error.to_string();
        assert!(
            message.contains("More than one addon is named twin"),
            "{message}"
        );
        assert!(message.contains("One, Two"), "{message}");
        assert!(message.contains("Use the folder name instead"), "{message}");
        for folder in ["One", "Two"] {
            let row = database
                .get_addon_by_folder(folder)
                .expect("get")
                .expect("row");
            assert!(row.toc_overrides.is_empty(), "{folder}");
        }
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
        let printed = error_text(&refusal, Paint::PLAIN);
        assert!(!printed.contains("invalid argument"), "{printed}");
        assert!(!printed.starts_with("Error"), "{printed}");
        assert!(printed.ends_with("Nothing was changed."), "{printed}");
        let error = refusal.to_string();
        assert!(error.contains("Bar is Not linked"), "{error}");
        assert!(error.contains("needs `--yes`."), "{error}");
        // With color the flags are drawn as commands.
        let colored = error_text(&refusal, Paint::terminal());
        assert!(
            colored.contains("needs \x1b[1m\x1b[38;5;6m--yes\x1b[0m. Its own lines are kept, and \x1b[1m\x1b[38;5;6m--clear\x1b[0m puts"),
            "{colored:?}"
        );
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

    use super::{command_profile, load_profile_config};
    use crate::cli::Commands;
    use crate::output::{Paint, ProfilesReport, Report};

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

        // Every command that reads its settings reads them that way, `find` included:
        // none opens a profile's config file on its own.
        let source = include_str!("main.rs");
        let commands = [
            "fn run_toc<",
            "async fn run_find(",
            "async fn run_check<",
            "async fn run_update<",
            "fn sync_report(",
            "async fn run_import_addons<",
            "fn run_restore_backup<",
            "async fn run_install_wago(",
            "async fn run_install_tukui(",
            "async fn run_install_wowinterface(",
            "async fn run_install_github(",
        ];
        for command in commands {
            let start = source.find(command).unwrap_or_else(|| panic!("{command}"));
            let rest = &source[start + command.len()..];
            let end = ["\nfn ", "\nasync fn "]
                .iter()
                .filter_map(|next| rest.find(next))
                .min()
                .unwrap_or(rest.len());
            let body = &rest[..end];
            assert!(body.contains("load_profile_config("), "{command}");
            assert!(!body.contains("ConfigStore::new("), "{command}");
        }
    }

    #[test]
    fn only_the_tui_opens_the_last_used_game() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        create_profile(&roots, &install(temp.path(), "_forever_"), None).expect("create");
        remember_last_used(&roots, "forever").expect("remember");

        let profile = |requested: Option<&str>, command: Commands| {
            command_profile(&roots, requested.map(str::to_string), &command)
                .expect("profile")
                .name
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

        let lines = ProfilesReport::new(&list_profiles(&roots).expect("list")).human(Paint::PLAIN);
        assert!(lines.len() >= 5, "{lines:?}");
        assert_eq!(lines[0], "Games (3)");
        let retail = crate::app::friendly_path(&retail);
        let forever = crate::app::friendly_path(&forever);
        let width = retail.chars().count().max(forever.chars().count());
        assert_eq!(
            lines[1],
            format!("  default  Retail       {retail:<width$}  2 addons")
        );
        assert_eq!(
            lines[2],
            format!(
                "  broken   broken       {:<width$}  0 addons",
                "no AddOns folder set"
            )
        );
        // What couldn't be read is a note under its row, however many lines it takes.
        let (last, notes) = lines[3..].split_last().expect("rows");
        assert!(
            notes[0].starts_with("      couldn't read its settings:"),
            "{notes:?}"
        );
        assert!(
            notes.iter().all(|line| line.starts_with("      ")),
            "{notes:?}"
        );
        assert_eq!(
            *last,
            format!("  forever  WoW Forever  {forever:<width$}  0 addons")
        );
    }
}
