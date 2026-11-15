//! The key table: for each state of the app, the keys that work there, what
//! they are called, and which key presses they answer to.
//!
//! The one table feeds the key line (`key_table`) and the pulse on a key
//! press (`footer_hint_for_key`). Help (`help_page`) lists every key of each
//! context; a test presses each Help key in its context and checks that each
//! key line key is in Help, so the two cannot drift apart.

use super::render_dashboard::{RowStatus, WIDE_FRAME_MIN_WIDTH};
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
const TEXT_MOVE: KeyCodes = &[KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End];
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
/// Details closes on enter too, the key that opens it.
const CLOSE_DETAILS: KeyCodes = &[KeyCode::Esc, KeyCode::Char('q'), KeyCode::Enter];
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

    pub(super) fn kept(mut self) -> Self {
        self.kept = true;
        self
    }

    /// What survives when the key line is too narrow: the primary action and
    /// question answers first, then `esc`, then `?`, then the rest.
    fn rank(&self) -> u8 {
        if self.kept || self.main || self.primary || self.key.is_empty() {
            0
        } else if self.key.contains("esc") {
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

/// What a Help row needs before its keys do anything. Help shows the row
/// only then, so it never offers a key that does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpWhen {
    Always,
    /// No Wago.io key is saved: a name search asks for one in place.
    NoWagoKey,
    /// The current addon is "Not linked": `f` finds it online.
    NotLinked,
    /// The welcome card is on the list.
    FirstRunCard,
}

/// The left hand side of a Help row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpLabel {
    /// Keys, as Help prints them.
    Keys(&'static str),
    /// A status word, as the Status column shows it.
    Status(RowStatus),
    /// Something to type into a box, not a key.
    Typed(&'static str),
}

/// One row of Help: the keys, what they do, and the key presses that do it.
/// A test presses each one in its context, so Help can't list a dead key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HelpRow {
    pub(super) label: HelpLabel,
    pub(super) text: &'static str,
    pub(super) keys: &'static [KeyEvent],
    pub(super) when: HelpWhen,
}

impl HelpRow {
    const fn key(label: &'static str, text: &'static str, keys: &'static [KeyEvent]) -> Self {
        Self {
            label: HelpLabel::Keys(label),
            text,
            keys,
            when: HelpWhen::Always,
        }
    }

    const fn status(status: RowStatus, text: &'static str) -> Self {
        Self {
            label: HelpLabel::Status(status),
            text,
            keys: &[],
            when: HelpWhen::Always,
        }
    }

    const fn typed(label: &'static str, text: &'static str) -> Self {
        Self {
            label: HelpLabel::Typed(label),
            text,
            keys: &[],
            when: HelpWhen::Always,
        }
    }

    const fn when(mut self, when: HelpWhen) -> Self {
        self.when = when;
        self
    }
}

/// One group of Help: a title and its rows.
pub(super) struct HelpSection {
    pub(super) title: &'static str,
    pub(super) rows: &'static [HelpRow],
}

/// Help for one context: the sheet title and two columns of groups. A
/// narrow window shows the right column under the left one.
pub(super) struct HelpPage {
    pub(super) title: &'static str,
    pub(super) left: &'static [HelpSection],
    pub(super) right: &'static [HelpSection],
}

impl HelpPage {
    #[cfg(test)]
    pub(super) fn sections(&self) -> impl Iterator<Item = &HelpSection> {
        self.left.iter().chain(self.right.iter())
    }
}

const fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

const fn ch(character: char) -> KeyEvent {
    press(KeyCode::Char(character))
}

const K_MOVE: &[KeyEvent] = &[press(KeyCode::Up), press(KeyCode::Down), ch('j'), ch('k')];
const K_ARROWS: &[KeyEvent] = &[press(KeyCode::Up), press(KeyCode::Down)];
const K_PAGE: &[KeyEvent] = &[press(KeyCode::PageUp), press(KeyCode::PageDown)];
const K_ENDS: &[KeyEvent] = &[press(KeyCode::Home), press(KeyCode::End)];
const K_ENTER: &[KeyEvent] = &[press(KeyCode::Enter)];
const K_ESC: &[KeyEvent] = &[press(KeyCode::Esc)];
const K_CLOSE: &[KeyEvent] = &[press(KeyCode::Esc), ch('q')];
const K_HELP: &[KeyEvent] = &[ch('?')];
const K_FIND: &[KeyEvent] = &[ch('f')];
const K_CTRL_C: &[KeyEvent] = &[KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)];
const K_TEXT_MOVE: &[KeyEvent] = &[
    press(KeyCode::Left),
    press(KeyCode::Right),
    press(KeyCode::Home),
    press(KeyCode::End),
];
const K_TEXT_DELETE: &[KeyEvent] = &[press(KeyCode::Backspace), press(KeyCode::Delete)];
const K_CTRL_U: &[KeyEvent] = &[KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)];

