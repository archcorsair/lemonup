//! Setup's screen: the header, the stepper, one page per step and the key
//! line, drawn without boxes like the addon list. The page only says what
//! is true right now: no status text in the footer, no key that does
//! nothing.

use super::render_dashboard::{WIDE_FRAME_MIN_WIDTH, shorten_path_middle};
use super::sheet::wrap_lines;
use super::*;
use crate::onboarding::{FoundState, NotFoundChoice, NotFoundState, PreferenceField, TypingState};

/// Columns before every line of the page.
const INDENT: &str = "   ";
/// Header (two rows), blank, stepper and the top rule: the rows above the
/// page and any question over it.
const ABOVE_PAGE_ROWS: u16 = 5;
/// Rule, blank, keys.
const BOTTOM_ROWS: u16 = 3;
/// The widest a text box gets.
const BOX_WIDTH: usize = 96;

impl App {
    pub(super) fn draw_setup_frame(&mut self, frame: &mut Frame<'_>) {
        let roles = self.ui_theme.roles;
        let area = frame.area();
        frame.render_widget(
            Block::default().style(Style::default().bg(roles.background).fg(roles.text)),
            area,
        );
        frame.render_widget(
            Paragraph::new(self.setup_lines(area.width, area.height)),
            area,
        );
        // "Quit setup?" sits over the page, between the two rules.
        let top = ABOVE_PAGE_ROWS.min(area.height);
        let region = Rect::new(
            area.x,
            area.y + top,
            area.width,
            area.height.saturating_sub(top + BOTTOM_ROWS),
        );
        self.render_question(frame, region);
    }

