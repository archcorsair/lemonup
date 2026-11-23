//! `lemonup toc`: an addon's Title, Notes and Category, or the one field just changed.

use lemonup_core::toc_edit::TocValueSource;
use serde::{Serialize, Serializer};

use super::{Paint, Report, columns};

/// What `lemonup toc` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TocReport {
    addon: String,
    folder: String,
    fields: Vec<TocRow>,
    changed: Option<TocChange>,
    /// The change was saved but the files could not be read again, so `fields` is empty
    /// without the addon having none.
    #[serde(skip)]
    fields_unread: bool,
}

/// One field as the game reads it from the addon's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TocRow {
    pub(crate) field: &'static str,
    pub(crate) value: Option<String>,
    /// In JSON `addon`, `override` or `override_not_applied`.
    #[serde(serialize_with = "serialize_value_source")]
    pub(crate) source: TocValueSource,
}

fn serialize_value_source<S: Serializer>(
    source: &TocValueSource,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(source.cli_name())
}

/// The field a run changed: `value` is the user's new one, `None` when the addon's own came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TocChange {
    pub(crate) field: &'static str,
    pub(crate) value: Option<String>,
    pub(crate) folders: Vec<String>,
}

impl TocReport {
    pub(crate) fn new(
        addon: &str,
        folder: &str,
        fields: Vec<TocRow>,
        changed: Option<TocChange>,
    ) -> Self {
        Self {
            addon: addon.to_string(),
            folder: folder.to_string(),
            fields,
            changed,
            fields_unread: false,
        }
    }

    /// The report of a change whose files could not be read again afterwards.
    pub(crate) fn with_fields_unread(self) -> Self {
        Self {
            fields_unread: true,
            ..self
        }
    }

    /// What `fields` can't say: that it is empty because the files could not be read.
    pub(crate) fn warnings(&self) -> Vec<String> {
        if self.fields_unread {
            vec![
                "The change is saved, but the addon's .toc files could not be read again."
                    .to_string(),
            ]
        } else {
            Vec::new()
        }
    }
}

impl Report for TocReport {
    const COMMAND: &'static str = "toc";

