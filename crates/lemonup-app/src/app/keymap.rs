//! The key table: for each state of the app, the keys that work there, what
//! they are called, and which key presses they answer to.
//!
//! The one table feeds the key line (`key_table`), the pulse on a key press
//! (`footer_hint_for_key`) and Help (`help_sections`), so they cannot drift
//! apart.

use super::render_dashboard::WIDE_FRAME_MIN_WIDTH;
use super::*;

/// Key presses one entry answers to.
pub(super) type KeyCodes = &'static [KeyCode];

const UP_DOWN: KeyCodes = &[
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Char('j'),
    KeyCode::Char('k'),
];
const ARROWS: KeyCodes = &[KeyCode::Up, KeyCode::Down];
const LEFT_RIGHT: KeyCodes = &[KeyCode::Left, KeyCode::Right];
const CHANGE: KeyCodes = &[
    KeyCode::Left,
    KeyCode::Right,
    KeyCode::Char('h'),
    KeyCode::Char('l'),
];
const PAGE: KeyCodes = &[
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Home,
    KeyCode::End,
];
const SCROLL: KeyCodes = &[
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Char('j'),
    KeyCode::Char('k'),
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Home,
    KeyCode::End,
];
const ENTER: KeyCodes = &[KeyCode::Enter];
const ESC: KeyCodes = &[KeyCode::Esc];
const BACKSPACE: KeyCodes = &[KeyCode::Backspace];
const CLOSE: KeyCodes = &[KeyCode::Esc, KeyCode::Char('q')];
const NO: KeyCodes = &[KeyCode::Char('n'), KeyCode::Esc, KeyCode::Char('q')];
const HELP: KeyCodes = &[KeyCode::Char('?')];
const SELECT: KeyCodes = &[KeyCode::Char(' '), KeyCode::Char('a')];

/// One entry of the key table, as the key line shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeyHint {
    /// The pulse target; `None` for a plain label such as "2 selected:".
    pub(super) id: Option<FooterHintId>,
    pub(super) key: &'static str,
    pub(super) label: String,
    /// The main action, drawn in lemon.
    pub(super) primary: bool,
    /// The key presses that run it.
    pub(super) codes: KeyCodes,
    /// Setup's two row footer puts secondary keys on the second row.
    pub(super) tier: FooterHintTier,
    /// A shorter label for a key line that is tight on room.
    pub(super) short: Option<&'static str>,
    /// The screen's primary action: the last entry to drop.
    pub(super) main: bool,
    /// Never dropped: the answers to a question.
    pub(super) kept: bool,
}

impl KeyHint {
    pub(super) fn key(
        id: FooterHintId,
        key: &'static str,
        label: impl Into<String>,
        codes: KeyCodes,
    ) -> Self {
        Self {
            id: Some(id),
            key,
            label: label.into(),
            primary: false,
            codes,
            tier: FooterHintTier::Primary,
            short: None,
            main: false,
            kept: false,
        }
    }

    fn primary(
        id: FooterHintId,
        key: &'static str,
        label: impl Into<String>,
        codes: KeyCodes,
    ) -> Self {
        Self {
            primary: true,
            ..Self::key(id, key, label, codes)
        }
    }

    pub(super) fn text(label: impl Into<String>) -> Self {
        Self {
            id: None,
            key: "",
            label: label.into(),
            primary: false,
            codes: &[],
            tier: FooterHintTier::Primary,
            short: None,
            main: false,
            kept: false,
        }
    }

    fn short(mut self, label: &'static str) -> Self {
        self.short = Some(label);
        self
    }

    fn main(mut self) -> Self {
        self.main = true;
        self
    }

    fn kept(mut self) -> Self {
        self.kept = true;
        self
    }

    /// What survives when the key line is too narrow: the primary action and
    /// question answers first, then `esc`, then `?`, then the rest.
    fn rank(&self) -> u8 {
        if self.kept || self.main || self.primary || self.key.is_empty() {
            0
        } else if self.key.starts_with("esc") || self.key == "n/esc" {
            1
        } else if self.key == "?" {
            2
        } else {
            3
        }
    }

    fn secondary(mut self) -> Self {
        self.tier = FooterHintTier::Secondary;
        self
    }

    pub(super) fn width(&self) -> usize {
        if self.key.is_empty() {
            self.label.chars().count()
        } else {
            self.key.chars().count() + 1 + self.label.chars().count()
        }
    }

