//! Writes the user's overrides of a few TOC display fields (`Title`, `Notes`, `Category`) into
//! the root `.toc` files of an addon folder, and takes them out again.
//!
//! The edit works on raw bytes, line by line, so a TOC keeps its line endings, its UTF-8 BOM, a
//! missing final newline and any bytes that are not valid UTF-8. Applying an override:
//!
//! 1. restores the file first (see [`restore`]), so applying is idempotent;
//! 2. turns every directive line for an overridden field, base or localized (`## Title:`,
//!    `## Title-deDE:`), into `# LemonUp: <original line>`, so the game ignores it;
//! 3. inserts `# LemonUp-managed` followed by `## <Field>: <value>` right after the last `##` line
//!    of the header block, using the file's line ending.
//!
//! Restoring removes every `# LemonUp-managed` line together with the line after it and turns
//! every `# LemonUp: X` line back into `X`, which gives back the original bytes exactly. A file
//! LemonUp never touched is left as it is.
//!
//! Only root `.toc` files are edited (the client loads one of them per addon folder, depending on
//! the game version); TOC files in subfolders such as `Libs` are never touched.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::error::{LemonupError, Result};

/// Prefix of an original directive line LemonUp commented out.
pub const COMMENT_PREFIX: &[u8] = b"# LemonUp: ";
/// Line placed before each directive LemonUp added. The line after it is LemonUp's.
pub const MANAGED_MARKER: &[u8] = b"# LemonUp-managed";
/// Longest override value accepted, in characters.
pub const MAX_VALUE_CHARS: usize = 255;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// A TOC field the user can override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TocField {
    Title,
    Notes,
    Category,
}

impl TocField {
    pub const ALL: [TocField; 3] = [TocField::Title, TocField::Notes, TocField::Category];

    /// The directive name, also the key the override is stored under.
    pub fn name(self) -> &'static str {
        match self {
            TocField::Title => "Title",
            TocField::Notes => "Notes",
            TocField::Category => "Category",
        }
    }

    /// The field stored under `name`. Keys are matched exactly; unknown keys (from a newer
    /// LemonUp, for example) are `None` and are kept but never acted on.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.name() == name)
    }
}

/// Checks an override value and returns it trimmed: it must not be empty, must not contain
/// control characters or line breaks, and must be at most [`MAX_VALUE_CHARS`] characters.
pub fn validate_override_value(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(LemonupError::InvalidArgument(
            "the value is empty; clear the field instead".to_string(),
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(LemonupError::InvalidArgument(
            "the value can't contain line breaks, tabs or other control characters".to_string(),
        ));
    }
    let length = trimmed.chars().count();
    if length > MAX_VALUE_CHARS {
        return Err(LemonupError::InvalidArgument(format!(
            "the value is {length} characters long; the limit is {MAX_VALUE_CHARS}"
        )));
    }
    Ok(trimmed.to_string())
}

/// Sets (`Some`) or clears (`None`) one field in a stored overrides map. A value is validated
/// and stored trimmed; other keys, unknown ones included, are left as they are.
pub fn set_override(
    overrides: &mut BTreeMap<String, String>,
    field: TocField,
    value: Option<&str>,
) -> Result<()> {
    match value {
        Some(value) => {
            let value = validate_override_value(value)?;
            overrides.insert(field.name().to_string(), value);
        }
        None => {
            overrides.remove(field.name());
        }
    }
    Ok(())
}

/// The known fields of a stored overrides map with validated values, in [`TocField::ALL`] order.
fn known_overrides(overrides: &BTreeMap<String, String>) -> Result<Vec<(TocField, String)>> {
    let mut fields = Vec::new();
    for field in TocField::ALL {
        if let Some(value) = overrides.get(field.name()) {
            let value = validate_override_value(value).map_err(|error| {
                LemonupError::InvalidArgument(format!("{} override: {error}", field.name()))
            })?;
            fields.push((field, value));
        }
    }
    Ok(fields)
}

#[derive(Debug, Clone)]
struct Line {
    content: Vec<u8>,
    ending: &'static [u8],
}

fn split_bom(bytes: &[u8]) -> (bool, &[u8]) {
    match bytes.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    }
}