    /// The change that was saved, or the addon's name in bold and one aligned row per field.
    fn human(&self, paint: Paint) -> Vec<String> {
        if let Some(change) = &self.changed {
            return match &change.value {
                Some(value) => vec![
                    format!(
                        "{} {}",
                        paint.done(),
                        paint.text(&format!(
                            "Saved {} for {}: {value}",
                            change.field, self.addon
                        ))
                    ),
                    paint.muted("/reload to take effect in game"),
                ],
                None => vec![format!(
                    "{} {}",
                    paint.done(),
                    paint.text(&format!("Restored {}'s {}", self.addon, change.field))
                )],
            };
        }
        let width = self
            .fields
            .iter()
            .map(|row| columns(row.field))
            .max()
            .unwrap_or(0);
        // The addon was named on the command line, so the rows need no heading.
        let mut lines = Vec::new();
        for row in &self.fields {
            // The name in bold, then the spaces that line the values up.
            let mut line = format!(
                "{}{}  ",
                paint.bold(row.field),
                " ".repeat(width.saturating_sub(columns(row.field)))
            );
            match &row.value {
                // As the game shows it, without color codes.
                Some(value) => {
                    line.push_str(&paint.text(&lemonup_core::toc_edit::display_value(value)))
                }
                None => line.push_str(&paint.muted("none")),
            }
            let mark = match row.source {
                TocValueSource::Override => Some("(yours)"),
                TocValueSource::OverrideNotApplied => Some("(yours, not in the files)"),
                TocValueSource::Addon => None,
            };
            if let Some(mark) = mark {
                line.push_str("  ");
                line.push_str(&paint.muted(mark));
            }
            lines.push(line);
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lemonup_core::toc_edit::TocValueSource;

    use super::{TocChange, TocReport, TocRow};
    use crate::output::tests::{json_document, strip};
    use crate::output::{Paint, Report};
    use crate::theme::Symbols;

    fn clique() -> TocReport {
        TocReport::new(
            "Clique",
            "Clique",
            vec![
                TocRow {
                    field: "Title",
                    value: Some("|cff00ff00Clique|r".to_string()),
                    source: TocValueSource::Addon,
                },
                TocRow {
                    field: "Notes",
                    value: Some("My Keys".to_string()),
                    source: TocValueSource::Override,
                },
                TocRow {
                    field: "Category",
                    value: None,
                    source: TocValueSource::Addon,
                },
            ],
            None,
        )
    }

    fn changed(value: Option<&str>) -> TocReport {
        TocReport::new(
            "Bagnon",
            "Bagnon",
            Vec::new(),
            Some(TocChange {
                field: "Category",
                value: value.map(str::to_string),
                folders: vec!["Bagnon".to_string(), "Bagnon_Config".to_string()],
            }),
        )
    }

    #[test]
    fn the_fields_are_aligned_with_the_users_own_marked() {
        assert_eq!(
            clique().human(Paint::PLAIN),
            [
                "Title     Clique",
                "Notes     My Keys  (yours)",
                "Category  none",
            ]
        );
        let mut report = clique();
        report.fields[1].source = TocValueSource::OverrideNotApplied;
        assert_eq!(
            report.human(Paint::PLAIN)[1],
            "Notes     My Keys  (yours, not in the files)"
        );
    }

    #[test]
    fn a_change_whose_files_could_not_be_read_again_warns() {
        assert!(changed(Some("Bags")).warnings().is_empty());
        let unread = changed(Some("Bags")).with_fields_unread();
        assert_eq!(
            unread.warnings(),
            ["The change is saved, but the addon's .toc files could not be read again."]
        );
        // The warning is in the document too.
        let warnings = unread.warnings();
        let document = json_document(&unread, None, &[warnings[0].as_str()]);
        assert!(
            document.trim_end().ends_with(
                r#""warnings":["The change is saved, but the addon's .toc files could not be read again."]}"#
            ),
            "{document}"
        );
    }

    #[test]
    fn a_saved_field_says_what_it_is_now_and_a_cleared_one_what_came_back() {
        assert_eq!(
            changed(Some("Bags")).human(Paint::PLAIN),
            [
                "✓ Saved Category for Bagnon: Bags",
                "/reload to take effect in game"
            ]
        );
        assert_eq!(
            changed(None).human(Paint::PLAIN),
            ["✓ Restored Bagnon's Category"]
        );
        assert_eq!(
            changed(None).human(Paint::PLAIN.with_symbols(Symbols::SIMPLE)),
            ["ok Restored Bagnon's Category"]
        );
    }

    #[test]
    fn colored_lines_have_a_green_done_mark_and_only_the_16_colors() {
        let green = "\x1b[38;5;2m";
        let reset = "\x1b[0m";
        let saved = changed(Some("Bags")).human(Paint::terminal());
        assert!(
            saved[0].starts_with(&format!("{green}✓{reset} ")),
            "{saved:?}"
        );
        assert_eq!(saved[1], "\x1b[2m/reload to take effect in game\x1b[0m");
        let shown = clique().human(Paint::terminal());
        // The field names are bold, and nothing heads the rows.
        assert_eq!(shown[0], "\x1b[1mTitle\x1b[0m     Clique");
        assert!(
            shown[1].starts_with("\x1b[1mNotes\x1b[0m     My Keys"),
            "{shown:?}"
        );
        assert!(shown[1].ends_with("\x1b[2m(yours)\x1b[0m"), "{shown:?}");
        for lines in [&saved, &shown] {
            let joined = lines.join("\n");
            assert!(!joined.contains("38;2;"), "24-bit color: {joined:?}");
            assert!(!joined.contains("48;"), "background: {joined:?}");
        }
        let stripped = shown.iter().map(|line| strip(line)).collect::<Vec<_>>();
        assert_eq!(stripped, clique().human(Paint::PLAIN));
    }

    #[test]
    fn the_json_document_has_every_field_and_the_change() {
        let addon_dir = Path::new("/games/wow/_retail_/Interface/AddOns");
        assert_eq!(
            json_document(&clique(), Some(addon_dir), &[]),
            concat!(
                r#"{"schema":1,"command":"toc","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","#,
                r#""addon":"Clique","folder":"Clique","fields":["#,
                r#"{"field":"Title","value":"|cff00ff00Clique|r","source":"addon"},"#,
                r#"{"field":"Notes","value":"My Keys","source":"override"},"#,
                r#"{"field":"Category","value":null,"source":"addon"}"#,
                r#"],"changed":null,"warnings":[]}"#,
                "\n"
            )
        );
        assert!(
            json_document(&changed(Some("Bags")), Some(addon_dir), &[]).contains(
                r#""changed":{"field":"Category","value":"Bags","folders":["Bagnon","Bagnon_Config"]}"#
            )
        );
        assert!(
            json_document(&changed(None), Some(addon_dir), &[]).contains(
                r#""changed":{"field":"Category","value":null,"folders":["Bagnon","Bagnon_Config"]}"#
            )
        );
    }
}