    fn answers(&self, code: KeyCode) -> bool {
        self.id.is_some() && self.codes.contains(&code)
    }
}

/// One group of Help: a title and (keys, what they do) rows.
pub(super) struct HelpSection {
    pub(super) title: &'static str,
    pub(super) rows: &'static [(&'static str, &'static str)],
}

/// What Help lists for each context.
pub(super) fn help_sections(context: HelpContext) -> &'static [HelpSection] {
    match context {
        HelpContext::Overview => &[
            HelpSection {
                title: " Movement ",
                rows: &[
                    ("↑/↓", "Move through addons"),
                    ("enter", "Open inspect"),
                    ("/", "Filter installed addons (enter keeps it)"),
                    ("esc", "Clear the filter, then the selection"),
                ],
            },
            HelpSection {
                title: " Actions ",
                rows: &[
                    ("space / a", "Select or deselect addons"),
                    ("c / u / x", "Check, update, or delete"),
                ],
            },
            HelpSection {
                title: " More ",
                rows: &[("i", "Open install and search"), ("1-4", "Sort the table")],
            },
        ],
        HelpContext::Inspect => &[
            HelpSection {
                title: " Actions ",
                rows: &[
                    ("space", "Select or deselect this addon"),
                    ("c / u / x", "Check, update, or delete"),
                ],
            },
            HelpSection {
                title: " Sections ",
                rows: &[
                    ("r / d / t", "Toggle included addons, deps, and addon info"),
                    ("↑/↓", "Scroll details when content overflows"),
                ],
            },
        ],
        HelpContext::InstallSearch => &[
            HelpSection {
                title: " Query ",
                rows: &[
                    ("enter", "Search Wago by name or install a pasted URL"),
                    ("elvui / tukui", "Install ElvUI or Tukui from TukUI"),
                    ("/", "Edit the query again"),
                ],
            },
            HelpSection {
                title: " Direct install ",
                rows: &[(
                    "enter / esc",
                    "Confirm or cancel a GitHub, WoWInterface or TukUI install",
                )],
            },
            HelpSection {
                title: " Results ",
                rows: &[
                    ("↑/↓", "Move through search results"),
                    ("enter", "Install the selected result"),
                ],
            },
            HelpSection {
                title: " Setup ",
                rows: &[(",", "Open Config for your Wago API key (Wago only)")],
            },
        ],
        HelpContext::Config => &[
            HelpSection {
                title: " Fields ",
                rows: &[
                    ("↑/↓", "Move between settings"),
                    ("enter", "Edit text or toggle the current field"),
                ],
            },
            HelpSection {
                title: " Adjust ",
                rows: &[
                    ("←/→", "Change booleans and numeric values"),
                    ("r", "Run onboarding again"),
                ],
            },
            HelpSection {
                title: " Transfer ",
                rows: &[("i", "Import addon list"), ("x", "Export addon list")],
            },
        ],
        HelpContext::Backup => &[
            HelpSection {
                title: " Backups ",
                rows: &[("↑/↓", "Choose a backup"), ("n", "Create a new backup now")],
            },
            HelpSection {
                title: " Restore ",
                rows: &[("enter / r", "Restore the selected backup")],
            },
        ],
    }
}

impl App {
    /// The keys that work right now, most useful first. The key line shows
    /// them; a key press pulses the entry that answers to it.
    pub(super) fn key_table(&self, width: u16) -> Vec<KeyHint> {
        if self.shell_mode == ShellMode::Onboarding {
            return self.onboarding_key_table();
        }
        if self.dashboard.pending_delete_folders().is_some()
            || self.pending_wago_install_confirmation.is_some()
        {
            return vec![
                KeyHint::key(FooterHintId::Confirm, "y", "confirm", &[KeyCode::Char('y')]).kept(),
                KeyHint::key(FooterHintId::Cancel, "n/esc", "cancel", NO).kept(),
            ];
        }
        match self.shell_ui.overlay.active {
            Some(OverlayKind::Inspect) => self.inspect_key_table(),
            Some(OverlayKind::Help) => self.help_key_table(),
            Some(_) => self.sheet_key_table(),
            None => self.dashboard_key_table(width),
        }
    }

