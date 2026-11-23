//! `lemonup profiles`: the games LemonUp knows, one row each.

use std::path::PathBuf;

use lemonup_core::ProfileSummary;
use serde::Serialize;

use super::{INDENT, Paint, Report, SUB_INDENT, Tone, columns, pad, section, wrap_words};

/// What `lemonup profiles` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ProfilesReport {
    profiles: Vec<ProfileRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ProfileRow {
    name: String,
    game: String,
    game_version: Option<&'static str>,
    #[serde(serialize_with = "super::lossy_path")]
    addon_dir: Option<PathBuf>,
    addons: Option<usize>,
    problem: Option<String>,
}

impl ProfilesReport {
    pub(crate) fn new(profiles: &[ProfileSummary]) -> Self {
        Self {
            profiles: profiles
                .iter()
                .map(|profile| ProfileRow {
                    name: profile.name.clone(),
                    game: profile.display_name.clone(),
                    game_version: profile.flavor.map(lemonup_core::profiles::profile_name_for),
                    addon_dir: profile.addon_dir.clone(),
                    addons: profile.addon_count,
                    problem: profile.problem.clone(),
                })
                .collect(),
        }
    }
}

impl Report for ProfilesReport {
    const COMMAND: &'static str = "profiles";

    /// `Games (N)`, then a row per game: name, game, AddOns folder and addon count in aligned
    /// columns, and what couldn't be read under it. Rows are never cut or wrapped.
    fn human(&self, paint: Paint) -> Vec<String> {
        let mut rows = Vec::new();
        let folders = self
            .profiles
            .iter()
            .map(|row| {
                row.addon_dir.as_deref().map_or_else(
                    || "no AddOns folder set".to_string(),
                    crate::app::friendly_path,
                )
            })
            .collect::<Vec<_>>();
        let widest = |widths: Vec<usize>| widths.into_iter().max().unwrap_or(0);
        let name_width = widest(self.profiles.iter().map(|row| columns(&row.name)).collect());
        let game_width = widest(self.profiles.iter().map(|row| columns(&row.game)).collect());
        let folder_width = widest(folders.iter().map(|folder| columns(folder)).collect());
        let count_width = widest(
            self.profiles
                .iter()
                .filter_map(|row| row.addons)
                .map(|count| count.to_string().len())
                .collect(),
        );
        for (row, folder) in self.profiles.iter().zip(&folders) {
            let mut line = format!(
                "{INDENT}{}  {}  ",
                paint.text(&pad(&row.name, name_width)),
                paint.text(&pad(&row.game, game_width)),
            );
            match row.addons {
                Some(count) => {
                    let word = if count == 1 { "addon" } else { "addons" };
                    line.push_str(&paint.muted(&pad(folder, folder_width)));
                    line.push_str(&paint.text(&format!("  {count:>count_width$} {word}")));
                }
                None => line.push_str(&paint.muted(folder)),
            }
            rows.push(line);
            if let Some(problem) = &row.problem {
                rows.extend(
                    wrap_words(SUB_INDENT, problem, paint.width())
                        .iter()
                        .map(|line| paint.muted(line)),
                );
            }
        }
        let mut lines = Vec::new();
        section(
            &mut lines,
            paint,
            Tone::Quiet,
            "Games",
            self.profiles.len(),
            rows,
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{GameFlavor, ProfileSummary};

    use super::ProfilesReport;
    use crate::output::{Output, OutputMode, Paint, Report};

    fn summary(
        name: &str,
        game: &str,
        flavor: Option<GameFlavor>,
        addon_dir: Option<&str>,
        addons: Option<usize>,
    ) -> ProfileSummary {
        ProfileSummary {
            name: name.to_string(),
            display_name: game.to_string(),
            addon_dir: addon_dir.map(PathBuf::from),
            flavor,
            addon_count: addons,
            problem: None,
        }
    }

    fn two_games() -> Vec<ProfileSummary> {
        vec![
            summary(
                "default",
                "Retail",
                Some(GameFlavor::Retail),
                Some("/games/wow/_retail_/Interface/AddOns"),
                Some(18),
            ),
            summary(
                "forever",
                "WoW Forever",
                Some(GameFlavor::Forever),
                Some("/games/wow/_forever_/Interface/AddOns"),
                Some(3),
            ),
        ]
    }

    /// `line` with its escape codes taken out.
    fn strip(line: &str) -> String {
        let mut out = String::new();
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn two_games_are_aligned_with_the_counts_right_aligned() {
        let lines = ProfilesReport::new(&two_games()).human(Paint::PLAIN);
        assert_eq!(
            lines,
            [
                "Games (2)",
                "  default  Retail       /games/wow/_retail_/Interface/AddOns   18 addons",
                "  forever  WoW Forever  /games/wow/_forever_/Interface/AddOns   3 addons",
            ]
        );
    }

    #[test]
    fn one_addon_is_singular() {
        let mut games = two_games();
        games[1].addon_count = Some(1);
        let lines = ProfilesReport::new(&games).human(Paint::PLAIN);
        assert_eq!(
            lines[2],
            "  forever  WoW Forever  /games/wow/_forever_/Interface/AddOns   1 addon"
        );
        assert_eq!(
            lines[1],
            "  default  Retail       /games/wow/_retail_/Interface/AddOns   18 addons"
        );
    }

    #[test]
    fn a_game_without_a_folder_or_a_count_says_so_and_shows_no_count() {
        let mut games = two_games();
        games.push(summary("broken", "broken", None, None, None));
        let lines = ProfilesReport::new(&games).human(Paint::PLAIN);
        assert_eq!(lines[0], "Games (3)");
        assert_eq!(lines[3], "  broken   broken       no AddOns folder set");
        assert_eq!(
            lines[1],
            "  default  Retail       /games/wow/_retail_/Interface/AddOns   18 addons"
        );
    }

    #[test]
    fn a_problem_is_a_note_under_its_row_wrapped_to_the_width() {
        let mut games = vec![summary("broken", "broken", None, None, Some(0))];
        games[0].problem = Some(
            "couldn't read its settings: expected a value at line 1 column 12 of the file \
             config.toml in its folder"
                .to_string(),
        );
        let lines = ProfilesReport::new(&games).human(Paint::PLAIN.with_width(60));
        assert_eq!(
            lines,
            [
                "Games (1)",
                "  broken  broken  no AddOns folder set  0 addons",
                "      couldn't read its settings: expected a value at line 1",
                "      column 12 of the file config.toml in its folder",
            ]
        );
    }

    #[test]
    fn a_folder_with_wide_characters_keeps_the_counts_aligned() {
        let games = vec![
            summary(
                "default",
                "Retail",
                Some(GameFlavor::Retail),
                Some("/games/魔兽世界/AddOns"),
                Some(18),
            ),
            summary(
                "forever",
                "WoW Forever",
                Some(GameFlavor::Forever),
                Some("/games/wow/AddOns"),
                Some(3),
            ),
        ];
        assert_eq!(
            ProfilesReport::new(&games).human(Paint::PLAIN),
            [
                "Games (2)",
                "  default  Retail       /games/魔兽世界/AddOns  18 addons",
                "  forever  WoW Forever  /games/wow/AddOns        3 addons",
            ]
        );
    }

    #[test]
    fn no_games_is_one_line() {
        assert_eq!(ProfilesReport::new(&[]).human(Paint::PLAIN), ["Games (0)"]);
    }

    #[test]
    fn colored_rows_use_only_the_16_colors_and_keep_the_text() {
        let mut games = two_games();
        games[0].problem = Some("its addon list couldn't be read".to_string());
        let report = ProfilesReport::new(&games);
        let colored = report.human(Paint::terminal());
        let joined = colored.join("\n");
        assert!(joined.contains('\x1b'), "{joined:?}");
        assert!(!joined.contains("38;2;"), "24-bit color: {joined:?}");
        // The heading is bold without a color, the folder and the problem are muted.
        assert_eq!(colored[0], "\x1b[1mGames\x1b[0m \x1b[2m(2)\x1b[0m");
        assert!(
            colored[1].contains("\x1b[2m/games/wow/_retail_/Interface/AddOns"),
            "{:?}",
            colored[1]
        );
        assert_eq!(
            colored[2],
            "\x1b[2m      its addon list couldn't be read\x1b[0m"
        );
        let stripped = colored.iter().map(|line| strip(line)).collect::<Vec<_>>();
        assert_eq!(stripped, report.human(Paint::PLAIN));
    }

    #[test]
    fn the_json_document_has_the_envelope_and_every_field() {
        let mut games = two_games();
        games.push(summary("broken", "broken", None, None, None));
        games[2].problem = Some("couldn't read its settings".to_string());
        let mut output = Output::new(
            OutputMode::Json,
            Paint::terminal(),
            Paint::terminal(),
            "default",
            Vec::new(),
            Vec::new(),
        );
        output
            .print(&ProfilesReport::new(&games), None)
            .expect("print");
        let (stdout, stderr) = output.into_writers();
        assert!(stderr.is_empty());
        assert_eq!(
            String::from_utf8(stdout).expect("utf-8"),
            concat!(
                r#"{"schema":1,"command":"profiles","profile":"default","addon_dir":null,"profiles":["#,
                r#"{"name":"default","game":"Retail","game_version":"retail","addon_dir":"/games/wow/_retail_/Interface/AddOns","addons":18,"problem":null},"#,
                r#"{"name":"forever","game":"WoW Forever","game_version":"forever","addon_dir":"/games/wow/_forever_/Interface/AddOns","addons":3,"problem":null},"#,
                r#"{"name":"broken","game":"broken","game_version":null,"addon_dir":null,"addons":null,"problem":"couldn't read its settings"}"#,
                r#"],"warnings":[]}"#,
                "\n"
            )
        );
    }
}
