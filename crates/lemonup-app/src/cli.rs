use std::path::PathBuf;

use crate::github::parse_github_target;
use crate::tukui::parse_tukui_target;
use crate::wago::WagoStability;
use crate::wowinterface::parse_wowinterface_target;
use clap::{Parser, Subcommand};
use lemonup_core::{DEFAULT_PROFILE, TocField};

#[derive(Debug, Parser)]
#[command(
    name = "lemonup",
    about = "World of Warcraft addon manager",
    version,
    disable_version_flag = true
)]
pub struct Cli {
    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    version: (),
    #[arg(long, default_value = DEFAULT_PROFILE)]
    pub profile: String,
    #[arg(long)]
    pub addon_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Open the LemonUp TUI (the default)
    Tui,
    /// Scan the AddOns folder and update LemonUp's list
    Sync,
    /// Check linked addons for updates without changing anything
    Check {
        #[arg(value_name = "ADDON", value_parser = parse_addon_selector)]
        addons: Vec<String>,
    },
    /// Update linked addons by exact addon or folder name; omit names to update all
    Update {
        #[arg(value_name = "ADDON", value_parser = parse_addon_selector)]
        addons: Vec<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Update all linked addons
    UpdateAll {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Install a Wago addon by slug or URL
    InstallWago {
        #[arg(value_name = "WAGO_ADDON", value_parser = parse_wago_install_target)]
        addon: String,
        #[arg(long, value_enum, default_value_t = WagoStability::Stable)]
        stability: WagoStability,
        #[arg(long)]
        dry_run: bool,
    },
    /// Install the canonical TukUI-hosted ElvUI or Tukui package
    InstallTukui {
        #[arg(value_name = "TUKUI_ADDON", value_parser = parse_tukui_install_target)]
        addon: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Install a WoWInterface addon by addon page URL
    InstallWowinterface {
        #[arg(value_name = "WOWI_ADDON", value_parser = parse_wowinterface_install_target)]
        addon: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Install a GitHub-hosted addon by repo URL
    InstallGithub {
        #[arg(value_name = "GITHUB_REPO", value_parser = parse_github_install_target)]
        addon: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Export your addon list to a JSON file
    ExportAddons {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Import an addon list from a JSON file and install what's missing
    ImportAddons {
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Restore a backup of your game settings
    RestoreBackup {
        #[arg(value_name = "BACKUP_ZIP")]
        archive: Option<PathBuf>,
    },
    /// Find where your Not linked addons come from (Wago.io, WoWInterface, TukUI); changes nothing
    Find {
        /// Link the exact matches to their source; nothing is downloaded
        #[arg(long)]
        link: bool,
        /// With --link, also link exact matches that take over folders of other addons
        #[arg(long, requires = "link")]
        yes: bool,
        /// Print one key=value line per result, for scripts
        #[arg(long)]
        plain: bool,
    },
    /// Show an addon's Title, Notes and Category, or change one in its .toc files
    Toc {
        /// The addon's exact name or folder name
        #[arg(value_name = "ADDON", value_parser = parse_addon_selector)]
        addon: String,
        /// The field to change: title, notes or category
        #[arg(value_name = "FIELD", value_parser = parse_toc_field)]
        field: Option<TocField>,
        /// The new value
        #[arg(
            value_name = "VALUE",
            requires = "field",
            conflicts_with = "clear",
            allow_hyphen_values = true
        )]
        value: Option<String>,
        /// Give the field back the addon's own value
        #[arg(long, requires = "field")]
        clear: bool,
        /// Change the files of an addon LemonUp didn't install (Not linked)
        #[arg(long)]
        yes: bool,
    },
}

fn parse_addon_selector(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("addon selector cannot be empty".to_string());
    }

    if trimmed.len() > 120 {
        return Err("addon selector is too long".to_string());
    }

    if trimmed.contains("..")
        || trimmed.contains(['\\', '/', ':', '<', '>', '"', '|', '?', '*', '`'])
    {
        return Err("addon selector contains forbidden characters".to_string());
    }

    if !trimmed.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, ' ' | '_' | '-' | '.' | '!' | '\'' | '(' | ')')
    }) {
        return Err("addon selector contains unsupported characters".to_string());
    }

    Ok(trimmed.to_string())
}

/// `title`, `notes` or `category`, in any case.
fn parse_toc_field(value: &str) -> Result<TocField, String> {
    TocField::ALL
        .into_iter()
        .find(|field| field.name().eq_ignore_ascii_case(value.trim()))
        .ok_or_else(|| format!("unknown field '{value}'; use title, notes or category"))
}

fn parse_wago_install_target(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Wago addon target cannot be empty".to_string());
    }