    /// Every row of the setup screen at `width` x `height`.
    pub(super) fn setup_lines(&self, width: u16, height: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let rule = Line::from(Span::styled(
            "─".repeat(usize::from(width)),
            Style::default().fg(roles.dim),
        ));
        let (title, body) = self.setup_page(width);
        let mut lines = self.setup_header_lines(width);
        lines.push(Line::from(""));
        lines.push(self.setup_stepper_line());
        lines.push(rule.clone());
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw(INDENT),
            Span::styled(
                title,
                Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
        // The page gets what the rows above it and the key line leave.
        let space = usize::from(height)
            .saturating_sub(lines.len())
            .saturating_sub(usize::from(BOTTOM_ROWS));
        lines.extend(body.into_iter().take(space));
        let filled = usize::from(height.saturating_sub(BOTTOM_ROWS));
        while lines.len() < filled {
            lines.push(Line::from(""));
        }
        lines.push(rule);
        lines.push(Line::from(""));
        lines.push(self.dashboard_key_line(width));
        lines
    }

    /// "Welcome. Let's get your addons set up." and "Step 1 of 3", beside
    /// the logo (the short one below 110 columns).
    fn setup_header_lines(&self, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        // Adding a game is only the game step, with no welcome.
        let welcome = if self.games.adding {
            [
                Span::styled(
                    "Add another game.",
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " Pick the install LemonUp should look after too.",
                    roles.muted,
                ),
            ]
        } else {
            [
                Span::styled(
                    "Welcome.",
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Let's get your addons set up.", roles.muted),
            ]
        };
        let mut step = Vec::new();
        if !self.games.adding {
            step.push(Span::styled(
                format!(
                    "Step {} of {}",
                    self.onboarding.step.index() + 1,
                    OnboardingStep::ALL.len()
                ),
                roles.muted,
            ));
        }
        if !self.runtime.is_default_profile() && !self.games.adding {
            step.push(Span::styled("   Profile: ", roles.muted));
            step.push(Span::styled(
                self.runtime.profile_name.clone(),
                Style::default()
                    .fg(roles.amber)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let (mut first, mut second) =
            if width >= WIDE_FRAME_MIN_WIDTH && !self.ui_theme.symbols.simple {
                let mut first = vec![Span::raw(" ")];
                first.extend(self.fruit_gradient_logo_line(LOGO_FULL[0]).spans);
                first.push(Span::raw("    "));
                let mut second = vec![Span::raw(" ")];
                second.extend(self.fruit_gradient_logo_line(LOGO_FULL[1]).spans);
                second.push(Span::raw("    "));
                (first, second)
            } else {
                let mut first = vec![Span::raw(" ")];
                first.extend(self.short_logo_spans());
                first.push(Span::raw("  "));
                (first, vec![Span::raw(" ")])
            };
        first.extend(welcome);
        second.extend(step);
        vec![Line::from(first), Line::from(second)]
    }

    /// `● Find your game  ───  ○ Wago.io  ───  ○ Preferences`.
    fn setup_stepper_line(&self) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let current = self.onboarding.step.index();
        let mut spans = vec![Span::raw(INDENT)];
        // Adding a game has only the game step.
        let steps = if self.games.adding { 1 } else { usize::MAX };
        for step in OnboardingStep::ALL.into_iter().take(steps) {
            if step.index() > 0 {
                spans.push(Span::styled("  ───  ", Style::default().fg(roles.dim)));
            }
            if step.index() < current {
                spans.push(Span::styled(
                    format!("{} ", self.ui_theme.symbols.done),
                    Style::default().fg(roles.green),
                ));
                spans.push(Span::styled(step.label(), roles.muted));
            } else if step.index() == current {
                spans.push(Span::styled(
                    format!("● {}", step.label()),
                    Style::default()
                        .fg(roles.lemon)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(format!("○ {}", step.label()), roles.muted));
            }
        }
        Line::from(spans)
    }

    /// The page's title and the lines under it, indented.
    fn setup_page(&self, width: u16) -> (String, Vec<Line<'static>>) {
        let (title, body) = match self.onboarding.step {
            OnboardingStep::FindGame => self.find_game_page(width),
            OnboardingStep::Wago => self.wago_page(width),
            OnboardingStep::Preferences => self.preferences_page(width),
        };
        let body = body
            .into_iter()
            .map(|line| {
                let mut spans = vec![Span::raw(INDENT)];
                spans.extend(line.spans);
                Line::from(spans)
            })
            .collect();
        (title, body)
    }

    /// Room for text after the indent, keeping a column free on the right.
    fn setup_text_width(width: u16) -> u16 {
        width.saturating_sub(u16::try_from(INDENT.len()).unwrap_or(3) + 1)
    }

    fn setup_prose(&self, text: &str, style: Style, width: u16) -> Vec<Line<'static>> {
        wrap_lines(
            &[Line::from(Span::styled(text.to_string(), style))],
            Self::setup_text_width(width),
        )
    }

    fn find_game_page(&self, width: u16) -> (String, Vec<Line<'static>>) {
        let roles = self.ui_theme.roles;
        let mut lines = Vec::new();
        if let Some((notice, is_error)) = self.startup_notice.as_ref() {
            let color = if *is_error { roles.amber } else { roles.green };
            lines.extend(self.setup_prose(
                &notice.replace(" | ", " · "),
                Style::default().fg(color),
                width,
            ));
            lines.push(Line::from(""));
        }
        let title = match &self.onboarding.phase {
            OnboardingPhase::Bootstrapping | OnboardingPhase::QuickChecking => {
                lines.push(Line::from(vec![
                    Span::styled(self.spinner_frame(), Style::default().fg(roles.lemon)),
                    Span::raw(" Checking the usual install places"),
                ]));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "This usually takes a few seconds.",
                    roles.muted,
                )));
                "Looking for World of Warcraft…".to_string()
            }
            OnboardingPhase::DeepScanning(progress) => {
                lines.push(Line::from(vec![
                    Span::styled(self.spinner_frame(), Style::default().fg(roles.lemon)),
                    Span::raw(format!(
                        " Checked {} folder{} {}",
                        group_thousands(progress.dirs_scanned),
                        plural_suffix(progress.dirs_scanned),
                        self.setup_scan_place()
                    )),
                ]));
                let room = usize::from(Self::setup_text_width(width)).saturating_sub(2);
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {}",
                        shorten_path_middle(
                            &friendly_path(Path::new(&progress.current_path)),
                            room
                        )
                    ),
                    roles.muted,
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    if self.setup_scan_is_home() {
                        "This usually takes a few seconds."
                    } else {
                        "This can take a few minutes."
                    },
                    roles.muted,
                )));
                "Looking for World of Warcraft…".to_string()
            }
            OnboardingPhase::Found(found) => {
                lines.extend(self.found_lines(found, width));
                if width >= WIDE_FRAME_MIN_WIDTH {
                    "Found World of Warcraft. Which game version should LemonUp look after?"
                        .to_string()
                } else {
                    "Found World of Warcraft. Which game version?".to_string()
                }
            }
            OnboardingPhase::NotFound(not_found) => {
                lines.extend(self.not_found_lines(not_found, width));
                "Couldn't find World of Warcraft".to_string()
            }
            OnboardingPhase::Typing(typing) => {
                lines.extend(self.typing_lines(typing, width));
                "Where is World of Warcraft?".to_string()
            }
        };
        (title, lines)
    }

    /// Where the search is looking: "in your home folder", "in /mnt".
    fn setup_scan_place(&self) -> String {
        let root = &self.onboarding.scan_root;
        if self.setup_scan_is_home() {
            "in your home folder".to_string()
        } else if root.parent().is_none() && root.has_root() && !cfg!(windows) {
            "on the whole computer".to_string()
        } else {
            format!("in {}", friendly_path(root))
        }
    }

    fn setup_scan_is_home(&self) -> bool {
        home_folder().is_some_and(|home| paths_match(&home, &self.onboarding.scan_root))
    }

    fn found_lines(&self, found: &FoundState, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let wide = width >= WIDE_FRAME_MIN_WIDTH;
        let (name_width, gap) = if wide { (18, 6) } else { (16, 4) };
        let count_width = found
            .installs
            .iter()
            .map(|install| addon_count_label(install.addons).chars().count())
            .max()
            .unwrap_or(0);
        let path_room = usize::from(Self::setup_text_width(width))
            .saturating_sub(2 + name_width + count_width + gap);
        let mut lines = Vec::new();
        for (index, install) in found.installs.iter().enumerate() {
            let selected = found.selected == index;
            let guarded = self.is_guarded_path(&install.addons_dir);
            let client = match GameFlavor::detect_from_path(&install.addons_dir) {
                Some(GameFlavor::Forever) => "WoW Forever",
                _ => "Retail",
            };
            let game_dir = install
                .addons_dir
                .ancestors()
                .nth(2)
                .unwrap_or(&install.addons_dir);
            let count = addon_count_label(install.addons);
            let mut spans = self.setup_choice_marker(selected);
            spans.push(Span::styled(
                format!("{client:<name_width$}"),
                self.setup_choice_style(selected),
            ));
            spans.push(Span::styled(
                format!("{count:>count_width$}"),
                if install.addons == 0 || guarded {
                    roles.muted
                } else {
                    Style::default().fg(roles.text)
                },
            ));
            spans.push(Span::raw(" ".repeat(gap)));
            spans.push(Span::styled(
                shorten_path_middle(&friendly_path(game_dir), path_room),
                roles.muted,
            ));
            lines.push(Line::from(spans));
        }
        let selected = found.selected_install().is_none();
        let mut spans = self.setup_choice_marker(selected);
        spans.push(Span::styled(
            format!(
                "{:<width$}",
                "Somewhere else…",
                width = name_width + count_width + gap
            ),
            self.setup_choice_style(selected),
        ));
        spans.push(Span::styled(
            "type or paste a different folder",
            roles.muted,
        ));
        lines.push(Line::from(spans));
        lines.push(Line::from(""));
        if found
            .selected_install()
            .is_some_and(|install| self.is_guarded_path(&install.addons_dir))
        {
            lines.extend(self.setup_prose(
                "This profile can't use your main AddOns folder. Choose a test copy instead.",
                Style::default().fg(roles.amber),
                width,
            ));
            lines.push(Line::from(""));
        }
        if found.installs.len() > 1 {
            let muted = roles.muted;
            lines.push(Line::from(Span::styled(
                "LemonUp looks after one game version at a time.",
                muted,
            )));
            let mut later = vec![
                Span::styled("You can add the other one later: ", muted),
                Span::styled(
                    "lemonup.org/getting-started",
                    Style::default().fg(roles.blue),
                ),
            ];
            if wide {
                later.push(Span::styled(" shows how.", muted));
            }
            lines.push(Line::from(later));
        }
        lines
    }

    fn not_found_lines(&self, not_found: &NotFoundState, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let label_width = if width >= WIDE_FRAME_MIN_WIDTH {
            36
        } else {
            32
        };
        let mut lines = self.setup_prose(&not_found.note, roles.muted, width);
        lines.push(Line::from(""));
        let detail_room =
            usize::from(Self::setup_text_width(width)).saturating_sub(2 + label_width);
        for (index, choice) in not_found.choices.iter().enumerate() {
            let selected = not_found.selected == index;
            let (label, detail) = not_found_choice_text(choice);
            let mut spans = self.setup_choice_marker(selected);
            spans.push(Span::styled(
                format!("{label:<label_width$}"),
                self.setup_choice_style(selected),
            ));
            spans.push(Span::styled(
                truncate_text(&detail, detail_room),
                roles.muted,
            ));
            lines.push(Line::from(spans));
        }
        lines
    }

    fn typing_lines(&self, typing: &TypingState, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let text = Style::default().fg(roles.text).add_modifier(Modifier::BOLD);
        let caret = Style::default().fg(roles.lemon);
        let mut lines = self.setup_box_lines(
            "Game folder",
            &self.onboarding.folder_box,
            text,
            caret,
            None,
            width,
        );
        if let Some(error) = &typing.error {
            // The first sentence says what is wrong, the rest what to do.
            let (problem, help) = match error.split_once(". ") {
                Some((problem, help)) => (format!("{problem}."), format!(" {help}")),
                None => (error.clone(), String::new()),
            };
            lines.extend(wrap_lines(
                &[Line::from(vec![
                    Span::styled(
                        "! ",
                        Style::default().fg(roles.red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(problem, Style::default().fg(roles.red)),
                    Span::styled(help, roles.muted),
                ])],
                Self::setup_text_width(width),
            ));
        }
        lines.push(Line::from(""));
        lines.extend(self.setup_prose(
            "Any folder of the game works: LemonUp finds the AddOns folder inside it.",
            roles.muted,
            width,
        ));
        lines
    }

    fn wago_page(&self, width: u16) -> (String, Vec<Line<'static>>) {
        let roles = self.ui_theme.roles;
        let mut lines = self.setup_prose(
            "Wago.io is one of the sites LemonUp gets addons from. Searching it, and updating \
             addons that came from it, needs your own Wago.io key. Wago gives one to its \
             Patreon supporters:",
            Style::default().fg(roles.text),
            width,
        );
        lines.push(Line::from(Span::styled(
            "addons.wago.io/patreon",
            Style::default().fg(roles.blue),
        )));
        lines.push(Line::from(""));
        lines.extend(self.setup_prose(
            "Addons from GitHub, WoWInterface and TukUI work without a key.",
            roles.muted,
            width,
        ));
        lines.push(Line::from(""));
        lines.extend(self.setup_box_lines(
            "Wago.io key",
            &self.onboarding.key_box.masked('•'),
            Style::default().fg(roles.text),
            Style::default().fg(roles.lemon),
            Some(" paste your key here, or leave empty"),
            width,
        ));
        lines.push(Line::from(Span::styled(
            "You can add or change it later in Settings.",
            roles.muted,
        )));
        ("Wago.io key (you can skip this)".to_string(), lines)
    }

    fn preferences_page(&self, width: u16) -> (String, Vec<Line<'static>>) {
        let roles = self.ui_theme.roles;
        let label_width = if width >= WIDE_FRAME_MIN_WIDTH {
            49
        } else {
            42
        };
        let mut lines = Vec::new();
        for field in self.onboarding.preference_fields() {
            let selected = field == self.onboarding.preference;
            let mut spans = self.setup_choice_marker(selected);
            spans.push(Span::styled(
                format!("{:<label_width$}", self.setup_preference_label(field)),
                self.setup_choice_style(selected),
            ));
            let arrow = Style::default().fg(roles.dim);
            spans.push(Span::styled("‹ ", arrow));
            spans.push(Span::styled(
                self.setup_preference_value(field),
                if selected {
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(roles.text)
                },
            ));
            spans.push(Span::styled(" ›", arrow));
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
        lines.extend(self.setup_prose(
            "Game settings are your keybinds, macros and addon setups (the game's WTF folder).",
            roles.muted,
            width,
        ));
        lines.extend(self.setup_prose(
            "A backup lets you put them back if an update breaks something.",
            roles.muted,
            width,
        ));
        if self.onboarding.other_game.is_some() {
            lines.extend(self.setup_prose(
                "Each game has its own addon list and backups. Switch games any time with p.",
                roles.muted,
                width,
            ));
        }
        if let Some(problem) = &self.onboarding.problem {
            lines.push(Line::from(""));
            lines.extend(wrap_lines(
                &[Line::from(vec![
                    Span::styled(
                        "! ",
                        Style::default().fg(roles.red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(problem.clone(), Style::default().fg(roles.red)),
                ])],
                Self::setup_text_width(width),
            ));
        }
        (
            "Last step. These are good defaults; change them any time in Settings.".to_string(),
            lines,
        )
    }

    /// A row's label: "Also set up" names the game it would set up.
    fn setup_preference_label(&self, field: PreferenceField) -> String {
        match (field, &self.onboarding.other_game) {
            (PreferenceField::AlsoSetUp, Some(addon_dir)) => {
                let name = GameFlavor::detect_from_path(addon_dir)
                    .map(lemonup_core::profiles::game_name)
                    .unwrap_or("my other game");
                format!("Also set up {name}")
            }
            _ => field.label().to_string(),
        }
    }

    /// A preference's value, in the words the Settings sheet uses.
    fn setup_preference_value(&self, field: PreferenceField) -> String {
        let field = match field {
            PreferenceField::AlsoSetUp => {
                return if self.onboarding.set_up_other_game {
                    "Yes"
                } else {
                    "No"
                }
                .to_string();
            }
            PreferenceField::Theme => ConfigField::Theme,
            PreferenceField::BackupWtf => ConfigField::BackupWtf,
            PreferenceField::BackupRetention => ConfigField::BackupRetention,
        };
        self.look_value_text(&self.onboarding.draft, field)
    }

    /// `▌ ` on the chosen row, two spaces on the others.
    fn setup_choice_marker(&self, selected: bool) -> Vec<Span<'static>> {
        vec![Span::styled(
            if selected { "▌ " } else { "  " },
            Style::default().fg(self.ui_theme.roles.lemon),
        )]
    }

    fn setup_choice_style(&self, selected: bool) -> Style {
        let style = Style::default().fg(self.ui_theme.roles.text);
        if selected {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    /// A text box with its name in the top border, as wide as the page
    /// allows. It always has focus here, so the cursor shows. `hint` shows
    /// in the empty box, after the cursor.
    fn setup_box_lines(
        &self,
        name: &str,
        input: &TextInput,
        text: Style,
        caret: Style,
        hint: Option<&str>,
        width: u16,
    ) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let border = Style::default().fg(roles.lemon);
        let outer = usize::from(Self::setup_text_width(width)).clamp(10, BOX_WIDTH);
        let inner = outer - 4;
        let mut spans = input.spans(inner, text, caret, true);
        if input.is_empty()
            && let Some(hint) = hint
        {
            spans.push(Span::styled(
                truncate_text(hint, inner.saturating_sub(1)),
                roles.muted,
            ));
        }
        let used: usize = spans.iter().map(|span| span.width()).sum();
        let top_fill = outer.saturating_sub(name.chars().count() + 5);
        let mut middle = vec![Span::styled("│ ", border)];
        middle.extend(spans);
        middle.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
        middle.push(Span::styled(" │", border));
        vec![
            Line::from(vec![
                Span::styled("┌─ ", border),
                Span::styled(
                    name.to_string(),
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {}┐", "─".repeat(top_fill)), border),
            ]),
            Line::from(middle),
            Line::from(Span::styled(format!("└{}┘", "─".repeat(outer - 2)), border)),
        ]
    }
}

/// A not-found choice's label and the muted text after it.
fn not_found_choice_text(choice: &NotFoundChoice) -> (String, String) {
    match choice {
        NotFoundChoice::TypeFolder => (
            "Type or paste the game folder".to_string(),
            format!("for example {}", example_game_folder()),
        ),
        NotFoundChoice::Search(root) if cfg!(windows) => (
            format!("Search drive {}", root.display()),
            "can take a few minutes".to_string(),
        ),
        NotFoundChoice::Search(root) if root.parent().is_none() => (
            "Search the whole computer".to_string(),
            "can take a few minutes".to_string(),
        ),
        NotFoundChoice::Search(root) => ("Search another drive".to_string(), friendly_path(root)),
    }
}

fn example_game_folder() -> &'static str {
    if cfg!(windows) {
        r"C:\Program Files (x86)\World of Warcraft"
    } else if cfg!(target_os = "macos") {
        "/Applications/World of Warcraft"
    } else {
        "~/Games/World of Warcraft"
    }
}

/// `1284` as "1,284".
pub(super) fn group_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}