/// The editing keys every text box has, for the sections about typing.
const TEXT_DELETE: HelpRow = HelpRow::key("backspace  del", "delete a letter", K_TEXT_DELETE);
const TEXT_MOVE_ROW: HelpRow = HelpRow::key("← →  Home End", "move in the text", K_TEXT_MOVE);
const TEXT_CLEAR: HelpRow = HelpRow::key("ctrl+u", "clear the text", K_CTRL_U);

const QUESTIONS: HelpSection = HelpSection {
    title: "Questions",
    rows: &[
        HelpRow::key("y", "yes", &[ch('y')]),
        HelpRow::key("n  esc  q", "no", &[ch('n'), press(KeyCode::Esc), ch('q')]),
    ],
};

const QUIT_ANYWHERE: HelpRow = HelpRow::key("ctrl+c", "quit from anywhere", K_CTRL_C);
const THIS_HELP: HelpRow = HelpRow::key("?", "this help", K_HELP);

const STATUS_WORDS: HelpSection = HelpSection {
    title: "What the status means",
    rows: &[
        HelpRow::status(RowStatus::UpdateReady, "a newer version is out"),
        HelpRow::status(RowStatus::UpToDate, "nothing to do"),
        HelpRow::status(RowStatus::NewerThanSource, "yours is newer than the source"),
        HelpRow::status(RowStatus::NotChecked, "linked, but not checked yet"),
        HelpRow::status(
            RowStatus::NotLinked,
            "LemonUp can't update it yet; press f to fix that",
        ),
        HelpRow::status(RowStatus::CheckFailed, "open details to see why"),
    ],
};

static LIST_HELP: HelpPage = HelpPage {
    title: "Help: your addon list",
    left: &[
        HelpSection {
            title: "Your addon list",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "move", K_MOVE),
                HelpRow::key("PgUp PgDn", "move a page", K_PAGE),
                HelpRow::key("Home End", "first, last", K_ENDS),
                HelpRow::key(
                    "→ ←  or  l h",
                    "show, hide an addon's folders",
                    &[
                        press(KeyCode::Right),
                        press(KeyCode::Left),
                        ch('l'),
                        ch('h'),
                    ],
                ),
                HelpRow::key("] [", "show, hide all folders", &[ch(']'), ch('[')]),
                HelpRow::key("enter", "open details", K_ENTER),
                HelpRow::key("/", "filter the list", &[ch('/')]),
                HelpRow::key(
                    "1 2 3 4",
                    "sort by name, status, author, source",
                    &[ch('1'), ch('2'), ch('3'), ch('4')],
                ),
            ],
        },
        HelpSection {
            title: "Selecting",
            rows: &[
                HelpRow::key("space", "select or unselect", &[ch(' ')]),
                HelpRow::key("a", "select all, or none", &[ch('a')]),
                HelpRow::key("esc", "clear the filter, then the selection", K_ESC),
            ],
        },
        HelpSection {
            title: "While you type a filter",
            rows: &[
                HelpRow::key("letters", "narrow the list", &[ch('w')]),
                TEXT_DELETE,
                TEXT_MOVE_ROW,
                TEXT_CLEAR,
                HelpRow::key("↑ ↓", "move through the matches", K_ARROWS),
                HelpRow::key("enter", "keep the filter", K_ENTER),
                HelpRow::key("esc", "clear the filter", K_ESC),
            ],
        },
        STATUS_WORDS,
    ],
    right: &[
        HelpSection {
            title: "Getting things done",
            rows: &[
                HelpRow::key("U", "update every addon that has an update", &[ch('U')]),
                HelpRow::key("u", "update this addon, or the selected ones", &[ch('u')]),
                HelpRow::key("C", "check every addon for updates", &[ch('C')]),
                HelpRow::key("c", "check this addon, or the selected ones", &[ch('c')]),
                HelpRow::key("x", "delete this addon, or the selected ones", &[ch('x')]),
                HelpRow::key("z", "undo the last delete", &[ch('z')]),
                HelpRow::key("f", "find a \"Not linked\" addon online", K_FIND)
                    .when(HelpWhen::NotLinked),
                HelpRow::key("d", "hide the welcome card", &[ch('d')]).when(HelpWhen::FirstRunCard),
            ],
        },
        HelpSection {
            title: "Other screens",
            rows: &[
                HelpRow::key("i", "install an addon", &[ch('i')]),
                HelpRow::key("b", "backups of your game settings", &[ch('b')]),
                HelpRow::key(",", "settings", &[ch(',')]),
                THIS_HELP,
            ],
        },
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key("esc", "go back one step", K_ESC),
                HelpRow::key("q", "quit (from the addon list)", &[ch('q')]),
                QUIT_ANYWHERE,
            ],
        },
    ],
};