    if trimmed.len() > 200 {
        return Err("Wago addon target is too long".to_string());
    }

    if trimmed.chars().any(|character| character.is_control()) {
        return Err("Wago addon target contains forbidden characters".to_string());
    }

    Ok(trimmed.to_string())
}

fn parse_tukui_install_target(value: &str) -> Result<String, String> {
    parse_tukui_target(value)
}

fn parse_wowinterface_install_target(value: &str) -> Result<String, String> {
    parse_wowinterface_target(value)
}

fn parse_github_install_target(value: &str) -> Result<String, String> {
    parse_github_target(value)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use lemonup_core::TocField;

    use super::{
        Cli, Commands, parse_addon_selector, parse_github_install_target,
        parse_tukui_install_target, parse_wago_install_target, parse_wowinterface_install_target,
    };

    #[test]
    fn version_flags_display_version() {
        for flag in ["--version", "-v"] {
            let err = Cli::try_parse_from(["lemonup", flag]).expect_err("version flag exits");
            assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn addon_selector_accepts_expected_characters() {
        let value = parse_addon_selector("DBM-Core").expect("valid selector");
        assert_eq!(value, "DBM-Core");

        let value = parse_addon_selector("Details! Damage Meter").expect("valid name");
        assert_eq!(value, "Details! Damage Meter");
    }

    #[test]
    fn addon_selector_rejects_path_like_or_empty_input() {
        assert!(parse_addon_selector("").is_err());
        assert!(parse_addon_selector("  ").is_err());
        assert!(parse_addon_selector("..\\secret").is_err());
        assert!(parse_addon_selector("../secret").is_err());
        assert!(parse_addon_selector("DBM-Core; rm -rf").is_err());
    }

    #[test]
    fn wago_target_rejects_empty_or_control_chars() {
        assert!(parse_wago_install_target("").is_err());
        assert!(parse_wago_install_target("  ").is_err());
        assert!(parse_wago_install_target("detail\nbreak").is_err());
        assert!(parse_wago_install_target("details").is_ok());
        assert!(parse_wago_install_target("https://addons.wago.io/addons/details").is_ok());
    }

    #[test]
    fn tukui_target_accepts_only_canonical_targets() {
        assert_eq!(parse_tukui_install_target("ElvUI").expect("elvui"), "elvui");
        assert_eq!(parse_tukui_install_target("tukui").expect("tukui"), "tukui");
        assert!(parse_tukui_install_target("Details").is_err());
    }

    #[test]
    fn update_command_accepts_multiple_exact_selectors() {
        let cli = Cli::try_parse_from(["lemonup", "update", "WeakAuras", "DBM-Core", "--dry-run"])
            .expect("parse update selectors");

        match cli.command.expect("subcommand") {
            Commands::Update {
                addons,
                force,
                dry_run,
            } => {
                assert_eq!(addons, vec!["WeakAuras", "DBM-Core"]);
                assert!(!force);
                assert!(dry_run);
            }
            other => panic!("expected update command, got {other:?}"),
        }
    }

    #[test]
    fn update_all_command_parses_as_distinct_alias() {
        let cli =
            Cli::try_parse_from(["lemonup", "update-all", "--force"]).expect("parse update-all");

        match cli.command.expect("subcommand") {
            Commands::UpdateAll { force, dry_run } => {
                assert!(force);
                assert!(!dry_run);
            }
            other => panic!("expected update-all command, got {other:?}"),
        }
    }

    #[test]
    fn sync_command_parses() {
        let cli = Cli::try_parse_from(["lemonup", "sync"]).expect("parse sync");

        match cli.command.expect("subcommand") {
            Commands::Sync => {}
            other => panic!("expected sync command, got {other:?}"),
        }
    }

    #[test]
    fn install_tukui_command_parses_canonical_target() {
        let cli = Cli::try_parse_from(["lemonup", "install-tukui", "ElvUI", "--dry-run"])
            .expect("parse install-tukui");

        match cli.command.expect("subcommand") {
            Commands::InstallTukui { addon, dry_run } => {
                assert_eq!(addon, "elvui");
                assert!(dry_run);
            }
            other => panic!("expected install-tukui command, got {other:?}"),
        }
    }

    #[test]
    fn export_and_import_commands_parse() {
        let export = Cli::try_parse_from(["lemonup", "export-addons"]).expect("export parse");
        assert!(matches!(
            export.command.expect("subcommand"),
            Commands::ExportAddons { output: None }
        ));

        let import =
            Cli::try_parse_from(["lemonup", "import-addons", "--dry-run"]).expect("import parse");
        match import.command.expect("subcommand") {
            Commands::ImportAddons { input, dry_run } => {
                assert!(input.is_none());
                assert!(dry_run);
            }
            other => panic!("expected import-addons command, got {other:?}"),
        }
    }

    #[test]
    fn wowinterface_target_accepts_expected_url_only() {
        assert_eq!(
            parse_wowinterface_install_target(
                "https://www.wowinterface.com/downloads/info25687-ElvUI_WindTools.html"
            )
            .expect("wowi url"),
            "25687"
        );
        assert!(parse_wowinterface_install_target("25687").is_err());
        assert!(parse_wowinterface_install_target("https://example.com/info25687.html").is_err());
    }

    #[test]
    fn install_wowinterface_command_parses_url() {
        let cli = Cli::try_parse_from([
            "lemonup",
            "install-wowinterface",
            "https://www.wowinterface.com/downloads/info25687-ElvUI_WindTools.html",
            "--dry-run",
        ])
        .expect("parse install-wowinterface");

        match cli.command.expect("subcommand") {
            Commands::InstallWowinterface { addon, dry_run } => {
                assert_eq!(addon, "25687");
                assert!(dry_run);
            }
            other => panic!("expected install-wowinterface command, got {other:?}"),
        }
    }

    #[test]
    fn github_target_accepts_repo_url_only() {
        assert_eq!(
            parse_github_install_target("https://github.com/WeakAuras/WeakAuras2")
                .expect("github url"),
            "https://github.com/WeakAuras/WeakAuras2"
        );
        assert!(parse_github_install_target("WeakAuras/WeakAuras2").is_err());
        assert!(parse_github_install_target("https://example.com/owner/repo").is_err());
    }

    #[test]
    fn install_github_command_parses_repo_url() {
        let cli = Cli::try_parse_from([
            "lemonup",
            "install-github",
            "https://github.com/WeakAuras/WeakAuras2",
            "--dry-run",
        ])
        .expect("parse install-github");

        match cli.command.expect("subcommand") {
            Commands::InstallGithub { addon, dry_run } => {
                assert_eq!(addon, "https://github.com/WeakAuras/WeakAuras2");
                assert!(dry_run);
            }
            other => panic!("expected install-github command, got {other:?}"),
        }
    }

    #[test]
    fn find_command_parses_and_yes_needs_link() {
        let cli = Cli::try_parse_from(["lemonup", "find"]).expect("find parses");
        assert!(matches!(
            cli.command,
            Some(Commands::Find {
                link: false,
                yes: false,
                plain: false
            })
        ));
        let cli = Cli::try_parse_from(["lemonup", "find", "--link", "--yes"]).expect("link parses");
        assert!(matches!(
            cli.command,
            Some(Commands::Find {
                link: true,
                yes: true,
                plain: false
            })
        ));
        let cli = Cli::try_parse_from(["lemonup", "find", "--plain"]).expect("plain parses");
        assert!(matches!(
            cli.command,
            Some(Commands::Find {
                link: false,
                yes: false,
                plain: true
            })
        ));
        assert!(Cli::try_parse_from(["lemonup", "find", "--yes"]).is_err());
    }

    #[test]
    fn toc_command_shows_sets_and_clears_a_field() {
        let show = Cli::try_parse_from(["lemonup", "toc", "AdiBags"]).expect("show");
        assert!(matches!(
            show.command.expect("subcommand"),
            Commands::Toc { addon, field: None, value: None, clear: false, yes: false }
                if addon == "AdiBags"
        ));
        let set =
            Cli::try_parse_from(["lemonup", "toc", "AdiBags", "CATEGORY", "My Bags", "--yes"])
                .expect("set");
        assert!(matches!(
            set.command.expect("subcommand"),
            Commands::Toc { field: Some(TocField::Category), value: Some(value), clear: false, yes: true, .. }
                if value == "My Bags"
        ));
        let clear =
            Cli::try_parse_from(["lemonup", "toc", "AdiBags", "title", "--clear"]).expect("clear");
        assert!(matches!(
            clear.command.expect("subcommand"),
            Commands::Toc {
                field: Some(TocField::Title),
                value: None,
                clear: true,
                ..
            }
        ));
    }

    #[test]
    fn toc_command_rejects_an_unknown_field_and_a_value_with_clear() {
        let error = Cli::try_parse_from(["lemonup", "toc", "AdiBags", "version", "2"])
            .expect_err("unknown field");
        assert!(
            error.to_string().contains("use title, notes or category"),
            "{error}"
        );
        assert!(
            Cli::try_parse_from(["lemonup", "toc", "AdiBags", "notes", "x", "--clear"]).is_err()
        );
    }
}
