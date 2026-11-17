//! Classifies the install/search overlay input and runs non-Wago direct installs.
//!
//! The overlay query box is the single install entry point: GitHub repo URLs, WoWInterface
//! addon pages and the canonical TukUI targets install directly through the same provider
//! functions the `install-github` / `install-wowinterface` / `install-tukui` CLI commands
//! call, while Wago URLs and plain text keep the existing Wago flow.

use std::path::Path;

use lemonup_core::StateDatabase;

use lemonup_core::SourceKind;

use super::{
    AddonScanOutcome, App, AppAction, DashboardItem, DetailMode, PendingWagoInstallRequest,
    SearchPaneState, StatusKind, WagoInstallSource, sync_dashboard_state,
};
use crate::github::{install_github_addon_checked, parse_github_target};
use crate::source_error::SourceError;
use crate::tukui::{install_tukui_addon_checked, parse_tukui_target};
use crate::wago::parse_wago_target;
use crate::wowinterface::{install_wowinterface_addon_checked, parse_wowinterface_target};

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

impl DirectInstallRequest {
    /// The addon's name as far as the link tells: the repo for GitHub.
    pub(super) fn name(&self) -> String {
        match self.provider {
            DirectInstallProvider::Github => self
                .display
                .rsplit('/')
                .next()
                .unwrap_or(&self.display)
                .to_string(),
            DirectInstallProvider::Wowinterface | DirectInstallProvider::Tukui => {
                self.display.clone()
            }
        }
    }
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
    /// Whether an install from the Install sheet (Wago.io or direct) is running.
    pub(super) fn install_running(&self) -> bool {
        self.wago_install_in_progress || self.direct_install_in_progress
    }

    /// Why an install can't start now: another install, or an update,
    /// delete or undo that is changing the AddOns folder.
    pub(super) fn install_busy_status(&self) -> Option<&'static str> {
        if self.wago_install_in_progress {
            Some("A Wago.io install is already running")
        } else if self.direct_install_in_progress {
            Some("an addon install is already running")
        } else if self.dashboard.update_in_progress() {
            Some("Wait for the update to finish first.")
        } else if self.delete_in_progress {
            Some("Wait for the delete to finish first.")
        } else if self.undo_delete_in_progress {
            Some("Wait for the undo to finish first.")
        } else if self.toc_apply_running {
            Some("Wait for the .toc change to finish first.")
        } else {
            None
        }
    }

    /// Why an update, delete or undo can't start now: an install, or a
    /// .toc change, is running.
    pub(super) fn install_running_status(&self) -> Option<&'static str> {
        if self.install_running() {
            Some("Wait for the install to finish first.")
        } else if self.toc_apply_running {
            Some("Wait for the .toc change to finish first.")
        } else {
            None
        }
    }

    /// The addon `f` finds online: the current one, when it is not linked.
    pub(super) fn not_linked_current_item(&self) -> Option<&DashboardItem> {
        self.dashboard
            .selected_item()
            .filter(|item| item.source == SourceKind::Manual)
    }

    /// What enter does with the text in `pane`'s box: ask about a link,
    /// search Wago.io, or open the key box when a search needs a key and
    /// none is saved. An empty box after an install starts over.
    pub(super) fn search_submit_actions(
        &self,
        pane: SearchPaneState,
        api_key: Option<String>,
    ) -> Vec<AppAction> {
        let intent = classify_search_input(pane.query.value());
        if pane.in_progress {
            return vec![AppAction::SetSearchPaneState(pane)];
        }
        if let Some(busy) = self.install_busy_status() {
            return vec![
                AppAction::SetSearchPaneState(pane),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Search,
                    busy,
                )),
            ];
        }
        match intent {
            SearchInputIntent::Empty => {
                vec![AppAction::SetSearchPaneState(pane.clear_install_state())]
            }
            // The line under the box already says why nothing happens.
            SearchInputIntent::InvalidUrl(_) => vec![AppAction::SetSearchPaneState(pane)],
            SearchInputIntent::Direct(request) => {
                self.request_direct_install_actions(pane, request)
            }
            intent if intent.needs_wago_key() && api_key.is_none() => {
                vec![AppAction::SetSearchPaneState(pane.open_key_box())]
            }
            _ if self.dashboard.pending_delete_folders().is_some() => vec![
                AppAction::SetSearchPaneState(pane),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Search,
                    "Answer the delete question first",
                )),
            ],
            SearchInputIntent::WagoUrl(target) => {
                let Some(addon_dir) = self.effective_addon_dir.clone() else {
                    return vec![
                        AppAction::SetSearchPaneState(pane),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Search,
                            "LemonUp doesn't know your AddOns folder yet",
                        )),
                    ];
                };
                let name = pane.query.value().trim().to_string();
                let request = PendingWagoInstallRequest {
                    target: target.clone(),
                    source: WagoInstallSource::DirectInput,
                };
                vec![
                    AppAction::SetSearchPaneState(pane.start_direct_install_check(&target, &name)),
                    AppAction::SetWagoInstallInProgress(true),
                    AppAction::StartWagoInstall {
                        addon_dir,
                        api_key: api_key.expect("checked above"),
                        request,
                        replace_folders: None,
                    },
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Working,
                        DetailMode::Search,
                        &format!("Installing {name} from Wago.io"),
                    )),
                ]
            }
            SearchInputIntent::WagoSearch(query) => vec![
                AppAction::SetSearchPaneState(pane.start_search()),
                AppAction::StartWagoSearch {
                    query: query.clone(),
                    api_key: api_key.expect("checked above"),
                },
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Search,
                    &format!("Searching Wago.io for \"{query}\""),
                )),
            ],
        }
    }

    /// Submit of a direct target: ask once, by the addon's name. Needs no Wago API key.
    pub(super) fn request_direct_install_actions(
        &self,
        pane: SearchPaneState,
        request: DirectInstallRequest,
    ) -> Vec<AppAction> {
        if self.dashboard.pending_delete_folders().is_some() {
            return vec![
                AppAction::SetSearchPaneState(pane),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Search,
                    "Answer the delete question first",
                )),
            ];
        }
        if self.effective_addon_dir.is_none() {
            return vec![
                AppAction::SetSearchPaneState(pane),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Problem,
                    DetailMode::Search,
                    "LemonUp doesn't know your AddOns folder yet",
                )),
            ];
        }
        vec![AppAction::SetSearchPaneState(
            pane.request_direct_install(request),
        )]
    }

    /// Enter on "Install X from GitHub?" (`replace_folders` empty), or yes to
    /// replacing the folders the question listed. `name` is what the status says.
    pub(super) fn start_direct_install_actions(
        &self,
        request: DirectInstallRequest,
        replace_folders: Vec<String>,
        name: String,
    ) -> Vec<AppAction> {
        if let Some(busy) = self.install_busy_status() {
            return vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                DetailMode::Search,
                busy,
            ))];
        }
        if self.dashboard.pending_delete_folders().is_some() {
            return vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                DetailMode::Search,
                "Answer the delete question first",
            ))];
        }
        let Some(addon_dir) = self.effective_addon_dir.clone() else {
            return vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Problem,
                DetailMode::Search,
                "LemonUp doesn't know your AddOns folder yet",
            ))];
        };
        let verb = if replace_folders.is_empty() {
            "Installing"
        } else {
            "Replacing your copy of"
        };
        vec![
            AppAction::SetSearchPaneState(self.search_pane.start_direct_install(request.clone())),
            AppAction::SetDirectInstallInProgress(true),
            AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Working,
                DetailMode::Search,
                &format!("{verb} {name} from {}", request.provider.label()),
            )),
            AppAction::StartDirectInstall {
                addon_dir,
                request,
                replace_folders,
            },
        ]
    }

    /// Enter in the Wago.io key box: saves the key the way Settings does,
    /// then runs the search that asked for it. The key is never shown.
    pub(super) fn save_wago_key_actions(&self) -> Vec<AppAction> {
        let Some(key) = self
            .search_pane
            .key_box
            .as_ref()
            .map(|key| key.value().trim().to_string())
            .filter(|key| !key.is_empty())
        else {
            return vec![];
        };
        let mut next = self.config_pane.clone();
        next.draft.wago_api_key = Some(key.clone());
        if let Err(error) = self.config_store.write_config(&next.draft) {
            return vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Problem,
                DetailMode::Search,
                &format!("Couldn't save your Wago.io key: {error}"),
            ))];
        }
        let mut actions = vec![
            AppAction::SetPersistedConfig(next.draft.clone()),
            AppAction::SetConfigPaneState(next.mark_saved(next.draft.clone())),
            AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Done,
                DetailMode::Search,
                "Saved your Wago.io key",
            )),
        ];
        let pane = self.search_pane.close_key_box().clear_install_state();
        actions.extend(self.search_submit_actions(pane, Some(key)));
        actions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectInstallOutcome {
    pub(super) addon_name: String,
    pub(super) parent_folder: String,
    pub(super) warning: Option<String>,
    pub(super) sync: AddonScanOutcome,
}