static DETAILS_HELP: HelpPage = HelpPage {
    title: "Help: Details",
    left: &[
        HelpSection {
            title: "This addon",
            rows: &[
                HelpRow::key("u", "update it", &[ch('u')]),
                HelpRow::key("c", "check it for updates", &[ch('c')]),
                HelpRow::key("x", "delete it", &[ch('x')]),
                HelpRow::key("space", "select or unselect it", &[ch(' ')]),
                HelpRow::key("f", "find it online, so LemonUp keeps it updated", K_FIND)
                    .when(HelpWhen::NotLinked),
            ],
        },
        HelpSection {
            title: "Moving",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "the previous or next addon", K_MOVE),
                HelpRow::key("PgUp PgDn", "scroll Details", K_PAGE),
                HelpRow::key("Home End", "top, bottom of Details", K_ENDS),
            ],
        },
    ],
    right: &[
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key(
                    "enter  esc  q",
                    "close Details",
                    &[press(KeyCode::Enter), press(KeyCode::Esc), ch('q')],
                ),
                THIS_HELP,
                QUIT_ANYWHERE,
            ],
        },
    ],
};

static DOCKED_DETAILS_HELP: HelpPage = HelpPage {
    title: "Help: Details beside the list",
    left: &[
        HelpSection {
            title: "Details",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "move; Details follows", K_MOVE),
                HelpRow::key("u", "update it, or the selected ones", &[ch('u')]),
                HelpRow::key("c", "check it, or the selected ones", &[ch('c')]),
                HelpRow::key("x", "delete it, or the selected ones", &[ch('x')]),
                HelpRow::key("space", "select or unselect", &[ch(' ')]),
                HelpRow::key("f", "find it online, so LemonUp keeps it updated", K_FIND)
                    .when(HelpWhen::NotLinked),
                HelpRow::key(
                    "enter  esc  q",
                    "close Details",
                    &[press(KeyCode::Enter), press(KeyCode::Esc), ch('q')],
                ),
            ],
        },
        HelpSection {
            title: "The list keeps working",
            rows: &[
                HelpRow::key("PgUp PgDn", "move a page", K_PAGE),
                HelpRow::key("Home End", "first, last", K_ENDS),
                HelpRow::key(
                    "→ ←  or  l h",
                    "show, hide an addon's folders",
                    &[
                        press(KeyCode::Right),
                        press(KeyCode::Left),
                        ch('l'),
                        ch('h'),
                    ],
                ),
                HelpRow::key("] [", "show, hide all folders", &[ch(']'), ch('[')]),
                HelpRow::key("/", "filter the list", &[ch('/')]),
                HelpRow::key(
                    "1 2 3 4",
                    "sort by name, status, author, source",
                    &[ch('1'), ch('2'), ch('3'), ch('4')],
                ),
                HelpRow::key("a", "select all, or none", &[ch('a')]),
            ],
        },
    ],
    right: &[
        HelpSection {
            title: "Getting things done",
            rows: &[
                HelpRow::key("U", "update every addon that has an update", &[ch('U')]),
                HelpRow::key("C", "check every addon for updates", &[ch('C')]),
                HelpRow::key("z", "undo the last delete", &[ch('z')]),
            ],
        },
        HelpSection {
            title: "Other screens",
            rows: &[
                HelpRow::key("i", "install an addon", &[ch('i')]),
                HelpRow::key("b", "backups of your game settings", &[ch('b')]),
                HelpRow::key(",", "settings", &[ch(',')]),
                THIS_HELP,
            ],
        },
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[QUIT_ANYWHERE],
        },
    ],
};

