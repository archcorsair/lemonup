use std::path::PathBuf;

use crate::tukui::parse_tukui_target;
use crate::wago::WagoStability;
use crate::wowinterface::parse_wowinterface_target;
use clap::{Parser, Subcommand};
use lemonup_core::DEFAULT_PROFILE;

#[derive(Debug, Parser)]
#[command(name = "lemonup", about = "Rust rewrite of the LemonUp addon manager")]
pub struct Cli {
    #[arg(long, default_value = DEFAULT_PROFILE)]
    pub profile: String,
    #[arg(long)]
    pub addon_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Launch the Ratatui interface
    Tui,
    /// Check tracked addons for updates without applying changes
    Check {
        #[arg(value_name = "ADDON", value_parser = parse_addon_selector)]
        addons: Vec<String>,
    },
    /// Update tracked addons by exact addon or folder name; omit targets to update all
    Update {
        #[arg(value_name = "ADDON", value_parser = parse_addon_selector)]
        addons: Vec<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Update all tracked addons
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        Cli, Commands, parse_addon_selector, parse_tukui_install_target, parse_wago_install_target,
        parse_wowinterface_install_target,
    };

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
}