/// Splits into lines, each with its own ending (`\r\n`, `\n`, or none for a last line without a
/// final newline).
fn split_lines(bytes: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match rest.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                let (content, ending): (&[u8], &'static [u8]) =
                    if index > 0 && rest[index - 1] == b'\r' {
                        (&rest[..index - 1], b"\r\n")
                    } else {
                        (&rest[..index], b"\n")
                    };
                lines.push(Line {
                    content: content.to_vec(),
                    ending,
                });
                rest = &rest[index + 1..];
            }
            None => {
                lines.push(Line {
                    content: rest.to_vec(),
                    ending: b"",
                });
                rest = &[];
            }
        }
    }
    lines
}

fn join_lines(bom: bool, lines: &[Line]) -> Vec<u8> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(BOM);
    }
    for line in lines {
        bytes.extend_from_slice(&line.content);
        bytes.extend_from_slice(line.ending);
    }
    bytes
}

fn trim_ascii_space(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| *byte != b' ' && *byte != b'\t')
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| *byte != b' ' && *byte != b'\t')
        .map_or(start, |index| index + 1);
    &bytes[start..end.max(start)]
}

/// The field a directive line sets, base or localized (`## Title:`, `## Title-deDE:`). The line
/// must start in column 0, like the game requires; the name is matched without case.
fn directive_field(content: &[u8]) -> Option<TocField> {
    let rest = content.strip_prefix(b"##")?;
    let colon = rest.iter().position(|byte| *byte == b':')?;
    let key = trim_ascii_space(&rest[..colon]);
    let base = match key.iter().position(|byte| *byte == b'-') {
        Some(dash) => &key[..dash],
        None => key,
    };
    TocField::ALL
        .into_iter()
        .find(|field| base.eq_ignore_ascii_case(field.name().as_bytes()))
}

/// Where LemonUp's lines go: right after the last `##` line of the header block (the leading run
/// of `#` and blank lines before the first file line), or at the top when there is none.
fn insert_index(lines: &[Line]) -> usize {
    let mut index = 0;
    for (position, line) in lines.iter().enumerate() {
        let trimmed = trim_ascii_space(&line.content);
        if trimmed.is_empty() {
            continue;
        }
        if !line.content.starts_with(b"#") {
            break;
        }
        if line.content.starts_with(b"##") {
            index = position + 1;
        }
    }
    index
}

/// Writes `overrides` (field name to value; unknown names are ignored) into one TOC file's bytes.
/// With no known field set this is [`restore`].
pub fn apply_overrides(bytes: &[u8], overrides: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let fields = known_overrides(overrides)?;
    let restored = restore(bytes);
    if fields.is_empty() {
        return Ok(restored);
    }

    let (bom, body) = split_bom(&restored);
    let mut lines = split_lines(body);
    let eol: &'static [u8] = lines
        .iter()
        .map(|line| line.ending)
        .find(|ending| !ending.is_empty())
        .unwrap_or(b"\n");
    let at = insert_index(&lines);

    for line in &mut lines {
        if directive_field(&line.content)
            .is_some_and(|field| fields.iter().any(|(overridden, _)| *overridden == field))
        {
            let mut commented = COMMENT_PREFIX.to_vec();
            commented.extend_from_slice(&line.content);
            line.content = commented;
        }
    }

    let mut block = Vec::with_capacity(fields.len() * 2);
    for (field, value) in &fields {
        block.push(Line {
            content: MANAGED_MARKER.to_vec(),
            ending: eol,
        });
        block.push(Line {
            content: format!("## {}: {value}", field.name()).into_bytes(),
            ending: eol,
        });
    }
    // Added after a last line without a final newline: that line gets one and LemonUp's last
    // line goes without, so restoring can tell and give the original bytes back.
    if at == lines.len()
        && at > 0
        && lines[at - 1].ending.is_empty()
        && let Some(last) = block.last_mut()
    {
        lines[at - 1].ending = eol;
        last.ending = b"";
    }
    lines.splice(at..at, block);
    Ok(join_lines(bom, &lines))
}