static INSTALL_HELP: HelpPage = HelpPage {
    title: "Help: Install",
    left: &[
        HelpSection {
            title: "Typing in the box",
            rows: &[
                HelpRow::key(
                    "letters",
                    "type a name, or paste a link",
                    &[ch('w'), ch('q')],
                ),
                HelpRow::key("enter", "search Wago.io, or install the link", K_ENTER),
                TEXT_DELETE,
                TEXT_MOVE_ROW,
                TEXT_CLEAR,
                HelpRow::key("esc", "close Install", K_ESC),
            ],
        },
        HelpSection {
            title: "What you can type",
            rows: &[
                HelpRow::typed("a name", "searches Wago.io (needs your Wago.io key)"),
                HelpRow::typed(
                    "a link",
                    "installs from GitHub, WoWInterface, Wago.io or TukUI",
                ),
                HelpRow::typed("elvui  tukui", "installs ElvUI or Tukui from TukUI"),
            ],
        },
        HelpSection {
            title: "Install from a link?",
            rows: &[
                HelpRow::key("enter", "install it", K_ENTER),
                HelpRow::key("esc", "not now", K_ESC),
            ],
        },
        HelpSection {
            title: "Your Wago.io key",
            rows: &[
                HelpRow::key("letters", "type or paste your key", &[ch('w')])
                    .when(HelpWhen::NoWagoKey),
                HelpRow::key("enter", "save the key and search", K_ENTER).when(HelpWhen::NoWagoKey),
                HelpRow::key("esc", "back to the search box", K_ESC).when(HelpWhen::NoWagoKey),
            ],
        },
    ],
    right: &[
        HelpSection {
            title: "Results",
            rows: &[
                HelpRow::key(
                    "↓",
                    "from the box into the results",
                    &[press(KeyCode::Down)],
                ),
                HelpRow::key("↑ ↓", "move; ↑ on the first goes back to the box", K_ARROWS),
                HelpRow::key("enter", "install the one you are on", K_ENTER),
                HelpRow::key("letters", "go back to the box and type", &[ch('w')]),
                HelpRow::key("PgUp PgDn", "scroll", K_PAGE),
                HelpRow::key("Home End", "top, bottom", K_ENDS),
                HelpRow::key("esc", "close Install", K_ESC),
            ],
        },
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key(
                    "?",
                    "this help (in the results, or with the box empty)",
                    K_HELP,
                ),
                QUIT_ANYWHERE,
            ],
        },
    ],
};

static SETTINGS_HELP: HelpPage = HelpPage {
    title: "Help: Settings",
    left: &[
        HelpSection {
            title: "Settings",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "move between settings", K_MOVE),
                HelpRow::key("enter", "switch it on or off, or edit it", K_ENTER),
                HelpRow::key(
                    "← →  or  h l",
                    "change a number or a choice",
                    &[
                        press(KeyCode::Left),
                        press(KeyCode::Right),
                        ch('h'),
                        ch('l'),
                    ],
                ),
                HelpRow::key("e", "edit it", &[ch('e')]),
                HelpRow::key("PgUp PgDn", "scroll", K_PAGE),
                HelpRow::key("Home End", "top, bottom", K_ENDS),
            ],
        },
        HelpSection {
            title: "While you edit a value",
            rows: &[
                HelpRow::key("enter", "save it", K_ENTER),
                HelpRow::key("esc", "cancel", K_ESC),
                TEXT_DELETE,
                TEXT_MOVE_ROW,
                TEXT_CLEAR,
            ],
        },
    ],
    right: &[
        HelpSection {
            title: "More",
            rows: &[
                HelpRow::key("r", "run setup again", &[ch('r')]),
                HelpRow::key("i", "load an addon list from a file", &[ch('i')]),
                HelpRow::key("x", "save your addon list to a file", &[ch('x')]),
            ],
        },
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key("esc  q", "close Settings", K_CLOSE),
                THIS_HELP,
                QUIT_ANYWHERE,
            ],
        },
    ],
};

