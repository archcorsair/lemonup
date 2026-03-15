mod action;
mod app;
mod cli;
mod event;
mod onboarding;
mod tui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use lemonup_core::{
    AddonRecord, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE, GameFlavor, LemonupError,
    ScannedAddon, StateDatabase, scan_addons_dir, validate_addons_path,
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
                "update skipped: profile={}, tracked addons={}, addon_dir=<unconfigured>, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                force,
                dry_run
            );
        }
        ConfigLoad::Missing(_) | ConfigLoad::Loaded(_) => {
            let addon_dir = effective_addon_dir.expect("checked above");
            let summary = refresh_managed_update_state(&mut database, &addon_dir, dry_run)?;
            println!(
                "update refresh: profile={}, tracked addons={}, scanned={}, refreshed={}, skipped_unmanaged={}, missing_on_disk={}, force={}, dry_run={}",
                runtime.profile_name,
                installed.len(),
                summary.scanned_addons,
                summary.refreshed_addons,
                summary.skipped_unmanaged,
                summary.missing_on_disk,
                force,
                dry_run
            );
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UpdateRefreshSummary {
    scanned_addons: usize,
    refreshed_addons: usize,
    skipped_unmanaged: usize,
    missing_on_disk: usize,
}

fn refresh_managed_update_state(
    database: &mut StateDatabase,
    addon_dir: &std::path::Path,
    dry_run: bool,
) -> lemonup_core::Result<UpdateRefreshSummary> {
    let scanned = scan_addons_dir(addon_dir, GameFlavor::Retail)?;
    let scanned_by_folder = scanned
        .iter()
        .map(|addon| (addon.folder.as_str(), addon))
        .collect::<HashMap<_, _>>();

    let mut refreshed_addons = 0;
    let mut skipped_unmanaged = 0;
    let mut missing_on_disk = 0;

    for addon in database.list_addons()? {
        if addon.source == lemonup_core::SourceKind::Manual {
            continue;
        }

        if !addon.has_authoritative_owned_folders() {
            skipped_unmanaged += 1;
            continue;
        }

        let Some(scanned_addon) = scanned_by_folder.get(addon.folder.as_str()) else {
            missing_on_disk += 1;
            continue;
        };

        let refreshed = build_managed_update_record(&addon, scanned_addon);
        if !dry_run {
            database.record_managed_addon(&refreshed)?;
        }
        refreshed_addons += 1;
    }

    Ok(UpdateRefreshSummary {
        scanned_addons: scanned.len(),
        refreshed_addons,
        skipped_unmanaged,
        missing_on_disk,
    })
}

fn build_managed_update_record(existing: &AddonRecord, scanned: &ScannedAddon) -> AddonRecord {
    let mut refreshed = AddonRecord {
        id: existing.id,
        name: scanned.name.clone(),
        folder: existing.folder.clone(),
        owned_folders: existing.owned_folders.clone(),
        ownership_source: existing.effective_ownership_source(),
        kind: if existing.kind_override {
            existing.kind
        } else {
            scanned.kind
        },
        kind_override: existing.kind_override,
        flavor: scanned.flavor,
        version: scanned.version.clone(),
        git_commit: scanned.git_commit.clone(),
        author: scanned.author.clone(),
        interface: scanned.interface.clone(),
        source: existing.source,
        source_url: existing.source_url.clone(),
        required_deps: scanned.required_deps.clone(),
        optional_deps: scanned.optional_deps.clone(),
        embedded_libs: scanned.embedded_libs.clone(),
        installed_at: existing.installed_at,
        updated_at: existing.updated_at,
        last_checked_at: existing.last_checked_at,
        remote_version: existing.remote_version.clone(),
    };
    refreshed.set_managed_owned_folders(existing.owned_folders.clone());
    refreshed
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

    use super::{build_managed_update_record, refresh_managed_update_state};
    use lemonup_core::{AddonKind, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase};

    fn write_addon(root: &std::path::Path, folder: &str, toc_body: &str) {
        let addon_dir = root.join(folder);
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        std::fs::write(addon_dir.join(format!("{folder}.toc")), toc_body).expect("write toc");
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
}
