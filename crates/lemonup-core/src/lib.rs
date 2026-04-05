pub mod config;
pub mod domain;
pub mod error;
pub mod events;
pub mod paths;
pub mod scan;
pub mod storage;
pub mod transfer;
pub mod wow;

pub use config::{AppConfig, ConfigLoad, ConfigStore, DefaultScreen, ThemeMode};
pub use domain::{
    AddonKind, AddonRecord, GameFlavor, InstallPlan, InstallSource, OwnedFolder, OwnershipSource,
    SourceKind, UpdateCheck, UpdateStatus,
};
pub use error::{LemonupError, Result};
pub use events::{OperationKind, OperationProgress, OperationStage};
pub use paths::{AppPaths, DEFAULT_PROFILE, paths_match};
pub use scan::{
    ScanSummary, ScannedAddon, TocMetadata, TocSelectionConfidence, TocSelectionResult,
    parse_toc_content, scan_addons_dir, select_toc_file,
};
pub use storage::StateDatabase;
pub use transfer::{
    DEFAULT_TRANSFER_FILE_NAME, ImportAnalysis, TransferAddon, TransferFile, TransferSourceKind,
    analyze_import, default_transfer_path, export_addons, parse_import_file,
};
pub use wow::{
    ScanProgressUpdate, detect_known_addons_path, quick_check_common_paths, search_for_wow,
    suggested_scan_roots, validate_addons_path,
};