    /// The table entry a key press answers to, for the pulse on the key line.
    pub(super) fn footer_hint_for_key(&self, key: KeyEvent) -> Option<FooterHintId> {
        if is_ctrl_c(key) || is_modified_char_chord(key) {
            return None;
        }
        // Labels change with the width, the keys do not: the wide table
        // holds every entry a narrower one does.
        self.key_table(u16::MAX)
            .into_iter()
            .find(|hint| hint.answers(key.code))
            .and_then(|hint| hint.id)
    }

    /// "↑/↓ scroll" (or "pgup/pgdn scroll" where the arrows move a cursor),
    /// only while the open sheet has more than fits.
    fn sheet_scroll_hint(&self, arrows: bool) -> Option<KeyHint> {
        if !self
            .sheet_scroll
            .get()
            .overflows(self.shell_ui.overlay.active)
        {
            return None;
        }
        Some(if arrows {
            KeyHint::key(FooterHintId::Scroll, "↑/↓", "scroll", SCROLL)
        } else {
            KeyHint::key(FooterHintId::Scroll, "pgup/pgdn", "scroll", PAGE)
        })
    }

    fn help_hint(&self) -> KeyHint {
        KeyHint::key(FooterHintId::Help, "?", "help", HELP)
    }

    fn close_hint(&self) -> KeyHint {
        KeyHint::key(FooterHintId::Close, "esc", "close", CLOSE).secondary()
    }

