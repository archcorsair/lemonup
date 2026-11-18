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
/// Settings opens on `s`, and on `,` as before.
const SETTINGS: KeyCodes = &[KeyCode::Char('s'), KeyCode::Char(',')];
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
    /// A Wago.io key is saved: only then does a key that uses it do anything.
    HasWagoKey,
    /// The current addon is "Not linked": `f` finds it online.
    NotLinked,
    /// The welcome card is on the list.
    FirstRunCard,
    /// No check or update is running: only then do the keys that start one
    /// do anything.
    NoJob,
    /// Docked Details does not fit beside the list: PgUp and PgDn scroll it.
    DetailsOverflow,
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
/// Home and End, and `gg` and `G` for the same. A test presses `g` twice.
const K_ENDS_VIM: &[KeyEvent] = &[press(KeyCode::Home), press(KeyCode::End), ch('g'), ch('G')];
const K_ENTER: &[KeyEvent] = &[press(KeyCode::Enter)];
const K_ESC: &[KeyEvent] = &[press(KeyCode::Esc)];
const K_CLOSE: &[KeyEvent] = &[press(KeyCode::Esc), ch('q')];
const K_HELP: &[KeyEvent] = &[ch('?')];
const K_FIND: &[KeyEvent] = &[ch('f')];
const K_FIND_ALL: &[KeyEvent] = &[ch('F')];
const K_EDIT: &[KeyEvent] = &[ch('e')];
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
                HelpRow::key("Home End  or  gg G", "first, last", K_ENDS_VIM),
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
                HelpRow::key("U", "update every addon that has an update", &[ch('U')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("u", "update this addon, or the selected ones", &[ch('u')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("C", "check every addon for updates", &[ch('C')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("c", "check this addon, or the selected ones", &[ch('c')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("x", "delete this addon, or the selected ones", &[ch('x')]),
                HelpRow::key("z", "undo the last delete", &[ch('z')]),
                HelpRow::key(
                    "f",
                    "find the \"Not linked\" addon under the cursor online",
                    K_FIND,
                )
                .when(HelpWhen::NotLinked),
                HelpRow::key(
                    "F",
                    "find your addons online, or the selected ones",
                    K_FIND_ALL,
                ),
                HelpRow::key("d", "hide the welcome card", &[ch('d')]).when(HelpWhen::FirstRunCard),
            ],
        },
        HelpSection {
            title: "Other screens",
            rows: &[
                HelpRow::key("i", "install an addon", &[ch('i')]),
                HelpRow::key("b", "backups of your game settings", &[ch('b')]),
                HelpRow::key("p", "switch game", &[ch('p')]),
                HelpRow::key("s  or  ,", "settings", &[ch('s'), ch(',')]),
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
                HelpRow::key("u", "update it", &[ch('u')]).when(HelpWhen::NoJob),
                HelpRow::key("c", "check it for updates", &[ch('c')]).when(HelpWhen::NoJob),
                HelpRow::key("x", "delete it", &[ch('x')]),
                HelpRow::key("space", "select or unselect it", &[ch(' ')]),
                HelpRow::key("e", "edit its .toc fields: Title, Notes, Category", K_EDIT),
                HelpRow::key("f", "find it online, so LemonUp keeps it updated", K_FIND)
                    .when(HelpWhen::NotLinked),
            ],
        },
        HelpSection {
            title: "Moving",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "the previous or next addon", K_MOVE),
                HelpRow::key("PgUp PgDn", "scroll Details", K_PAGE),
                HelpRow::key("Home End  or  gg G", "top, bottom of Details", K_ENDS_VIM),
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
                HelpRow::key("u", "update it, or the selected ones", &[ch('u')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("c", "check it, or the selected ones", &[ch('c')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("x", "delete it, or the selected ones", &[ch('x')]),
                HelpRow::key("space", "select or unselect", &[ch(' ')]),
                HelpRow::key("e", "edit its .toc fields: Title, Notes, Category", K_EDIT),
                HelpRow::key(
                    "f",
                    "find this one online, so LemonUp keeps it updated",
                    K_FIND,
                )
                .when(HelpWhen::NotLinked),
                HelpRow::key("F", "find your addons online", K_FIND_ALL),
                HelpRow::key("PgUp PgDn", "scroll Details", K_PAGE).when(HelpWhen::DetailsOverflow),
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
                HelpRow::key("Home End  or  gg G", "first, last", K_ENDS_VIM),
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
                HelpRow::key("U", "update every addon that has an update", &[ch('U')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("C", "check every addon for updates", &[ch('C')])
                    .when(HelpWhen::NoJob),
                HelpRow::key("z", "undo the last delete", &[ch('z')]),
            ],
        },
        HelpSection {
            title: "Other screens",
            rows: &[
                HelpRow::key("i", "install an addon", &[ch('i')]),
                HelpRow::key("b", "backups of your game settings", &[ch('b')]),
                HelpRow::key("p", "switch game", &[ch('p')]),
                HelpRow::key("s  or  ,", "settings", &[ch('s'), ch(',')]),
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
                HelpRow::key("↑ ↓  or  j k", "choose a setting", K_MOVE),
                HelpRow::key(
                    "← →  or  h l",
                    "change it",
                    &[
                        press(KeyCode::Left),
                        press(KeyCode::Right),
                        ch('h'),
                        ch('l'),
                    ],
                ),
                HelpRow::key(
                    "enter",
                    "switch it on or off, type your Wago.io key, or run a row such as \
                     \"Change game folder…\"",
                    K_ENTER,
                ),
                HelpRow::key("PgUp PgDn", "scroll", K_PAGE),
                HelpRow::key("Home End  or  gg G", "top, bottom", K_ENDS_VIM),
            ],
        },
        HelpSection {
            title: "While you type your key",
            rows: &[
                HelpRow::key("enter", "save it", K_ENTER),
                HelpRow::key("esc", "keep the one you had", K_ESC),
                TEXT_DELETE,
                TEXT_MOVE_ROW,
                TEXT_CLEAR,
            ],
        },
    ],
    right: &[HelpSection {
        title: "Everywhere",
        rows: &[
            HelpRow::key("esc  q", "close Settings", K_CLOSE),
            THIS_HELP,
            QUIT_ANYWHERE,
        ],
    }],
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
            HelpRow::key("b", "back up your game settings now", &[ch('b')]),
            HelpRow::key("x", "delete this backup (asks first)", &[ch('x')]),
            HelpRow::key("PgUp PgDn", "scroll", K_PAGE),
            HelpRow::key("Home End  or  gg G", "top, bottom", K_ENDS_VIM),
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

static TOC_EDIT_HELP: HelpPage = HelpPage {
    title: "Help: Edit .toc fields",
    left: &[
        HelpSection {
            title: "Edit .toc fields",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "choose Title, Notes or Category", K_MOVE),
                HelpRow::key("enter", "edit it", K_ENTER),
                HelpRow::key("PgUp PgDn", "scroll", K_PAGE),
                HelpRow::key("Home End  or  gg G", "top, bottom", K_ENDS_VIM),
            ],
        },
        HelpSection {
            title: "While you edit a field",
            rows: &[
                HelpRow::key(
                    "enter",
                    "save it and write it to the addon's files",
                    K_ENTER,
                ),
                HelpRow::key("esc", "keep what you had", K_ESC),
                HelpRow::key(
                    "tab",
                    "fill in the next suggestion (Category)",
                    &[press(KeyCode::Tab)],
                ),
                TEXT_DELETE,
                TEXT_MOVE_ROW,
                TEXT_CLEAR,
                HelpRow::typed("an empty box", "gives back the addon's own value"),
            ],
        },
    ],
    right: &[
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key("esc  q", "close, back to Details", K_CLOSE),
                THIS_HELP,
                QUIT_ANYWHERE,
            ],
        },
    ],
};

static GAMES_HELP: HelpPage = HelpPage {
    title: "Help: Switch game",
    left: &[HelpSection {
        title: "Switch game",
        rows: &[
            HelpRow::key("↑ ↓  or  j k", "choose a game", K_MOVE),
            HelpRow::key("enter", "open this game, or add another game", K_ENTER),
            HelpRow::key(
                "x",
                "remove this game from LemonUp (asks first)",
                &[ch('x')],
            ),
        ],
    }],
    right: &[
        QUESTIONS,
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key("esc  q", "close Switch game", K_CLOSE),
                THIS_HELP,
                QUIT_ANYWHERE,
            ],
        },
    ],
};

static FIND_HELP: HelpPage = HelpPage {
    title: "Help: Find your addons online",
    left: &[
        HelpSection {
            title: "Find your addons online",
            rows: &[
                HelpRow::key("↑ ↓  or  j k", "move; past the last row, scroll", K_MOVE),
                HelpRow::key(
                    "PgUp PgDn",
                    "move a page; past the last row, scroll",
                    K_PAGE,
                ),
                HelpRow::key(
                    "Home End  or  gg G",
                    "first and last row, top and bottom of the report",
                    K_ENDS_VIM,
                ),
                HelpRow::key(
                    "space",
                    "select or unselect a match, or its replacement",
                    &[ch(' ')],
                ),
            ],
        },
        HelpSection {
            title: "Sources",
            rows: &[
                HelpRow::key(
                    "1",
                    "Wago.io on or off; sends folder names and file hashes",
                    &[ch('1')],
                )
                .when(HelpWhen::HasWagoKey),
                HelpRow::key("2", "WoWInterface on or off", &[ch('2')]),
                HelpRow::key("3", "TukUI on or off", &[ch('3')]),
            ],
        },
        HelpSection {
            title: "Picking and linking",
            rows: &[HelpRow::key(
                "enter",
                "pick in a \"Choose one\" row; on Link, ask first",
                K_ENTER,
            )],
        },
    ],
    right: &[
        QUESTIONS,
        HelpSection {
            title: "When the question lists many folders",
            rows: &[HelpRow::key(
                "↑ ↓  PgUp PgDn",
                "scroll the folders",
                &[
                    press(KeyCode::Up),
                    press(KeyCode::Down),
                    ch('j'),
                    ch('k'),
                    press(KeyCode::PageUp),
                    press(KeyCode::PageDown),
                ],
            )],
        },
        HelpSection {
            title: "Everywhere",
            rows: &[
                HelpRow::key("esc  q", "close; stops a lookup, nothing changes", K_CLOSE),
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
        HelpContext::TocEdit => &TOC_EDIT_HELP,
        HelpContext::Games => &GAMES_HELP,
        HelpContext::Find => &FIND_HELP,
    }
}

impl App {
    /// The keys that work right now, most useful first. The key line shows
    /// them; a key press pulses the entry that answers to it.
    pub(super) fn key_table(&self, width: u16) -> Vec<KeyHint> {
        if self.active_question().is_some() {
            return self.question_key_table();
        }
        if self.shell_mode == ShellMode::Onboarding {
            return self.onboarding_key_table();
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
    /// With addons selected, `u`, `c` and `x` act on all of them but `f` only
    /// on the one under the cursor, so the label says so.
    fn find_online_hint(&self, selected: usize) -> KeyHint {
        if selected > 0 {
            return KeyHint::primary(
                FooterHintId::FindOnline,
                "f",
                "find this one online",
                &[KeyCode::Char('f')],
            )
            .short("find this one");
        }
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
            KeyHint::key(FooterHintId::Cursor, "←/→", "cursor", TEXT_MOVE),
            KeyHint::key(FooterHintId::ClearText, "ctrl+u", "clear text", &[]),
        ]
    }

    fn close_hint(&self) -> KeyHint {
        KeyHint::key(FooterHintId::Close, "esc", "close", CLOSE)
    }

    /// Setup's keys, as the mockups name them. A text box takes typing, so
    /// its line has no letters.
    fn onboarding_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        let choose = || KeyHint::key(Id::Nav, "↑/↓", "choose", UP_DOWN);
        match self.onboarding.step {
            OnboardingStep::FindGame => match &self.onboarding.phase {
                phase if phase.is_searching() => vec![
                    KeyHint::key(
                        Id::Cancel,
                        "esc",
                        "stop looking and choose the folder myself",
                        ESC,
                    )
                    .short("stop looking")
                    .main(),
                ],
                OnboardingPhase::Typing(_) => vec![
                    KeyHint::key(Id::Validate, "enter", "check this folder", ENTER)
                        .short("check")
                        .main(),
                    KeyHint::key(Id::Back, "esc", "back", ESC),
                ],
                _ => {
                    let mut hints = vec![choose()];
                    if !self.setup_choice_is_blocked() {
                        hints.push(KeyHint::key(Id::Next, "enter", "continue", ENTER).main());
                    }
                    hints.push(KeyHint::key(Id::Cancel, "esc", "quit setup", ESC));
                    hints
                }
            },
            OnboardingStep::Wago => vec![
                KeyHint::key(Id::Next, "enter", "continue", ENTER).main(),
                KeyHint::key(Id::Back, "esc", "back", ESC),
            ],
            OnboardingStep::Preferences => vec![
                choose(),
                KeyHint::key(Id::Change, "←/→", "change", CHANGE),
                KeyHint::key(Id::Next, "enter", "finish and show my addons", ENTER)
                    .short("finish")
                    .main(),
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
        // While a check or update runs, `u` and `c` would start nothing.
        let idle = self.dashboard_job_running().is_none();
        if idle && selected > 0 {
            hints.push(update("update").main());
        } else if idle && status == RowStatus::UpdateReady {
            hints.push(update("update"));
        }
        if target.item.source == SourceKind::Manual {
            hints.push(self.find_online_hint(selected).main());
        }
        if idle && (selected > 0 || target.item.source != SourceKind::Manual) {
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
        hints.push(
            KeyHint::key(Id::Edit, "e", "edit .toc fields", &[KeyCode::Char('e')]).short("edit"),
        );
        if docked {
            hints.push(KeyHint::key(Id::Nav, "↑/↓", "move", UP_DOWN));
            if self.docked_details_overflow() {
                hints.push(KeyHint::key(
                    Id::Scroll,
                    "pgup/pgdn",
                    "scroll details",
                    PAGE,
                ));
            }
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
        hints.push(KeyHint::key(
            FooterHintId::Close,
            "esc or ?",
            "close",
            &[KeyCode::Esc, KeyCode::Char('?'), KeyCode::Char('q')],
        ));
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
                        KeyHint::key(Id::Save, "enter", "save", ENTER).main(),
                        KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
                        KeyHint::key(Id::Back, "backspace", "delete", BACKSPACE),
                    ];
                    hints.extend(self.text_box_hints());
                    return hints;
                }
                // Only what works on the row under the cursor.
                let mut hints = vec![KeyHint::key(Id::Fields, "↑/↓", "choose", UP_DOWN)];
                hints.push(match self.config_pane.selected_field().kind() {
                    ConfigRowKind::Text => KeyHint::key(Id::Toggle, "enter", "edit", ENTER).main(),
                    ConfigRowKind::Toggle | ConfigRowKind::Stepped => {
                        KeyHint::key(Id::Change, "←/→", "change", CHANGE).main()
                    }
                    ConfigRowKind::Action => KeyHint::key(Id::Run, "enter", "run", ENTER).main(),
                });
                hints.push(self.close_hint());
                hints.extend(self.sheet_scroll_hint(false));
                hints.push(self.help_hint());
                hints
            }
            DetailMode::Backup => {
                // While a backup, restore or delete runs, only moving and
                // closing do anything.
                let busy = self.backup_pane.in_progress;
                let selected = self.backup_pane.selected_backup().is_some();
                let mut hints = Vec::new();
                if self.backup_pane.backups.len() > 1 {
                    hints.push(KeyHint::key(Id::Nav, "↑/↓", "choose", UP_DOWN));
                }
                if selected && !busy {
                    hints.push(
                        KeyHint::key(
                            Id::Confirm,
                            "enter",
                            "restore",
                            &[KeyCode::Enter, KeyCode::Char('r')],
                        )
                        .main(),
                    );
                }
                if !busy {
                    hints.push(
                        KeyHint::key(Id::Run, "b", "back up now", &[KeyCode::Char('b')])
                            .short("back up"),
                    );
                }
                if selected && !busy {
                    hints.push(KeyHint::key(
                        Id::Delete,
                        "x",
                        "delete",
                        &[KeyCode::Char('x')],
                    ));
                }
                hints.push(self.close_hint());
                hints.extend(self.sheet_scroll_hint(false));
                hints.push(self.help_hint());
                hints
            }
            DetailMode::TocEdit => self.toc_edit_key_table(),
            DetailMode::Games => {
                let mut hints = vec![KeyHint::key(Id::Nav, "↑/↓", "choose", UP_DOWN)];
                let selected = self.games.selected.min(self.games.games.len());
                match self.games.games.get(selected) {
                    None => hints.push(KeyHint::key(Id::Run, "enter", "add game", ENTER).main()),
                    Some(game) if game.profile == self.runtime.profile_name => {
                        hints.push(KeyHint::key(Id::Close, "enter", "keep this game", ENTER))
                    }
                    Some(_) => hints.push(KeyHint::key(Id::Run, "enter", "switch", ENTER).main()),
                }
                if self.games.games.get(selected).is_some_and(|game| {
                    game.profile != self.runtime.profile_name && game.profile != DEFAULT_PROFILE
                }) {
                    hints.push(KeyHint::key(
                        Id::Delete,
                        "x",
                        "remove",
                        &[KeyCode::Char('x')],
                    ));
                }
                hints.push(self.close_hint());
                hints.extend(self.sheet_scroll_hint(false));
                hints.push(self.help_hint());
                hints
            }
            DetailMode::Find => self.find_key_table(),
            DetailMode::Overview => vec![KeyHint::key(Id::Close, "esc", "close", CLOSE)],
        }
    }

    /// The Find sheet: by step, and in the results by the row under the
    /// cursor.
    fn find_key_table(&self) -> Vec<KeyHint> {
        use super::find_sheet::{FindFocus, FindStep};
        use FooterHintId as Id;
        match self.find.step.as_ref() {
            Some(FindStep::Looking { .. }) => vec![
                self.find_sources_hint(),
                KeyHint::key(Id::Cancel, "esc", "cancel", CLOSE),
                self.help_hint(),
            ],
            Some(FindStep::Results(results)) => {
                let mut hints = Vec::new();
                if results.focusables(self.find_offers_search()).len() > 1 {
                    hints.push(KeyHint::key(Id::Nav, "↑/↓", "move", SCROLL));
                } else {
                    // With no row to move to, the arrows scroll a long report.
                    hints.extend(self.sheet_scroll_hint(true));
                }
                if self.find_space_works() {
                    let label = match results.cursor {
                        FindFocus::Result(index) if results.picks[index].is_some() => "unselect",
                        _ => "select",
                    };
                    hints.push(KeyHint::key(
                        Id::Toggle,
                        "space",
                        label,
                        &[KeyCode::Char(' ')],
                    ));
                }
                if self.find_enter_works() {
                    let label = match results.cursor {
                        FindFocus::Result(_) => "pick".to_string(),
                        FindFocus::Action if self.find_offers_search() => {
                            "search by name".to_string()
                        }
                        FindFocus::Action => {
                            let (links, replaces) = results.counts();
                            match (links, replaces) {
                                (_, 0) => "link".to_string(),
                                (0, _) => "replace".to_string(),
                                _ => "link and replace".to_string(),
                            }
                        }
                    };
                    hints.push(KeyHint::key(Id::Confirm, "enter", label, ENTER).main());
                }
                hints.push(self.find_sources_hint());
                hints.push(self.close_hint());
                hints.push(self.help_hint());
                hints
            }
            Some(FindStep::Failed(_)) | None => vec![self.close_hint(), self.help_hint()],
        }
    }

    /// The numbers of the sources: Wago.io's only while it can be on.
    fn find_sources_hint(&self) -> KeyHint {
        let (keys, codes) = self.find_source_keys();
        KeyHint::key(FooterHintId::Sources, keys, "sources", codes)
    }

    /// `F`, while some addon is Not linked.
    fn find_all_hint(&self) -> Option<KeyHint> {
        self.dashboard
            .items
            .iter()
            .any(|item| item.source == SourceKind::Manual)
            .then(|| {
                KeyHint::key(
                    FooterHintId::FindAll,
                    "F",
                    "find all",
                    &[KeyCode::Char('F')],
                )
            })
    }

    /// The .toc editor: moving and enter, or the box's keys while a field
    /// is edited. Tab shows only while it has a suggestion to fill in.
    fn toc_edit_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        if self.toc_pane.edit.is_some() {
            let mut hints = vec![
                KeyHint::key(Id::Save, "enter", "save", ENTER).main(),
                KeyHint::key(Id::Cancel, "esc", "cancel", ESC),
            ];
            if !self.toc_suggestions().is_empty() {
                hints.push(
                    KeyHint::key(Id::Next, "tab", "next suggestion", &[KeyCode::Tab])
                        .short("suggest"),
                );
            }
            hints.extend(self.text_box_hints());
            return hints;
        }
        let mut hints = vec![
            KeyHint::key(Id::Fields, "↑/↓", "choose", UP_DOWN),
            KeyHint::key(Id::Edit, "enter", "edit", ENTER).main(),
            KeyHint::key(Id::Close, "esc", "back to details", CLOSE).short("back"),
        ];
        hints.extend(self.sheet_scroll_hint(false));
        hints.push(self.help_hint());
        hints
    }

    /// Install: typing always goes to the box, so `q` and letters never
    /// close or move anything, and `esc` is the way out.
    fn search_key_table(&self) -> Vec<KeyHint> {
        use FooterHintId as Id;
        let pane = &self.search_pane;
        let close = |label: &'static str| KeyHint::key(Id::Close, "esc", label, ESC);
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
        let failed = matches!(pane.install_state, SearchInstallState::Error { .. });
        if pane.results_focused() && !failed {
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
    /// The list's keys, with `p switch game` once there are two games or
    /// more, while it does something: no job runs, nothing is selected and
    /// no filter is being typed.
    fn dashboard_key_table(&self, width: u16) -> Vec<KeyHint> {
        let mut hints = self.dashboard_list_key_table(width);
        if self.games.games.len() >= 2
            && !self.dashboard.filter_editing()
            && self.dashboard.selected_parent_count() == 0
            && self.switch_busy_note().is_none()
        {
            let switch = KeyHint::key(
                FooterHintId::SwitchGame,
                "p",
                "switch game",
                &[KeyCode::Char('p')],
            )
            .short("games");
            match hints
                .iter()
                .position(|hint| hint.id == Some(FooterHintId::Help))
            {
                Some(index) => hints.insert(index, switch),
                None => hints.push(switch),
            }
        }
        hints
    }

    fn dashboard_list_key_table(&self, width: u16) -> Vec<KeyHint> {
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
        if let Some(job) = self.dashboard_job_running() {
            return vec![
                KeyHint::text(match job {
                    DashboardJobKind::Update => "LemonUp is updating. You can keep browsing.",
                    DashboardJobKind::Check => {
                        "LemonUp is checking for updates. You can keep browsing."
                    }
                }),
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
            let mut hints = vec![
                KeyHint::text(format!("{selected} selected:")),
                update.main(),
                check,
                KeyHint::key(Id::Delete, "x", "delete", &[KeyCode::Char('x')]),
                KeyHint::key(Id::Select, "space", select_label, SELECT),
                esc,
                help,
            ];
            // `f` still works on the addon under the cursor.
            if self.not_linked_current_item().is_some() {
                hints.insert(4, self.find_online_hint(selected));
            }
            return hints;
        }
        let settings = KeyHint::key(Id::Config, "s", "settings", SETTINGS);
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
                self.find_online_hint(self.dashboard.selected_parent_count())
                    .main(),
            ];
            hints.extend(self.find_all_hint());
            hints.extend([
                details,
                KeyHint::key(Id::Install, "i", "install", &[KeyCode::Char('i')]),
            ]);
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
            .map(|_| self.find_online_hint(selected));
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
                KeyHint::key(Id::Config, "s", "settings", SETTINGS),
                help,
            ]
        } else {
            vec![check_all, details, install, filter, help]
        };
        if let Some(find) = find {
            hints.insert(2, find);
        }
        if wide && let Some(find_all) = self.find_all_hint() {
            let at = hints
                .iter()
                .position(|hint| hint.id == Some(Id::Help))
                .unwrap_or(hints.len());
            hints.insert(at, find_all);
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