static BACKUPS_HELP: HelpPage = HelpPage {
    title: "Help: Backups",
    left: &[HelpSection {
        title: "Backups of your game settings",
        rows: &[
            HelpRow::key("↑ ↓  or  j k", "choose a backup", K_MOVE),
            HelpRow::key(
                "enter  r",
                "put this backup back (asks first)",
                &[press(KeyCode::Enter), ch('r')],
            ),
            HelpRow::key("n", "back up your game settings now", &[ch('n')]),
            HelpRow::key("x", "delete this backup (asks first)", &[ch('x')]),
            HelpRow::key("PgUp PgDn", "scroll", K_PAGE),
            HelpRow::key("Home End", "top, bottom", K_ENDS),
        ],
    }],
    right: &[
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key("esc  q", "close Backups", K_CLOSE),
                THIS_HELP,
                QUIT_ANYWHERE,
            ],
        },
    ],
};

/// What Help shows for each context.
pub(super) fn help_page(context: HelpContext) -> &'static HelpPage {
    match context {
        HelpContext::Overview => &LIST_HELP,
        HelpContext::Inspect => &DETAILS_HELP,
        HelpContext::DetailsDocked => &DOCKED_DETAILS_HELP,
        HelpContext::InstallSearch => &INSTALL_HELP,
        HelpContext::Config => &SETTINGS_HELP,
        HelpContext::Backup => &BACKUPS_HELP,
    }
}