    fn onboarding_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        if self.onboarding.is_editing {
            return vec![
                KeyHint::key(Id::Save, "enter", "apply", ENTER).main(),
                KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                KeyHint::key(Id::Back, "backspace", "delete", BACKSPACE).secondary(),
            ];
        }
        match self.onboarding.step {
            OnboardingStep::Theme => vec![
                KeyHint::key(Id::ThemeToggle, "←/→", "theme", LEFT_RIGHT),
                KeyHint::key(Id::Next, "enter", "next", ENTER),
                KeyHint::key(Id::Cancel, "esc", "exit", ESC).secondary(),
            ],
            OnboardingStep::Directory => match self.onboarding.phase {
                OnboardingPhase::Found(_) => vec![
                    KeyHint::key(Id::Nav, "j/k", "choice", UP_DOWN),
                    KeyHint::key(Id::Next, "enter", "next", ENTER),
                    KeyHint::key(Id::Back, "esc", "back", ESC).secondary(),
                ],
                OnboardingPhase::DeepScanning(_) => {
                    vec![KeyHint::key(Id::Cancel, "esc", "stop scan", ESC)]
                }
                _ => vec![
                    KeyHint::key(Id::Nav, "j/k", "choice", UP_DOWN),
                    KeyHint::key(
                        Id::Validate,
                        "enter",
                        "search",
                        &[KeyCode::Enter, KeyCode::Char('d')],
                    ),
                    KeyHint::key(Id::Edit, "e", "type path", &[KeyCode::Char('e')]),
                    KeyHint::key(Id::Back, "esc", "back", ESC).secondary(),
                ],
            },
            OnboardingStep::Wago => vec![
                KeyHint::key(Id::Edit, "e", "edit key", &[KeyCode::Char('e')]),
                KeyHint::key(Id::Next, "enter", "next", ENTER),
                KeyHint::key(Id::Back, "esc", "back", ESC).secondary(),
            ],
            OnboardingStep::Settings => vec![
                KeyHint::key(Id::SettingsNav, "j/k", "settings", UP_DOWN),
                KeyHint::key(Id::Change, "←/→", "change", CHANGE),
                KeyHint::key(Id::Next, "enter", "next", ENTER),
                KeyHint::key(Id::Back, "esc", "back", ESC).secondary(),
            ],
            OnboardingStep::Review => vec![
                KeyHint::key(Id::Next, "enter", "finish", ENTER),
                KeyHint::key(Id::Back, "esc", "back", ESC),
            ],
        }
    }

    fn inspect_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        let Some(target) = self.inspect_resolved_target() else {
            return vec![
                KeyHint::key(Id::Close, "esc", "close", CLOSE),
                self.help_hint(),
            ];
        };
        let select_label = if self.dashboard.is_parent_selected(&target.item.folder) {
            "deselect"
        } else {
            "select"
        };
        let mut hints = vec![
            KeyHint::key(Id::Select, "space", select_label, &[KeyCode::Char(' ')]),
            KeyHint::key(Id::Check, "c", "check", &[KeyCode::Char('c')]),
        ];
        if is_refreshable_dashboard_item(target.item) {
            hints.push(KeyHint::key(Id::Update, "u", "update", &[KeyCode::Char('u')]).main());
        }
        hints.extend([
            KeyHint::key(Id::Delete, "x", "delete", &[KeyCode::Char('x')]),
            KeyHint::key(Id::Relations, "r", "relations", &[KeyCode::Char('r')])
                .short("rel")
                .secondary(),
            KeyHint::key(Id::Dependencies, "d", "deps", &[KeyCode::Char('d')]).secondary(),
            KeyHint::key(Id::Technical, "t", "tech", &[KeyCode::Char('t')]).secondary(),
        ]);
        hints.extend(self.sheet_scroll_hint(true));
        hints.push(self.help_hint());
        hints
    }

    fn help_key_table(&self) -> Vec<KeyHint> {
        let mut hints = Vec::new();
        hints.extend(self.sheet_scroll_hint(true));
        hints.push(KeyHint::key(FooterHintId::Help, "?", "close", HELP));
        hints.push(self.close_hint());
        hints
    }

    /// Install, Config, Backups and the rest: by what the sheet is doing.
    fn sheet_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        match self.dashboard.detail_mode {
            DetailMode::Install => {
                if self.install_pane.is_editing {
                    vec![
                        KeyHint::key(Id::Save, "enter", "apply", ENTER).main(),
                        KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                    ]
                } else {
                    let mut hints = vec![
                        KeyHint::key(Id::Edit, "e", "edit", &[KeyCode::Char('e')]),
                        KeyHint::key(Id::Run, "enter", "install", ENTER).main(),
                        self.close_hint(),
                    ];
                    hints.extend(self.sheet_scroll_hint(true));
                    hints.push(self.help_hint());
                    hints
                }
            }
            DetailMode::Search => self.search_key_table(),
            DetailMode::Update => vec![
                KeyHint::key(
                    Id::Update,
                    "u",
                    "update",
                    &[KeyCode::Char('u'), KeyCode::Char('r')],
                ),
                KeyHint::key(
                    Id::Check,
                    "c",
                    "check",
                    &[KeyCode::Char('c'), KeyCode::Char('v')],
                ),
                self.close_hint(),
            ],
            DetailMode::Config => {
                if self.config_pane.edit.is_some() {
                    return vec![
                        KeyHint::key(Id::Save, "enter", "apply", ENTER).main(),
                        KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                        KeyHint::key(Id::Back, "backspace", "delete", BACKSPACE).secondary(),
                    ];
                }
                let field = self.config_pane.selected_field();
                let mut hints = vec![
                    KeyHint::key(Id::Fields, "↑/↓", "fields", UP_DOWN),
                    KeyHint::key(
                        Id::Toggle,
                        "enter",
                        if field.is_textual() { "edit" } else { "toggle" },
                        ENTER,
                    )
                    .main(),
                ];
                if field.supports_quick_adjust() {
                    hints.push(KeyHint::key(Id::Change, "←/→", "change", CHANGE));
                }
                hints.extend([
                    KeyHint::key(Id::Run, "r", "onboarding", &[KeyCode::Char('r')]).short("setup"),
                    KeyHint::key(Id::Install, "i", "import", &[KeyCode::Char('i')]).secondary(),
                    KeyHint::key(Id::Delete, "x", "export", &[KeyCode::Char('x')]).secondary(),
                    self.close_hint(),
                ]);
                hints.extend(self.sheet_scroll_hint(false));
                hints.push(self.help_hint());
                hints
            }
            DetailMode::Backup => {
                if self.backup_pane.pending_restore.is_some() {
                    return vec![
                        KeyHint::key(Id::Confirm, "y", "restore", &[KeyCode::Char('y')]).kept(),
                        KeyHint::key(Id::Cancel, "n", "cancel", NO)
                            .secondary()
                            .kept(),
                    ];
                }
                if self.backup_pane.pending_delete.is_some() {
                    return vec![
                        KeyHint::key(Id::Confirm, "y", "delete", &[KeyCode::Char('y')]).kept(),
                        KeyHint::key(Id::Cancel, "n", "cancel", NO)
                            .secondary()
                            .kept(),
                    ];
                }
                let selected = self.backup_pane.selected_backup().is_some();
                let busy = self.backup_pane.in_progress;
                let mut hints = Vec::new();
                if !self.backup_pane.backups.is_empty() {
                    hints.push(KeyHint::key(Id::Nav, "↑/↓", "backups", UP_DOWN));
                }
                if selected {
                    hints.push(
                        KeyHint::key(
                            Id::Confirm,
                            "enter/r",
                            "restore",
                            &[KeyCode::Enter, KeyCode::Char('r')],
                        )
                        .main(),
                    );
                }
                if !busy {
                    hints.push(
                        KeyHint::key(Id::Run, "n", "backup", &[KeyCode::Char('n')]).secondary(),
                    );
                }
                if selected && !busy {
                    hints.push(
                        KeyHint::key(Id::Delete, "x", "delete", &[KeyCode::Char('x')]).secondary(),
                    );
                }
                hints.push(self.close_hint());
                hints.extend(self.sheet_scroll_hint(false));
                hints.push(self.help_hint());
                hints
            }
            DetailMode::Overview => vec![KeyHint::key(Id::Close, "esc", "close", CLOSE)],
        }
    }

    fn search_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        if self.search_pane.pending_direct_install().is_some() {
            return vec![
                KeyHint::key(Id::Confirm, "enter", "install", ENTER).kept(),
                KeyHint::key(Id::Cancel, "esc", "cancel", CLOSE).kept(),
            ];
        }
        let intent = classify_search_input(&self.search_pane.query);
        let submit_label = match intent {
            SearchInputIntent::Direct(_) | SearchInputIntent::WagoUrl(_) => "install",
            _ => "search",
        };
        let config = (self.wago_api_key.is_none())
            .then(|| KeyHint::key(Id::Config, ",", "config", &[KeyCode::Char(',')]));
        if self.search_pane.is_editing {
            let mut hints = Vec::new();
            if !self.search_pane.in_progress {
                hints.push(KeyHint::key(Id::Save, "enter", submit_label, ENTER).main());
            }
            hints.push(KeyHint::key(Id::Cancel, "esc", "cancel", ESC));
            hints.push(KeyHint::key(Id::Back, "backspace", "delete", BACKSPACE).secondary());
            hints.extend(config);
            return hints;
        }
        let mut hints = Vec::new();
        if self.search_pane.results.is_empty() {
            let can_submit = self.wago_api_key.is_some() || !intent.needs_wago_key();
            if can_submit && !self.search_pane.in_progress {
                hints.push(KeyHint::key(Id::Install, "enter", submit_label, ENTER).main());
            }
            hints.push(KeyHint::key(Id::Edit, "/", "query", &[KeyCode::Char('/')]));
            hints.extend(config);
            hints.push(self.close_hint());
            hints.extend(self.sheet_scroll_hint(true));
        } else {
            hints.push(KeyHint::key(Id::Results, "↑/↓", "results", UP_DOWN));
            hints.push(KeyHint::key(Id::Install, "enter", "install", ENTER).main());
            hints.push(KeyHint::key(Id::Edit, "/", "query", &[KeyCode::Char('/')]));
            hints.extend(config);
            hints.push(self.close_hint());
            hints.extend(self.sheet_scroll_hint(false));
        }
        hints.push(self.help_hint());
        hints
    }

    /// The addon list: the keys for exactly what is on screen.
    fn dashboard_key_table(&self, width: u16) -> Vec<KeyHint> {
        use FooterHintId as Id;
        let wide = width >= WIDE_FRAME_MIN_WIDTH;
        let help = self.help_hint();
        let details = KeyHint::key(Id::Inspect, "enter", "details", ENTER);
        let filter = KeyHint::key(Id::Filter, "/", "filter", &[KeyCode::Char('/')]);
        let install = KeyHint::key(Id::Install, "i", "install", &[KeyCode::Char('i')]);
        let update = KeyHint::key(Id::Update, "u", "update", &[KeyCode::Char('u')]);
        let check = KeyHint::key(Id::Check, "c", "check", &[KeyCode::Char('c')]);
        if self.dashboard.filter_editing() {
            let mut hints = vec![KeyHint::text("type to filter")];
            if self.dashboard.rows.is_empty() {
                hints.push(KeyHint::key(Id::Clear, "esc", "clear filter", ESC));
                return hints;
            }
            hints.extend([
                KeyHint::key(Id::Nav, "↑/↓", "move", ARROWS),
                KeyHint::key(
                    Id::Save,
                    "enter",
                    if wide { "keep filter" } else { "keep" },
                    ENTER,
                ),
                KeyHint::key(
                    Id::Clear,
                    "esc",
                    if wide { "clear filter" } else { "clear" },
                    ESC,
                ),
            ]);
            return hints;
        }
        if self
            .dashboard
            .job_ui()
            .is_some_and(|job| job.kind == DashboardJobKind::Update)
        {
            return vec![
                KeyHint::text("LemonUp is updating. You can keep browsing."),
                details,
                filter,
                help,
                KeyHint::key(Id::Quit, "q", "quit", &[KeyCode::Char('q')]),
            ];
        }
        let selected = self.dashboard.selected_parent_count();
        if selected > 0 {
            // `a` clears a selection rather than selecting all, and `esc`
            // clears a filter before the selection, so the hints say so.
            let esc = if self.dashboard.has_filter() {
                KeyHint::key(Id::Clear, "esc", "clear filter", ESC).short("clear")
            } else {
                KeyHint::key(Id::Clear, "esc", "clear selection", ESC).short("clear")
            };
            let select_label = if self
                .dashboard
                .selected_row()
                .is_some_and(|row| self.dashboard.is_parent_selected(&row.folder))
            {
                "deselect"
            } else {
                "select"
            };
            return vec![
                KeyHint::text(format!("{selected} selected:")),
                update.main(),
                check,
                KeyHint::key(Id::Delete, "x", "delete", &[KeyCode::Char('x')]),
                KeyHint::key(Id::Select, "space", select_label, SELECT),
                esc,
                help,
            ];
        }
        let updates = self.dashboard.update_ready_folders().len();
        let checked = self
            .dashboard
            .items
            .iter()
            .any(|item| item.last_checked_at.is_some());
        let check_all =
            |label: &'static str| KeyHint::key(Id::CheckAll, "C", label, &[KeyCode::Char('C')]);
        let select = KeyHint::key(Id::Select, "space", "select", SELECT);
        if updates > 0 {
            let update_all = KeyHint::primary(
                Id::UpdateAll,
                "U",
                format!("update all ({updates})"),
                &[KeyCode::Char('U')],
            );
            return if wide {
                vec![
                    update_all,
                    update,
                    check,
                    details,
                    select,
                    filter,
                    install,
                    help,
                    check_all("check all"),
                ]
            } else {
                vec![
                    update_all,
                    update,
                    details,
                    install,
                    filter,
                    help,
                    check_all("check all"),
                ]
            };
        }
        let check_all = check_all(if checked { "check again" } else { "check all" }).main();
        if wide {
            vec![
                check_all,
                details,
                select,
                filter,
                install,
                KeyHint::key(Id::Backup, "b", "backups", &[KeyCode::Char('b')]),
                KeyHint::key(Id::Config, ",", "settings", &[KeyCode::Char(',')]),
                help,
            ]
        } else {
            vec![check_all, details, install, filter, help]
        }
    }
}

