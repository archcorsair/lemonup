use std::path::PathBuf;

use crate::wago::WagoStability;
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
    /// Run the basic non-interactive updater path
    Update {
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

#[cfg(test)]
mod tests {
    use super::{parse_addon_selector, parse_wago_install_target};

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
}