impl App {
    /// The keys that work right now, most useful first. The key line shows
    /// them; a key press pulses the entry that answers to it.
    pub(super) fn key_table(&self, width: u16) -> Vec<KeyHint> {
        if self.shell_mode == ShellMode::Onboarding {
            return self.onboarding_key_table();
        }
        if self.active_question().is_some() {
            return self.question_key_table();
        }
        match self.shell_ui.overlay.active {
            Some(OverlayKind::Inspect) => self.inspect_key_table(width),
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

    /// `f`, shown only while the current addon is not linked.
    fn find_online_hint(&self) -> KeyHint {
        KeyHint::primary(
            FooterHintId::FindOnline,
            "f",
            "find online",
            &[KeyCode::Char('f')],
        )
    }

    fn help_hint(&self) -> KeyHint {
        KeyHint::key(FooterHintId::Help, "?", "help", HELP)
    }

    /// The cursor and clear keys of a focused text box. They go last on
    /// the key line, so a narrow one drops them first. Ctrl+U has no key
    /// codes: a plain `u` is a letter in the box and must not pulse it.
    fn text_box_hints(&self) -> [KeyHint; 2] {
        [
            KeyHint::key(FooterHintId::Cursor, "←/→", "cursor", TEXT_MOVE).secondary(),
            KeyHint::key(FooterHintId::ClearText, "ctrl+u", "clear text", &[]).secondary(),
        ]
    }

    fn close_hint(&self) -> KeyHint {
        KeyHint::key(FooterHintId::Close, "esc", "close", CLOSE).secondary()
    }

    fn onboarding_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        if self.onboarding.is_editing {
            let mut hints = vec![
                KeyHint::key(Id::Save, "enter", "apply", ENTER).main(),
                KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                KeyHint::key(Id::Back, "backspace", "delete", BACKSPACE).secondary(),
            ];
            hints.extend(self.text_box_hints());
            return hints;
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

    /// Details, as a sheet or docked beside the list. The keys act on the
    /// addon Details shows; docked, the list keys keep working, so with a
    /// selection `u` `c` `x` act on the selection as they do in the list.
    fn inspect_key_table(&self, width: u16) -> Vec<KeyHint> {
        use FooterHintId as Id;
        let docked = self.details_docked();
        if docked && self.dashboard.filter_editing() {
            return self.dashboard_key_table(width);
        }
        let close = if docked {
            KeyHint::key(Id::Close, "enter or esc", "close details", CLOSE_DETAILS).short("close")
        } else {
            KeyHint::key(Id::Close, "esc", "close", CLOSE_DETAILS)
        };
        let Some(target) = self.inspect_resolved_target() else {
            return vec![close, self.help_hint()];
        };
        let selected = if docked {
            self.dashboard.selected_parent_count()
        } else {
            0
        };
        let mut hints = Vec::new();
        if selected > 0 {
            hints.push(KeyHint::text(format!("{selected} selected:")));
        }
        let status = self.dashboard_row_status(target.item);
        let update = |label| KeyHint::primary(Id::Update, "u", label, &[KeyCode::Char('u')]);
        if selected > 0 {
            hints.push(update("update").main());
        } else if status == RowStatus::UpdateReady && !self.dashboard.update_in_progress() {
            hints.push(update("update"));
        }
        if target.item.source == SourceKind::Manual {
            hints.push(self.find_online_hint().main());
        }
        if selected > 0 || target.item.source != SourceKind::Manual {
            let check = &[KeyCode::Char('c')];
            hints.push(if selected == 0 && status == RowStatus::CheckFailed {
                KeyHint::primary(Id::Check, "c", "try again", check)
            } else {
                KeyHint::key(Id::Check, "c", "check", check)
            });
        }
        hints.push(KeyHint::key(
            Id::Delete,
            "x",
            "delete",
            &[KeyCode::Char('x')],
        ));
        let select_label = if self.dashboard.is_parent_selected(&target.item.folder) {
            "deselect"
        } else {
            "select"
        };
        hints.push(KeyHint::key(
            Id::Select,
            "space",
            select_label,
            &[KeyCode::Char(' ')],
        ));
        if docked {
            hints.push(KeyHint::key(Id::Nav, "↑/↓", "move", UP_DOWN));
        } else {
            hints.push(
                KeyHint::key(Id::Nav, "↑/↓", "previous or next addon", UP_DOWN)
                    .short("other addons"),
            );
            hints.extend(self.sheet_scroll_hint(false));
        }
        hints.push(close);
        hints.push(self.help_hint());
        hints
    }

    fn help_key_table(&self) -> Vec<KeyHint> {
        let mut hints = Vec::new();
        hints.extend(self.sheet_scroll_hint(true));
        hints.push(
            KeyHint::key(
                FooterHintId::Close,
                "esc or ?",
                "close",
                &[KeyCode::Esc, KeyCode::Char('?'), KeyCode::Char('q')],
            )
            .secondary(),
        );
        hints
    }

    /// Install, Config, Backups and the rest: by what the sheet is doing.
    fn sheet_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        match self.dashboard.detail_mode {
            DetailMode::Install => {
                if self.install_pane.is_editing {
                    let mut hints = vec![
                        KeyHint::key(Id::Save, "enter", "apply", ENTER).main(),
                        KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                    ];
                    hints.extend(self.text_box_hints());
                    hints
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
                    let mut hints = vec![
                        KeyHint::key(Id::Save, "enter", "apply", ENTER).main(),
                        KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                        KeyHint::key(Id::Back, "backspace", "delete", BACKSPACE).secondary(),
                    ];
                    hints.extend(self.text_box_hints());
                    return hints;
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
                    KeyHint::key(Id::Run, "r", "setup", &[KeyCode::Char('r')]),
                    KeyHint::key(Id::Install, "i", "import", &[KeyCode::Char('i')]).secondary(),
                    KeyHint::key(Id::Delete, "x", "export", &[KeyCode::Char('x')]).secondary(),
                    self.close_hint(),
                ]);
                hints.extend(self.sheet_scroll_hint(false));
                hints.push(self.help_hint());
                hints
            }
            DetailMode::Backup => {
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

    /// Install: typing always goes to the box, so `q` and letters never
    /// close or move anything, and `esc` is the way out.
    fn search_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        let pane = &self.search_pane;
        let close = |label: &'static str| KeyHint::key(Id::Close, "esc", label, ESC).secondary();
        if pane.pending_direct_install().is_some() {
            return vec![
                KeyHint::key(Id::Confirm, "enter", "install", ENTER)
                    .kept()
                    .main(),
                KeyHint::key(Id::Cancel, "esc", "not now", ESC).kept(),
            ];
        }
        if pane.key_box.is_some() {
            let mut hints = vec![
                KeyHint::key(Id::Save, "enter", "save the key and search", ENTER)
                    .short("save and search")
                    .main(),
                KeyHint::key(Id::Back, "esc", "back", ESC),
            ];
            hints.extend(self.text_box_hints());
            return hints;
        }
        if pane.busy() {
            return vec![
                KeyHint::text(if pane.in_progress {
                    "Searching Wago.io."
                } else {
                    "Installing. This takes a few seconds."
                }),
                close("close"),
            ];
        }
        if pane.results_focused() {
            let mut hints = vec![
                KeyHint::text("type to change the search"),
                KeyHint::key(Id::Results, "↑/↓", "choose", ARROWS),
                KeyHint::key(Id::Install, "enter", "install", ENTER).main(),
                close("close"),
            ];
            hints.extend(self.sheet_scroll_hint(false));
            hints.push(self.help_hint());
            return hints;
        }
        let empty = pane.query.is_empty();
        match &pane.install_state {
            SearchInstallState::Success { .. } if empty => {
                return vec![
                    KeyHint::key(Id::Install, "enter", "install another", ENTER).main(),
                    close("back to my addons"),
                    self.help_hint(),
                ];
            }
            SearchInstallState::Error { .. } => {
                let mut hints = vec![
                    KeyHint::key(Id::Install, "enter", "try again", ENTER).main(),
                    close("close"),
                ];
                if empty {
                    hints.push(self.help_hint());
                }
                return hints;
            }
            _ => {}
        }
        let mut hints = Vec::new();
        if empty {
            hints.push(KeyHint::text("type or paste"));
        }
        match classify_search_input(pane.query.value()) {
            SearchInputIntent::Direct(_) | SearchInputIntent::WagoUrl(_) => {
                hints.push(KeyHint::key(Id::Install, "enter", "install", ENTER).main());
            }
            SearchInputIntent::WagoSearch(_) => {
                hints.push(KeyHint::key(Id::Install, "enter", "search", ENTER).main());
            }
            SearchInputIntent::Empty | SearchInputIntent::InvalidUrl(_) => {}
        }
        if !pane.results.is_empty() {
            hints.push(KeyHint::key(Id::Results, "↓", "results", &[KeyCode::Down]));
        }
        hints.push(close("close"));
        if empty {
            hints.push(self.help_hint());
        } else {
            hints.extend(self.text_box_hints());
        }
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
                hints.extend(self.text_box_hints());
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
            hints.extend(self.text_box_hints());
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
        let settings = KeyHint::key(Id::Config, ",", "settings", &[KeyCode::Char(',')]);
        let quit = KeyHint::key(Id::Quit, "q", "quit", &[KeyCode::Char('q')]);
        let install_main =
            || KeyHint::primary(Id::Install, "i", "install", &[KeyCode::Char('i')]).main();
        // Nothing to list: only the keys that can help.
        if self.dashboard.items.is_empty() {
            return match self.scan_state {
                ScanState::Pending | ScanState::Running(_) => vec![help, quit],
                ScanState::Failed(_) => vec![settings.main(), help, quit],
                _ => vec![install_main(), settings, help, quit],
            };
        }
        if self.dashboard.rows.is_empty() {
            return vec![
                KeyHint::key(Id::Clear, "esc", "clear filter", ESC).main(),
                filter,
                install,
                help,
            ];
        }
        // Nothing is linked yet, so checking and updating have nothing to
        // do; installing is how an addon gets linked.
        if self
            .dashboard
            .items
            .iter()
            .all(|item| item.source == SourceKind::Manual)
        {
            // `f` links the addon under the cursor; installing is for new ones.
            let mut hints = vec![
                self.find_online_hint().main(),
                details,
                KeyHint::key(Id::Install, "i", "install", &[KeyCode::Char('i')]),
            ];
            if wide {
                hints.push(KeyHint::key(Id::Select, "space", "select", SELECT));
            }
            hints.push(filter);
            if self.first_run_card_visible() {
                hints.push(KeyHint::key(
                    Id::HideCard,
                    "d",
                    "hide card",
                    &[KeyCode::Char('d')],
                ));
            }
            if wide {
                hints.push(KeyHint::key(
                    Id::Backup,
                    "b",
                    "backups",
                    &[KeyCode::Char('b')],
                ));
                hints.push(settings);
            }
            hints.push(help);
            return hints;
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
        let find = self
            .not_linked_current_item()
            .map(|_| self.find_online_hint());
        if updates > 0 {
            let update_all = KeyHint::primary(
                Id::UpdateAll,
                "U",
                format!("update all ({updates})"),
                &[KeyCode::Char('U')],
            );
            let mut hints = if wide {
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
            if let Some(find) = find {
                hints.insert(if wide { 4 } else { 3 }, find);
            }
            return hints;
        }
        let check_all = check_all(if checked { "check again" } else { "check all" }).main();
        let mut hints = if wide {
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
        };
        if let Some(find) = find {
            hints.insert(2, find);
        }
        hints
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
