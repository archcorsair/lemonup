//! `lemonup restore-backup`: the backup of your game settings that was put back.

use std::path::PathBuf;

use serde::Serialize;

use super::{Paint, Report};

/// What `lemonup restore-backup` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RestoreReport {
    #[serde(serialize_with = "super::lossy_path_buf")]
    archive: PathBuf,
}

impl RestoreReport {
    pub(crate) fn new(archive: PathBuf) -> Self {
        Self { archive }
    }
}

impl Report for RestoreReport {
    const COMMAND: &'static str = "restore-backup";

    fn human(&self, paint: Paint) -> Vec<String> {
        vec![format!(
            "{} {}{}",
            paint.done(),
            paint.text("Restored your game settings from "),
            paint.path(&self.archive)
        )]
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::RestoreReport;
    use crate::output::tests::json_document;
    use crate::output::{Paint, Report};
    use crate::theme::Symbols;

    fn report() -> RestoreReport {
        RestoreReport::new(PathBuf::from(
            "/data/backups/retail/WTF-2026-11-22-2140.zip",
        ))
    }

    #[test]
    fn the_line_names_the_backup() {
        assert_eq!(
            report().human(Paint::PLAIN),
            ["✓ Restored your game settings from /data/backups/retail/WTF-2026-11-22-2140.zip"]
        );
        assert!(
            report().human(Paint::PLAIN.with_symbols(Symbols::SIMPLE))[0]
                .starts_with("ok Restored")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_backup_is_a_link_where_the_terminal_has_links() {
        let line = &report().human(Paint::terminal().with_links(true))[0];
        assert!(
            line.ends_with("from \x1b]8;;file:///data/backups/retail/WTF-2026-11-22-2140.zip\x1b\\\x1b[1m/data/backups/retail/WTF-2026-11-22-2140.zip\x1b[0m\x1b]8;;\x1b\\"),
            "{line:?}"
        );
    }

    #[test]
    fn the_done_mark_is_green_and_only_the_16_colors_are_used() {
        let line = &report().human(Paint::terminal())[0];
        assert!(
            line.starts_with("\x1b[38;5;2m✓\x1b[0m Restored"),
            "{line:?}"
        );
        // The backup's path is bold.
        assert!(
            line.ends_with("from \x1b[1m/data/backups/retail/WTF-2026-11-22-2140.zip\x1b[0m"),
            "{line:?}"
        );
        assert!(!line.contains("38;2;"), "{line:?}");
        assert!(!line.contains("48;"), "{line:?}");
    }

    #[test]
    fn the_json_document_has_the_archive() {
        assert_eq!(
            json_document(
                &report(),
                Some(Path::new("/games/wow/_retail_/Interface/AddOns")),
                &[]
            ),
            concat!(
                r#"{"schema":1,"command":"restore-backup","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","#,
                r#""archive":"/data/backups/retail/WTF-2026-11-22-2140.zip","warnings":[]}"#,
                "\n"
            )
        );
    }
}
