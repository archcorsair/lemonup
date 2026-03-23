mod action;
mod app;
mod cli;
mod drift;
mod event;
mod onboarding;
mod tui;
mod tukui;
mod update;
mod wago;
mod wowinterface;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use lemonup_core::{
    AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE, LemonupError, StateDatabase, UpdateStatus,
    validate_addons_path,
};
use tracing_subscriber::{EnvFilter, fmt};

use crate::app::{App, AppRuntime};
use crate::cli::{Cli, Commands};
use crate::event::EventHandler;
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
        Commands::Check { addons } => run_check(paths, addons).await?,
        Commands::Update {
            addons,
            force,
            dry_run,
        } => run_update(paths, runtime, addons, force, dry_run).await?,
        Commands::UpdateAll { force, dry_run } => {
            run_update(paths, runtime, Vec::new(), force, dry_run).await?
        }
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

async fn run_check(paths: AppPaths, addons: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
    let mut database = StateDatabase::open(paths.state_db_file)?;
    let show_details = !addons.is_empty();
    let api_key = resolve_wago_api_key(&config_state);
    let checks = refresh_live_update_checks(&mut database, &addons, api_key.as_deref())
        .await
        .map_err(Box::<dyn std::error::Error>::from)?;

    let up_to_date = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::UpToDate)
        .count();
    let update_available = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::UpdateAvailable)
        .count();
    let unknown = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::Unknown)
        .count();
    let errors = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::Error)
        .count();

    println!(
        "check summary: targets={}, checked={}, up_to_date={}, update_available={}, unknown={}, errors={}",
        if addons.is_empty() {
            "all".to_string()
        } else {
            addons.join("|")
        },
        checks.len(),
        up_to_date,
        update_available,
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

    Ok(())
}

async fn run_update(
    paths: AppPaths,
    runtime: AppRuntime,
    addons: Vec<String>,
    force: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
    let mut database = StateDatabase::open(paths.state_db_file)?;
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
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) if effective_addon_dir.is_none() => {
            println!(
                "update skipped: profile={}, tracked_addons={}, targets={}, addon_dir=<unconfigured>, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                render_target_list(&addons),
                force,
                dry_run
            );
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) => {
            let addon_dir = effective_addon_dir.expect("checked above");
            let api_key = resolve_wago_api_key(&config_state);
            let run = apply_live_updates(
                &mut database,
                &addon_dir,
                &addons,
                api_key.as_deref(),
                force,
                dry_run,
            )
            .await?;
            let summary = run.summary;
            println!(
                "update summary: profile={}, tracked_addons={}, targets={}, updated={}, up_to_date={}, skipped_manual={}, skipped_unmanaged={}, skipped_unsupported={}, errors={}, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                render_target_list(&addons),
                summary.updated_addons,
                summary.up_to_date,
                summary.skipped_manual,
                summary.skipped_unmanaged,
                summary.skipped_unsupported,
                summary.errors,
                force,
                dry_run
            );
            print_update_details(&addons, &run.results);
        }
    }

    Ok(())
}

fn render_target_list(addons: &[String]) -> String {
    if addons.is_empty() {
        "all".to_string()
    } else {
        addons.join("|")
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
    let skipped_unmanaged = results
        .iter()
        .filter(|result| result.status == LiveUpdateStatus::SkippedUnmanaged)
        .count();
    let skipped_unsupported = results
        .iter()
        .filter(|result| result.status == LiveUpdateStatus::SkippedUnsupported)
        .count();
    let error_lines = results
        .iter()
        .filter(|result| result.status == LiveUpdateStatus::Error)
        .map(format_update_result_line)
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    if skipped_manual > 0 || skipped_unmanaged > 0 || skipped_unsupported > 0 {
        lines.push(format!(
            "update detail: skipped manual={}, unmanaged={}, unsupported={} | rerun `lemonup update <addon...>` for per-addon detail",
            skipped_manual, skipped_unmanaged, skipped_unsupported
        ));
    }

    if lines.is_empty() && error_lines.is_empty() {
        lines.push(
            "update detail: all processed targets were updated or already up to date".to_string(),
        );
        return lines;
    }

    lines.extend(error_lines);
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
) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
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
        return Ok(());
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

    let Some(api_key) = resolve_wago_api_key(&config_state) else {
        println!(
            "install skipped: profile={}, addon={}, reason=no Wago API key configured",
            runtime.profile_name, addon
        );
        return Ok(());
    };

    let mut database = StateDatabase::open(paths.state_db_file)?;
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

    Ok(())
}

async fn run_install_tukui(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
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
        return Ok(());
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

    let mut database = StateDatabase::open(paths.state_db_file)?;
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

    Ok(())
}

async fn run_install_wowinterface(
    paths: AppPaths,
    runtime: AppRuntime,
    addon: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = ConfigStore::new(paths.config_file.clone());
    let config_state = config_store.load()?;
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
        return Ok(());
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

    let mut database = StateDatabase::open(paths.state_db_file)?;
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{build_update_detail_lines, refresh_managed_update_state};
    use crate::update::{
        LiveUpdateResult, LiveUpdateStatus, build_managed_update_record, build_update_checks,
        determine_update_status,
    };
    use lemonup_core::{
        AddonKind, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase, UpdateStatus,
    };

    fn write_addon(root: &std::path::Path, folder: &str, toc_body: &str) {
        let addon_dir = root.join(folder);
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        std::fs::write(addon_dir.join(format!("{folder}.toc")), toc_body).expect("write toc");
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
                .contains("No tracked addon matches: Unknown")
        );
    }

    #[test]
    fn determine_update_status_handles_manual_and_missing_versions() {
        let manual = lemonup_core::AddonRecord::new("Scratch", "Scratch", SourceKind::Manual);
        let (status, message) = determine_update_status(&manual);
        assert_eq!(status, UpdateStatus::Unknown);
        assert_eq!(
            message.as_deref(),
            Some("manual addons cannot be checked yet")
        );

        let tracked = lemonup_core::AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        let (status, message) = determine_update_status(&tracked);
        assert_eq!(status, UpdateStatus::Unknown);
        assert_eq!(message.as_deref(), Some("no tracked remote version yet"));
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
                update_result(
                    "DBM-Core",
                    LiveUpdateStatus::SkippedUnsupported,
                    SourceKind::GitHub,
                ),
            ],
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            "update detail: skipped manual=2, unmanaged=0, unsupported=1 | rerun `lemonup update <addon...>` for per-addon detail"
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
