//! `lemonup export-addons`: the file the addon list was saved to.

use std::path::PathBuf;

use lemonup_core::TransferAddon;
use serde::Serialize;

use super::{Paint, Report};

/// What `lemonup export-addons` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExportReport {
    #[serde(serialize_with = "super::lossy_path_buf")]
    file: PathBuf,
    count: usize,
    /// The addons a newer LemonUp linked, with the source it named.
    #[serde(skip)]
    newer_sources: Vec<(String, String)>,
}

impl ExportReport {
    pub(crate) fn new(file: PathBuf, addons: &[TransferAddon]) -> Self {
        Self {
            file,
            count: addons.len(),
            newer_sources: addons
                .iter()
                .filter_map(|addon| {
                    addon
                        .unknown_source()
                        .map(|source| (addon.name.clone(), source.to_string()))
                })
                .collect(),
        }
    }

    /// One warning per addon only a newer LemonUp can reinstall.
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.newer_sources
            .iter()
            .map(|(name, source)| {
                format!(
                    "{name} is linked to '{source}' by a newer version of LemonUp. Only a newer \
                     version can reinstall it."
                )
            })
            .collect()
    }
}

impl Report for ExportReport {
    fn command(&self) -> &'static str {
        "export-addons"
    }

    fn human(&self, paint: Paint) -> Vec<String> {
        let addons = match self.count {
            1 => "1 addon".to_string(),
            count => format!("{count} addons"),
        };
        vec![format!(
            "{} {}{}",
            paint.done(),
            paint.text(&format!("Saved {addons} to ")),
            paint.path(&self.file)
        )]
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{TransferAddon, TransferSourceKind};

    use super::ExportReport;
    use crate::output::tests::json_document;
    use crate::output::{Paint, Report};
    use crate::theme::Symbols;

    fn addon(name: &str, source: TransferSourceKind) -> TransferAddon {
        TransferAddon {
            name: name.to_string(),
            folder: name.to_string(),
            source,
            url: Some(format!("https://example.com/{name}")),
            owned_folders: None,
            reinstallable: true,
        }
    }

    fn report(count: usize) -> ExportReport {
        let addons = (0..count)
            .map(|at| addon(&format!("Addon{at}"), TransferSourceKind::GitHub))
            .collect::<Vec<_>>();
        ExportReport::new(PathBuf::from("/data/lemonup-addons.json"), &addons)
    }

    #[test]
    fn the_line_names_the_count_and_the_file() {
        assert_eq!(
            report(18).human(Paint::PLAIN),
            ["✓ Saved 18 addons to /data/lemonup-addons.json"]
        );
        assert_eq!(
            report(1).human(Paint::PLAIN),
            ["✓ Saved 1 addon to /data/lemonup-addons.json"]
        );
        assert_eq!(
            report(0).human(Paint::PLAIN.with_symbols(Symbols::SIMPLE)),
            ["ok Saved 0 addons to /data/lemonup-addons.json"]
        );
    }

    #[test]
    fn the_done_mark_is_green_and_only_the_16_colors_are_used() {
        let line = &report(2).human(Paint::terminal())[0];
        assert!(line.starts_with("\x1b[38;5;2m✓\x1b[0m Saved"), "{line:?}");
        // The file's path is bold.
        assert!(
            line.ends_with("to \x1b[1m/data/lemonup-addons.json\x1b[0m"),
            "{line:?}"
        );
        assert!(!line.contains("38;2;"), "{line:?}");
        assert!(!line.contains("48;"), "{line:?}");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_a_link_where_the_terminal_has_links() {
        let line = &report(2).human(Paint::terminal().with_links(true))[0];
        assert!(
            line.ends_with("to \x1b]8;;file:///data/lemonup-addons.json\x1b\\\x1b[1m/data/lemonup-addons.json\x1b[0m\x1b]8;;\x1b\\"),
            "{line:?}"
        );
    }

    #[test]
    fn an_addon_from_a_newer_lemonup_is_a_warning() {
        let addons = [
            addon("Bagnon", TransferSourceKind::Wowinterface),
            addon(
                "Plater",
                TransferSourceKind::Other("curseforge".to_string()),
            ),
        ];
        let report = ExportReport::new(PathBuf::from("/data/list.json"), &addons);
        assert_eq!(
            report.warnings(),
            [
                "Plater is linked to 'curseforge' by a newer version of LemonUp. Only a newer \
              version can reinstall it."
            ]
        );
        assert!(self::report(3).warnings().is_empty());
    }

    #[test]
    fn the_json_document_has_the_file_and_the_count() {
        assert_eq!(
            json_document(&report(18), None, &[]),
            concat!(
                r#"{"schema":1,"command":"export-addons","profile":"default","addon_dir":null,"#,
                r#""file":"/data/lemonup-addons.json","count":18,"warnings":[]}"#,
                "\n"
            )
        );
    }
}
