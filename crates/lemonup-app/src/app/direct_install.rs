//! Classifies the install/search overlay input and runs non-Wago direct installs.
//!
//! The overlay query box is the single install entry point: GitHub repo URLs, WoWInterface
//! addon pages and the canonical TukUI targets install directly through the same provider
//! functions the `install-github` / `install-wowinterface` / `install-tukui` CLI commands
//! call, while Wago URLs and plain text keep the existing Wago flow.

use std::path::Path;

use lemonup_core::StateDatabase;

use super::{AddonScanOutcome, App, AppAction, DetailMode, StatusKind, sync_dashboard_state};
use crate::github::{install_github_addon, parse_github_target};
use crate::tukui::{install_tukui_addon, parse_tukui_target};
use crate::wago::parse_wago_target;
use crate::wowinterface::{install_wowinterface_addon, parse_wowinterface_target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectInstallProvider {
    Github,
    Wowinterface,
    Tukui,
}

impl DirectInstallProvider {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Wowinterface => "WoWInterface",
            Self::Tukui => "TukUI",
        }
    }
}

/// A non-Wago install target, already normalized by the provider parser the CLI uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectInstallRequest {
    pub(super) provider: DirectInstallProvider,
    /// Exactly what the matching CLI value parser yields: the canonical GitHub repo URL,
    /// the WoWInterface addon id, or the TukUI slug.
    pub(super) target: String,
    /// Short label for the confirmation line, e.g. `owner/repo`.
    pub(super) display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SearchInputIntent {
    Empty,
    /// GitHub, WoWInterface or TukUI: installs without a Wago API key.
    Direct(DirectInstallRequest),
    /// A Wago addon URL, carrying the parsed Wago slug.
    WagoUrl(String),
    /// Anything else: a Wago name search, exactly as before.
    WagoSearch(String),
    /// Looked like an addon URL but no provider accepted it.
    InvalidUrl(String),
}

impl SearchInputIntent {
    pub(super) fn needs_wago_key(&self) -> bool {
        matches!(self, Self::WagoUrl(_) | Self::WagoSearch(_))
    }
}

const UNSUPPORTED_URL_MESSAGE: &str =
    "LemonUp doesn't know that site. It can install from GitHub, WoWInterface, Wago.io and TukUI.";

/// Classifies overlay input in this order: GitHub repo URL, WoWInterface URL, TukUI target,
/// Wago URL; everything else is a Wago name search unless it looks like a URL.
pub(super) fn classify_search_input(input: &str) -> SearchInputIntent {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return SearchInputIntent::Empty;
    }

    let github = parse_github_target(trimmed);
    if let Ok(target) = &github {
        let display = target.trim_start_matches("https://github.com/").to_string();
        return SearchInputIntent::Direct(DirectInstallRequest {
            provider: DirectInstallProvider::Github,
            target: target.clone(),
            display,
        });
    }

    let wowinterface = parse_wowinterface_target(trimmed);
    if let Ok(addon_id) = &wowinterface {
        return SearchInputIntent::Direct(DirectInstallRequest {
            provider: DirectInstallProvider::Wowinterface,
            target: addon_id.clone(),
            display: format!("addon {addon_id}"),
        });
    }

    let tukui = parse_tukui_target(trimmed);
    if let Ok(slug) = &tukui {
        let display = match slug.as_str() {
            "elvui" => "ElvUI",
            "tukui" => "Tukui",
            other => other,
        }
        .to_string();
        return SearchInputIntent::Direct(DirectInstallRequest {
            provider: DirectInstallProvider::Tukui,
            target: slug.clone(),
            display,
        });
    }

    let wago = parse_wago_target(trimmed);
    let has_scheme = trimmed.starts_with("https://") || trimmed.starts_with("http://");
    if has_scheme && let Ok(slug) = &wago {
        return SearchInputIntent::WagoUrl(slug.clone());
    }

    if !looks_like_url(trimmed) {
        return SearchInputIntent::WagoSearch(trimmed.to_string());
    }

    let lowered = trimmed.to_ascii_lowercase();
    let host_error = if lowered.contains("github.com") {
        github.err()
    } else if lowered.contains("wowinterface.com") {
        wowinterface.err()
    } else if lowered.contains("tukui.org") {
        tukui.err()
    } else if lowered.contains("wago.io") {
        match wago {
            Err(error) => Some(error),
            Ok(_) => {
                Some("A Wago.io link looks like https://addons.wago.io/addons/<name>".to_string())
            }
        }
    } else {
        None
    };
    SearchInputIntent::InvalidUrl(host_error.unwrap_or_else(|| UNSUPPORTED_URL_MESSAGE.to_string()))
}

fn looks_like_url(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered.contains("://")
        || lowered.starts_with("www.")
        || ["github.com", "wowinterface.com", "tukui.org", "wago.io"]
            .iter()
            .any(|host| lowered.contains(host))
}

impl App {
    /// Status text when any overlay install (Wago or direct) is already running.
    pub(super) fn install_busy_status(&self) -> Option<&'static str> {
        if self.wago_install_in_progress {
            Some("A Wago.io install is already running")
        } else if self.direct_install_in_progress {
            Some("an addon install is already running")
        } else {
            None
        }
    }

    /// Submit of a direct target: ask for confirmation. Needs no Wago API key.
    pub(super) fn request_direct_install_actions(
        &self,
        request: DirectInstallRequest,
    ) -> Vec<AppAction> {
        if self.dashboard.pending_delete_folders().is_some() {
            return vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                DetailMode::Search,
                "confirm or cancel the pending delete before installing addons",
            ))];
        }
        if self.effective_addon_dir.is_none() {
            return vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Problem,
                DetailMode::Search,
                "addon directory is not configured",
            ))];
        }
        let status = direct_install_prompt(&request);
        vec![
            AppAction::SetSearchPaneState(self.search_pane.request_direct_install(request)),
            AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                DetailMode::Search,
                &status,
            )),
        ]
    }
}

