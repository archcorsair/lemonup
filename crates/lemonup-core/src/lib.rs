pub mod binding;
pub mod config;
pub mod domain;
pub mod error;
pub mod events;
pub mod find;
pub mod fingerprint;
pub mod paths;
pub mod scan;
pub mod storage;
pub mod toc_conditions;
pub mod toc_edit;
pub mod transfer;
pub mod v1_migration;
pub mod wow;

pub use config::{AppConfig, ConfigLoad, ConfigStore, ThemeMode};
pub use domain::{
    AddonKind, AddonRecord, GameFlavor, InstallPlan, InstallSource, OwnedFolder, OwnershipSource,
    SourceKind, UnrecognizedValues, UpdateCheck, UpdateStatus,
};
pub use error::{LemonupError, Result};
pub use events::{OperationKind, OperationProgress, OperationStage};
pub use paths::{AppPaths, DEFAULT_PROFILE, paths_match};
pub use scan::{
    ScanReport, ScanSummary, ScannedAddon, TocMetadata, TocSelectionConfidence, TocSelectionResult,
    UPDATE_STAGING_PREFIX, parse_toc_content, read_addon_toc_fields, scan_addons_dir,
    scan_addons_dir_report, scan_addons_dir_with_tracked, select_toc_file,
};
pub use storage::StateDatabase;
pub use toc_edit::{TocField, TocFieldValues, TocValueSource};
pub use transfer::{
    DEFAULT_TRANSFER_FILE_NAME, ImportAnalysis, TransferAddon, TransferFile, TransferSourceKind,
    analyze_import, default_transfer_path, export_addons, parse_import_file,
};
pub use v1_migration::{V1Location, V1MigrationOutcome, V1MigrationReport, migrate_v1_if_needed};
pub use wow::{
    ScanProgressUpdate, addon_count, detect_known_addons_path, detect_known_addons_paths,
    quick_check_common_paths, resolve_install_addons_paths, search_for_wow,
    search_for_wow_installs, suggested_scan_roots, validate_addons_path,
};