/// Fits the key line into `width` columns, after the leading space. It
/// tightens first (two space separators, then short labels), then drops whole
/// entries: the rest from the end, then `?`, then `esc`; the primary action
/// and question answers go last. The kept entries stay in table order.
pub(super) fn fit_key_line(
    mut hints: Vec<KeyHint>,
    width: u16,
    wide: bool,
) -> (Vec<KeyHint>, &'static str) {
    let room = usize::from(width).saturating_sub(1);
    let needed = |hints: &[KeyHint], separator: &str| {
        hints.iter().map(KeyHint::width).sum::<usize>()
            + separator.len() * hints.len().saturating_sub(1)
    };
    if wide && needed(&hints, "   ") <= room {
        return (hints, "   ");
    }
    let separator = "  ";
    if needed(&hints, separator) <= room {
        return (hints, separator);
    }
    for hint in &mut hints {
        if let Some(short) = hint.short {
            hint.label = short.to_string();
        }
    }
    for rank in [3, 2, 1, 0] {
        while needed(&hints, separator) > room && hints.len() > 1 {
            let Some(index) = hints
                .iter()
                .rposition(|hint| hint.rank() == rank && !hint.kept)
            else {
                break;
            };
            hints.remove(index);
        }
    }
    (hints, separator)
}