/// The confirmation line, e.g. `Install from GitHub: owner/repo? enter confirm, esc cancel`.
pub(super) fn direct_install_prompt(request: &DirectInstallRequest) -> String {
    format!(
        "Install from {}: {}? enter confirm, esc cancel",
        request.provider.label(),
        request.display
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectInstallOutcome {
    pub(super) addon_name: String,
    pub(super) parent_folder: String,
    pub(super) warning: Option<String>,
    pub(super) sync: AddonScanOutcome,
}

/// Installs through the provider functions the CLI install commands call (not a dry run);
/// those apply the flavor detection and WoW Forever rules for `addon_dir`.
pub(super) async fn run_direct_install_task(
    state_db_file: &Path,
    addon_dir: &Path,
    request: &DirectInstallRequest,
) -> std::result::Result<DirectInstallOutcome, String> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let (addon_name, parent_folder, warning) = match request.provider {
        DirectInstallProvider::Github => {
            let summary =
                install_github_addon(&mut database, addon_dir, &request.target, false).await?;
            (summary.repo_name, summary.parent_folder, None)
        }
        DirectInstallProvider::Wowinterface => {
            let summary =
                install_wowinterface_addon(&mut database, addon_dir, &request.target, false)
                    .await?;
            (summary.addon_name, summary.parent_folder, summary.warning)
        }
        DirectInstallProvider::Tukui => {
            let summary =
                install_tukui_addon(&mut database, addon_dir, &request.target, false).await?;
            (summary.addon_name, summary.parent_folder, None)
        }
    };
    drop(database);
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;
    Ok(DirectInstallOutcome {
        addon_name,
        parent_folder,
        warning,
        sync,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DirectInstallProvider, DirectInstallRequest, SearchInputIntent, classify_search_input,
    };

    fn direct(provider: DirectInstallProvider, target: &str, display: &str) -> SearchInputIntent {
        SearchInputIntent::Direct(DirectInstallRequest {
            provider,
            target: target.to_string(),
            display: display.to_string(),
        })
    }

    #[test]
    fn classifies_github_repo_url_as_direct_github_install() {
        assert_eq!(
            classify_search_input("  https://github.com/WeakAuras/WeakAuras2  "),
            direct(
                DirectInstallProvider::Github,
                "https://github.com/WeakAuras/WeakAuras2",
                "WeakAuras/WeakAuras2",
            )
        );
    }

    #[test]
    fn classifies_wowinterface_url_as_direct_install_by_addon_id() {
        assert_eq!(
            classify_search_input(
                "https://www.wowinterface.com/downloads/info25687-ElvUI_WindTools.html"
            ),
            direct(DirectInstallProvider::Wowinterface, "25687", "addon 25687")
        );
    }

    #[test]
    fn classifies_elvui_and_tukui_keywords_as_tukui_installs() {
        assert_eq!(
            classify_search_input("elvui"),
            direct(DirectInstallProvider::Tukui, "elvui", "ElvUI")
        );
        assert_eq!(
            classify_search_input("ElvUI"),
            direct(DirectInstallProvider::Tukui, "elvui", "ElvUI")
        );
        assert_eq!(
            classify_search_input("tukui"),
            direct(DirectInstallProvider::Tukui, "tukui", "Tukui")
        );
    }

    #[test]
    fn classifies_wago_url_as_wago_install_with_slug() {
        assert_eq!(
            classify_search_input("https://addons.wago.io/addons/details"),
            SearchInputIntent::WagoUrl("details".to_string())
        );
    }

    #[test]
    fn classifies_plain_text_as_wago_search() {
        assert_eq!(
            classify_search_input("Details Damage Meter"),
            SearchInputIntent::WagoSearch("Details Damage Meter".to_string())
        );
        assert_eq!(
            classify_search_input("details"),
            SearchInputIntent::WagoSearch("details".to_string())
        );
        assert_eq!(classify_search_input("   "), SearchInputIntent::Empty);
    }

    #[test]
    fn classifies_unparsable_urls_as_invalid_with_provider_error() {
        match classify_search_input("https://github.com/WeakAuras") {
            SearchInputIntent::InvalidUrl(message) => {
                assert!(message.contains("github.com/<owner>/<repo>"), "{message}")
            }
            other => panic!("expected invalid url, got {other:?}"),
        }
        match classify_search_input("https://example.com/addon") {
            SearchInputIntent::InvalidUrl(message) => {
                assert!(
                    message.starts_with("LemonUp doesn't know that site"),
                    "{message}"
                )
            }
            other => panic!("expected invalid url, got {other:?}"),
        }
        assert!(matches!(
            classify_search_input("github.com/WeakAuras/WeakAuras2"),
            SearchInputIntent::InvalidUrl(_)
        ));
    }

    #[test]
    fn only_wago_intents_need_the_wago_key() {
        assert!(classify_search_input("details").needs_wago_key());
        assert!(classify_search_input("https://addons.wago.io/addons/details").needs_wago_key());
        assert!(!classify_search_input("elvui").needs_wago_key());
        assert!(!classify_search_input("https://github.com/owner/repo").needs_wago_key());
        assert!(!classify_search_input("https://example.com/addon").needs_wago_key());
    }
}