/// Installs through the provider functions the CLI install commands call (not a dry run);
/// those apply the flavor detection and WoW Forever rules for `addon_dir`. Folders already in
/// AddOns (a "Not linked" copy, say) on `replace_folders` are taken over like an update; any
/// other folder in the way comes back as `SourceErrorKind::FoldersInTheWay` (asked again), and
/// a folder of a linked addon as `SourceErrorKind::FoldersOfLinkedAddon` (never replaced).
pub(super) async fn run_direct_install_task(
    state_db_file: &Path,
    addon_dir: &Path,
    request: &DirectInstallRequest,
    replace_folders: &[String],
) -> std::result::Result<DirectInstallOutcome, SourceError> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let (addon_name, parent_folder, warning) = match request.provider {
        DirectInstallProvider::Github => {
            let summary = install_github_addon_checked(
                &mut database,
                addon_dir,
                &request.target,
                false,
                replace_folders,
            )
            .await?;
            (summary.repo_name, summary.parent_folder, None)
        }
        DirectInstallProvider::Wowinterface => {
            let summary = install_wowinterface_addon_checked(
                &mut database,
                addon_dir,
                &request.target,
                false,
                replace_folders,
            )
            .await?;
            (summary.addon_name, summary.parent_folder, summary.warning)
        }
        DirectInstallProvider::Tukui => {
            let summary = install_tukui_addon_checked(
                &mut database,
                addon_dir,
                &request.target,
                false,
                replace_folders,
            )
            .await?;
            (summary.addon_name, summary.parent_folder, None)
        }
    };
    drop(database);
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;
    // The installed addon's own name (its title) reads better than a repo slug.
    let addon_name = installed_addon_name(&sync, &parent_folder).unwrap_or(addon_name);
    Ok(DirectInstallOutcome {
        addon_name,
        parent_folder,
        warning,
        sync,
    })
}

/// The name the addon list shows for the addon installed at `parent_folder`.
pub(super) fn installed_addon_name(sync: &AddonScanOutcome, parent_folder: &str) -> Option<String> {
    sync.addons
        .iter()
        .find(|addon| addon.folder == parent_folder)
        .map(|addon| addon.name.trim().to_string())
        .filter(|name| !name.is_empty())
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