/// Takes LemonUp's lines out of one TOC file's bytes and uncomments the original lines.
pub fn restore(bytes: &[u8]) -> Vec<u8> {
    let (bom, body) = split_bom(bytes);
    let lines = split_lines(body);
    if !lines
        .iter()
        .any(|line| line.content == MANAGED_MARKER || line.content.starts_with(COMMENT_PREFIX))
    {
        return bytes.to_vec();
    }

    let mut restored = Vec::with_capacity(lines.len());
    let mut ends_without_newline = false;
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        if line.content == MANAGED_MARKER {
            let last_removed = lines.get(index + 1).unwrap_or(line);
            ends_without_newline = index + 2 >= lines.len() && last_removed.ending.is_empty();
            index += 2;
            continue;
        }
        let content = line
            .content
            .strip_prefix(COMMENT_PREFIX)
            .unwrap_or(&line.content);
        restored.push(Line {
            content: content.to_vec(),
            ending: line.ending,
        });
        index += 1;
    }
    if ends_without_newline && let Some(last) = restored.last_mut() {
        last.ending = b"";
    }
    join_lines(bom, &restored)
}

/// Writes `overrides` into every root `.toc` file of `folder` (restoring them when no known field
/// is set) and returns the names of the files that changed. TOC files in subfolders are never
/// touched.
///
/// Each file is replaced through a temporary file and a rename inside `folder`. Only call this
/// on a staged copy (see the app's folder swap), never on a live addon folder.
pub fn apply_overrides_to_folder(
    folder: &Path,
    overrides: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    known_overrides(overrides)?;
    let mut changed = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if !name
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".toc")
        {
            continue;
        }
        let path = entry.path();
        let original = fs::read(&path)?;
        let updated = apply_overrides(&original, overrides)?;
        if updated == original {
            continue;
        }
        let mut temp_name = OsString::from(".");
        temp_name.push(&name);
        temp_name.push(".lemonup-tmp");
        let temp = folder.join(temp_name);
        if let Err(error) = fs::write(&temp, &updated).and_then(|()| fs::rename(&temp, &path)) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        changed.push(name.to_string_lossy().into_owned());
    }
    changed.sort();
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use tempfile::tempdir;

    use super::{
        MAX_VALUE_CHARS, TocField, apply_overrides, apply_overrides_to_folder, restore,
        set_override, validate_override_value,
    };

    fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn all_fields() -> BTreeMap<String, String> {
        overrides(&[
            ("Title", "My Title"),
            ("Notes", "My notes"),
            ("Category", "Mine"),
        ])
    }

    fn assert_round_trip(original: &[u8]) {
        for set in [
            all_fields(),
            overrides(&[("Title", "Only Title")]),
            overrides(&[("Category", "Only Category")]),
        ] {
            let applied = apply_overrides(original, &set).expect("apply");
            assert_ne!(applied, original, "apply changed nothing");
            assert_eq!(
                apply_overrides(&applied, &set).expect("apply again"),
                applied,
                "apply is not idempotent"
            );
            assert_eq!(
                restore(&applied),
                original,
                "restore did not give the original"
            );
            assert_eq!(
                apply_overrides(&applied, &BTreeMap::new()).expect("clear"),
                original
            );
        }
    }

    #[test]
    fn apply_then_restore_gives_back_the_original_bytes() {
        let cases: &[&[u8]] = &[
            b"## Interface: 110002\n## Title: Foo\n## Notes: Bar\n\nFoo.lua\n",
            b"## Interface: 110002\r\n## Title: Foo\r\n## Category: Combat\r\n\r\nFoo.lua\r\n",
            b"\xEF\xBB\xBF## Title: Foo\n## Interface: 110002\nFoo.lua\n",
            b"\xEF\xBB\xBF## Title: Foo\r\n## Interface: 110002",
            b"## Interface: 110002\n## Title: Foo",
            b"## Interface: 110002\n## Title: Foo\nFoo.lua",
            b"## Interface: 110002\n## Author: Caf\xE9 Crew\n## Title: Foo\n# just a comment\nCaf\xE9.lua\n",
            b"Foo.lua\nBar.lua\n",
            b"",
            b"\xEF\xBB\xBF",
            b"## Title: Foo\n## Title-deDE: Fuu\n## Notes-frFR: Note\n## Category-deDE: Kampf\nFoo.lua\n",
            b"## Title: First\nFoo.lua\n## Title: Late\n",
        ];
        for case in cases {
            assert_round_trip(case);
        }
    }

    #[test]
    fn apply_comments_base_and_localized_lines_and_adds_the_override_last_in_the_header() {
        let original =
            b"## Interface: 110002\n## Title: Foo\n## title-deDE: Fuu\n## Notes: Keep\n\nFoo.lua\n";
        let applied =
            apply_overrides(original, &overrides(&[("Title", "  Bar  ")])).expect("apply");
        assert_eq!(
            String::from_utf8(applied).expect("utf8"),
            "## Interface: 110002\n# LemonUp: ## Title: Foo\n# LemonUp: ## title-deDE: Fuu\n## Notes: Keep\n# LemonUp-managed\n## Title: Bar\n\nFoo.lua\n"
        );
    }

    #[test]
    fn apply_uses_the_file_line_ending_and_keeps_bom_and_missing_final_newline() {
        let original = b"\xEF\xBB\xBF## Interface: 110002\r\n## Category: Combat";
        let applied =
            apply_overrides(original, &overrides(&[("Category", "Mine")])).expect("apply");
        assert_eq!(
            applied,
            b"\xEF\xBB\xBF## Interface: 110002\r\n# LemonUp: ## Category: Combat\r\n# LemonUp-managed\r\n## Category: Mine".to_vec()
        );
    }

    #[test]
    fn apply_without_header_puts_the_override_at_the_top() {
        let applied =
            apply_overrides(b"Foo.lua\n", &overrides(&[("Notes", "Hello")])).expect("apply");
        assert_eq!(
            applied,
            b"# LemonUp-managed\n## Notes: Hello\nFoo.lua\n".to_vec()
        );
    }

    #[test]
    fn apply_ignores_unknown_keys_and_indented_lines() {
        let original = b"## Interface: 1\n  ## Title: Indented\n";
        let applied =
            apply_overrides(original, &overrides(&[("X-Future", "kept")])).expect("apply");
        assert_eq!(applied, original.to_vec());
        let applied = apply_overrides(original, &overrides(&[("Title", "New")])).expect("apply");
        assert_eq!(
            applied,
            b"## Interface: 1\n# LemonUp-managed\n## Title: New\n  ## Title: Indented\n".to_vec()
        );
    }

    #[test]
    fn restore_leaves_untouched_files_alone() {
        let original = b"## Title: Foo\r\nFoo.lua";
        assert_eq!(restore(original), original.to_vec());
    }

    #[test]
    fn validation_rejects_empty_control_and_long_values() {
        assert!(validate_override_value("   ").is_err());
        assert!(validate_override_value("two\nlines").is_err());
        assert!(validate_override_value("tab\there").is_err());
        assert!(validate_override_value(&"x".repeat(MAX_VALUE_CHARS + 1)).is_err());
        assert_eq!(
            validate_override_value(&"é".repeat(MAX_VALUE_CHARS)).expect("max length"),
            "é".repeat(MAX_VALUE_CHARS)
        );
        assert_eq!(
            validate_override_value("  |cff00ff00Green|r  ").expect("valid"),
            "|cff00ff00Green|r"
        );
        assert!(apply_overrides(b"## Title: Foo\n", &overrides(&[("Title", "a\rb")])).is_err());

        let mut stored = overrides(&[("X-Future", "kept")]);
        assert!(set_override(&mut stored, TocField::Notes, Some("")).is_err());
        set_override(&mut stored, TocField::Notes, Some(" Note ")).expect("set");
        assert_eq!(stored.get("Notes").map(String::as_str), Some("Note"));
        set_override(&mut stored, TocField::Notes, None).expect("clear");
        assert_eq!(stored, overrides(&[("X-Future", "kept")]));
    }

    #[test]
    fn folder_rewrites_every_root_toc_and_never_subfolder_tocs() {
        let temp = tempdir().expect("tempdir");
        let folder = temp.path().join("Foo");
        fs::create_dir_all(folder.join("Libs").join("LibStub")).expect("dirs");
        let base = b"## Interface: 110002\n## Title: Foo\nFoo.lua\n".to_vec();
        let mainline = b"## Interface: 110002\r\n## Title: Foo Mainline\r\n".to_vec();
        let forever = b"## Interface: 16001\n## Title: Foo Forever\n".to_vec();
        let lib = b"## Title: LibStub\n".to_vec();
        fs::write(folder.join("Foo.toc"), &base).expect("toc");
        fs::write(folder.join("Foo_Mainline.TOC"), &mainline).expect("toc");
        fs::write(folder.join("Foo_Forever.toc"), &forever).expect("toc");
        fs::write(folder.join("Foo.lua"), b"## Title: not a toc\n").expect("lua");
        fs::write(folder.join("Libs/LibStub/LibStub.toc"), &lib).expect("lib toc");

        let changed =
            apply_overrides_to_folder(&folder, &overrides(&[("Title", "Bar")])).expect("apply");
        assert_eq!(changed, ["Foo.toc", "Foo_Forever.toc", "Foo_Mainline.TOC"]);
        for name in ["Foo.toc", "Foo_Mainline.TOC", "Foo_Forever.toc"] {
            let text = fs::read_to_string(folder.join(name)).expect("read");
            assert!(text.contains("# LemonUp-managed"), "{name}: {text}");
            assert!(text.contains("## Title: Bar"), "{name}: {text}");
        }
        assert_eq!(
            fs::read(folder.join("Libs/LibStub/LibStub.toc")).expect("lib"),
            lib
        );
        assert_eq!(
            fs::read(folder.join("Foo.lua")).expect("lua"),
            b"## Title: not a toc\n"
        );
        let leftovers = fs::read_dir(&folder)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".lemonup-tmp")
            })
            .count();
        assert_eq!(leftovers, 0);

        let restored = apply_overrides_to_folder(&folder, &BTreeMap::new()).expect("restore");
        assert_eq!(restored.len(), 3);
        assert_eq!(fs::read(folder.join("Foo.toc")).expect("base"), base);
        assert_eq!(
            fs::read(folder.join("Foo_Mainline.TOC")).expect("mainline"),
            mainline
        );
        assert_eq!(
            fs::read(folder.join("Foo_Forever.toc")).expect("forever"),
            forever
        );
        assert!(
            apply_overrides_to_folder(&folder, &BTreeMap::new())
                .expect("no-op")
                .is_empty()
        );
    }

    #[test]
    fn folder_refuses_an_invalid_stored_value_before_writing() {
        let temp = tempdir().expect("tempdir");
        let original = b"## Title: Foo\n".to_vec();
        fs::write(temp.path().join("Foo.toc"), &original).expect("toc");
        assert!(apply_overrides_to_folder(temp.path(), &overrides(&[("Title", "")])).is_err());
        assert_eq!(
            fs::read(temp.path().join("Foo.toc")).expect("toc"),
            original
        );
    }
}
