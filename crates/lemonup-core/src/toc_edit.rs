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
//! Restoring removes every `# LemonUp-managed` line, together with the line after it when that is
//! a base `## Title:`, `## Notes:` or `## Category:` line (any other line after a marker is kept),
//! and turns every `# LemonUp: X` line back into `X`, which gives back the original bytes exactly.
//! A file LemonUp never touched is left as it is.
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
/// Longest TOC line the game reads, in bytes, without its line ending.
pub const MAX_LINE_BYTES: usize = 1024;
/// Longest override value accepted, in bytes: `## Category: <value>`, the longest line LemonUp
/// writes, stays within [`MAX_LINE_BYTES`].
pub const MAX_VALUE_BYTES: usize = MAX_LINE_BYTES - "## Category: ".len();

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
/// control characters or line breaks, and must be at most [`MAX_VALUE_CHARS`] characters and
/// [`MAX_VALUE_BYTES`] bytes.
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
    if trimmed.len() > MAX_VALUE_BYTES {
        return Err(LemonupError::InvalidArgument(format!(
            "the value is {} bytes long, too long for a TOC line; the limit is {MAX_VALUE_BYTES}",
            trimmed.len()
        )));
    }
    Ok(trimmed.to_string())
}

/// Checks the known fields of a stored overrides map the way they are stored: each value must
/// pass [`validate_override_value`] and already be trimmed. Unknown keys are not checked.
pub fn validate_stored_overrides(overrides: &BTreeMap<String, String>) -> Result<()> {
    for field in TocField::ALL {
        if let Some(value) = overrides.get(field.name()) {
            let checked = validate_override_value(value).map_err(|error| {
                LemonupError::InvalidArgument(format!("{} override: {error}", field.name()))
            })?;
            if checked != *value {
                return Err(LemonupError::InvalidArgument(format!(
                    "{} override has leading or trailing spaces",
                    field.name()
                )));
            }
        }
    }
    Ok(())
}

/// Splits a stored overrides map into what this version can write and a message for each known
/// field it has to skip ("Skipped the Title override: ..."), so a value an older or newer
/// LemonUp stored never blocks an install or update. Unknown keys are kept in the usable map
/// (writing ignores them).
pub fn usable_overrides(
    overrides: &BTreeMap<String, String>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut usable = overrides.clone();
    let mut skipped = Vec::new();
    for field in TocField::ALL {
        if let Some(value) = overrides.get(field.name())
            && let Err(error) = validate_override_value(value)
        {
            usable.remove(field.name());
            skipped.push(format!("Skipped the {} override: {error}", field.name()));
        }
    }
    (usable, skipped)
}

/// The addon name a stored `Title` override gives (color codes removed), or `None` without a
/// usable one. An addon with a Title override is listed under it, after installs, updates and
/// scans alike.
pub fn title_override_name(overrides: &BTreeMap<String, String>) -> Option<String> {
    let value = overrides.get(TocField::Title.name())?;
    let value = validate_override_value(value).ok()?;
    Some(crate::scan::strip_wow_color_codes(value)).filter(|name| !name.trim().is_empty())
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

/// The headings the game's AddOns list offers, for suggestions while the user types a Category.
pub const STANDARD_CATEGORIES: [&str; 27] = [
    "Action Bars",
    "Auctions",
    "Buffs & Debuffs",
    "Chat",
    "Collections",
    "Combat",
    "Data Broker",
    "Development Tools",
    "Dungeons & Raids",
    "Equipment",
    "Expansion Features",
    "Housing",
    "Inventory",
    "Libraries",
    "Loot",
    "Map",
    "Media",
    "Minigames",
    "Other",
    "Pet Battles",
    "Professions",
    "PvP",
    "Quests",
    "Roleplay",
    "UI Overhaul",
    "Unit Frames",
    "User Interface",
];

/// The values the game reads for the fields an addon's TOC sets, as a scan found them (see
/// [`crate::TocMetadata`]). `None` when the TOC has no such line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TocFieldValues {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub category: Option<String>,
}

impl TocFieldValues {
    pub fn from_scanned(addon: &crate::ScannedAddon) -> Self {
        Self {
            title: addon.toc_title.clone(),
            notes: addon.notes.clone(),
            category: addon.category.clone(),
        }
    }

    pub fn get(&self, field: TocField) -> Option<&str> {
        match field {
            TocField::Title => self.title.as_deref(),
            TocField::Notes => self.notes.as_deref(),
            TocField::Category => self.category.as_deref(),
        }
        .filter(|value| !value.trim().is_empty())
    }
}

/// Where the value the game uses for a field comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocValueSource {
    /// No override: the addon's own line, or none.
    Addon,
    /// The user's override, and the files have it.
    Override,
    /// An override is stored but the files say something else (another program rewrote the
    /// TOC, for example), so the game does not show it.
    OverrideNotApplied,
}

impl TocValueSource {
    /// The word the CLI prints.
    pub fn cli_name(self) -> &'static str {
        match self {
            Self::Addon => "addon",
            Self::Override => "override",
            Self::OverrideNotApplied => "override_not_applied",
        }
    }
}

/// Where `field`'s value comes from, given the stored `overrides` and the `effective` value a
/// scan read. A Title is compared without color codes; every value without surrounding spaces.
pub fn value_source(
    field: TocField,
    overrides: &BTreeMap<String, String>,
    effective: Option<&str>,
) -> TocValueSource {
    let Some(wanted) = overrides.get(field.name()) else {
        return TocValueSource::Addon;
    };
    let normalize = |value: &str| match field {
        TocField::Title => display_value(value),
        TocField::Notes | TocField::Category => value.trim().to_string(),
    };
    if effective.is_some_and(|effective| normalize(effective) == normalize(wanted)) {
        TocValueSource::Override
    } else {
        TocValueSource::OverrideNotApplied
    }
}

/// `value` as a person reads it: without WoW color codes and surrounding spaces.
pub fn display_value(value: &str) -> String {
    crate::scan::strip_wow_color_codes(value.to_string())
        .trim()
        .to_string()
}

/// Up to `limit` Category suggestions for `typed`: the headings in `used` (what other addons
/// already use) and [`STANDARD_CATEGORIES`], each once whatever its case. Those that start with
/// `typed` come first, then those that contain it, alphabetically within each; case is ignored.
pub fn category_suggestions<'a>(
    typed: &str,
    used: impl IntoIterator<Item = &'a str>,
    limit: usize,
) -> Vec<String> {
    let typed = typed.trim().to_lowercase();
    let mut candidates: Vec<String> = Vec::new();
    for candidate in used.into_iter().chain(STANDARD_CATEGORIES) {
        let candidate = candidate.trim();
        if candidate.is_empty()
            || candidates
                .iter()
                .any(|known| known.to_lowercase() == candidate.to_lowercase())
        {
            continue;
        }
        candidates.push(candidate.to_string());
    }
    candidates.sort_by_key(|candidate| candidate.to_lowercase());
    let (mut starts, contains): (Vec<String>, Vec<String>) = candidates
        .into_iter()
        .filter(|candidate| candidate.to_lowercase().contains(&typed))
        .partition(|candidate| candidate.to_lowercase().starts_with(&typed));
    starts.extend(contains);
    starts.truncate(limit);
    starts
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

/// Whether a line after a `# LemonUp-managed` marker is one LemonUp wrote: a base (not localized)
/// `## Title:`, `## Notes:` or `## Category:` directive, name matched without case.
fn is_managed_directive(content: &[u8]) -> bool {
    let Some(rest) = content.strip_prefix(b"##") else {
        return false;
    };
    let Some(colon) = rest.iter().position(|byte| *byte == b':') else {
        return false;
    };
    let key = trim_ascii_space(&rest[..colon]);
    TocField::ALL
        .into_iter()
        .any(|field| key.eq_ignore_ascii_case(field.name().as_bytes()))
}

/// The ending to give a last line that had none. A line whose content ends in a lone `\r` gets
/// `\r\n`: a plain `\n` would join that `\r` into a CRLF ending, and restoring would drop it.
fn ending_for_last_line(content: &[u8], eol: &'static [u8]) -> &'static [u8] {
    if content.ends_with(b"\r") {
        b"\r\n"
    } else {
        eol
    }
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
        lines[at - 1].ending = ending_for_last_line(&lines[at - 1].content, eol);
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
            // Only LemonUp's own directive goes with the marker; any other line stays.
            let removed = match lines.get(index + 1) {
                Some(next) if is_managed_directive(&next.content) => 2,
                _ => 1,
            };
            let last_removed = &lines[index + removed - 1];
            ends_without_newline = index + removed >= lines.len() && last_removed.ending.is_empty();
            index += removed;
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
        MAX_LINE_BYTES, MAX_VALUE_BYTES, MAX_VALUE_CHARS, TocField, apply_overrides,
        apply_overrides_to_folder, restore, set_override, title_override_name, usable_overrides,
        validate_override_value, validate_stored_overrides,
    };
    use super::{TocValueSource, category_suggestions, display_value, value_source};

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
            b"## Title: Foo\r",
            b"## Interface: 110002\n## Title: Foo\r",
            b"## Interface: 110002\r\n## Notes: Bar\r",
            b"\xEF\xBB\xBF## Category: Combat\r",
            b"## Interface: 110002\n## Title: Foo\nFoo.lua\r",
            b"\r",
        ];
        for case in cases {
            assert_round_trip(case);
        }
    }

    /// The header of ElvUI's root TOC (v16.01), with one Title line per game version.
    const ELVUI_TOC: &[u8] = b"## Interface: 11509, 16001, 20506, 38002, 50504, 120100, 120105\r\n## Version: v16.01\r\n\r\n## Notes: User Interface Replacement\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bMists|r [AllowLoadGameType mists]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bMidnight|r [AllowLoadGameType standard]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bForever|r [AllowLoadGameType camelot][ExcludeLoadGameType standard, classic]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bWrath|r [AllowLoadGameType wrath]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bTBC|r [AllowLoadGameType tbc]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bClassic|r [AllowLoadGameType vanilla]\r\n\r\n## Category-enUS: UI Overhaul\r\n\r\nElvUI.lua\r\n";

    #[test]
    fn titles_for_each_game_version_are_overridden_and_restored_byte_for_byte() {
        use crate::domain::GameFlavor;
        use crate::scan::parse_toc_content;

        assert_round_trip(ELVUI_TOC);
        let read = |bytes: &[u8], flavor| {
            parse_toc_content(&String::from_utf8_lossy(bytes), "ElvUI", Some(flavor))
        };

        // Before an override, the editor shows the Forever line, from the addon.
        let none = BTreeMap::new();
        let original = read(ELVUI_TOC, GameFlavor::Forever);
        assert_eq!(
            original.toc_title.as_deref(),
            Some("|cff1784d1ElvUI|r |cfd9b9b9bForever|r")
        );
        assert_eq!(
            value_source(TocField::Title, &none, original.toc_title.as_deref()),
            TocValueSource::Addon
        );
        assert_eq!(
            display_value(original.toc_title.as_deref().unwrap_or_default()),
            "ElvUI Forever"
        );

        // An override comments out every Title line, for every game version, and LemonUp's
        // unconditional line is the only one left, so it wins in every game version.
        let mine = overrides(&[("Title", "My UI")]);
        let applied = apply_overrides(ELVUI_TOC, &mine).expect("apply");
        let text = String::from_utf8_lossy(&applied);
        assert_eq!(
            text.matches("\r\n# LemonUp: ## Title: ").count(),
            6,
            "{text}"
        );
        assert_eq!(text.matches("\r\n## Title: ").count(), 1, "{text}");
        for flavor in [GameFlavor::Forever, GameFlavor::Retail, GameFlavor::Classic] {
            let parsed = read(&applied, flavor);
            assert_eq!(parsed.title, "My UI");
            assert_eq!(parsed.toc_title.as_deref(), Some("My UI"));
            assert_eq!(
                value_source(TocField::Title, &mine, parsed.toc_title.as_deref()),
                TocValueSource::Override
            );
        }
        assert_eq!(restore(&applied), ELVUI_TOC);
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
    fn restore_removes_only_lemonup_directives_after_a_marker() {
        let damaged =
            b"## Interface: 110002\n## Title: Foo\n# LemonUp-managed\nCore.lua\nFoo.lua\n";
        assert_eq!(
            restore(damaged),
            b"## Interface: 110002\n## Title: Foo\nCore.lua\nFoo.lua\n".to_vec()
        );
        let localized =
            b"# LemonUp-managed\n## Title-deDE: Fuu\n# LemonUp-managed\n##  notes : Mine\n";
        assert_eq!(restore(localized), b"## Title-deDE: Fuu\n".to_vec());
        let other = b"# LemonUp-managed\n## Author: Someone\nFoo.lua";
        assert_eq!(restore(other), b"## Author: Someone\nFoo.lua".to_vec());
    }

    #[test]
    fn a_last_line_ending_in_a_lone_carriage_return_keeps_it() {
        let original = b"## Interface: 110002\n## Title: Foo\r";
        let applied = apply_overrides(original, &overrides(&[("Title", "Bar")])).expect("apply");
        assert_eq!(
            applied,
            b"## Interface: 110002\n# LemonUp: ## Title: Foo\r\r\n# LemonUp-managed\n## Title: Bar"
                .to_vec()
        );
        assert_eq!(restore(&applied), original.to_vec());
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

        // Four-byte characters reach the byte limit before the character limit.
        let clef = "\u{1D11E}";
        let fits = clef.repeat(MAX_VALUE_BYTES / clef.len());
        let line = format!(
            "## Category: {}",
            validate_override_value(&fits).expect("fits")
        );
        assert!(line.len() <= MAX_LINE_BYTES);
        let too_long = clef.repeat(MAX_VALUE_BYTES / clef.len() + 1);
        assert!(too_long.chars().count() <= MAX_VALUE_CHARS);
        let error = validate_override_value(&too_long).expect_err("too many bytes");
        assert!(error.to_string().contains("bytes"), "{error}");

        let mut stored = overrides(&[("X-Future", "kept")]);
        assert!(set_override(&mut stored, TocField::Notes, Some("")).is_err());
        set_override(&mut stored, TocField::Notes, Some(" Note ")).expect("set");
        assert_eq!(stored.get("Notes").map(String::as_str), Some("Note"));
        set_override(&mut stored, TocField::Notes, None).expect("clear");
        assert_eq!(stored, overrides(&[("X-Future", "kept")]));
    }

    #[test]
    fn stored_values_are_checked_as_stored_and_unusable_ones_are_skipped() {
        assert!(validate_stored_overrides(&overrides(&[("Title", "Foo"), ("X", " y ")])).is_ok());
        assert!(validate_stored_overrides(&overrides(&[("Title", " Foo")])).is_err());
        assert!(validate_stored_overrides(&overrides(&[("Notes", "")])).is_err());

        let (usable, skipped) = usable_overrides(&overrides(&[
            ("Title", ""),
            ("Notes", "Fine"),
            ("X-Future", "kept"),
        ]));
        assert_eq!(
            usable,
            overrides(&[("Notes", "Fine"), ("X-Future", "kept")])
        );
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].starts_with("Skipped the Title override: "),
            "{}",
            skipped[0]
        );
    }

    #[test]
    fn title_override_name_strips_color_codes() {
        assert_eq!(
            title_override_name(&overrides(&[("Title", "|cff00ff00My|r Foo")])).as_deref(),
            Some("My Foo")
        );
        assert_eq!(title_override_name(&overrides(&[("Title", "")])), None);
        assert_eq!(title_override_name(&overrides(&[("Notes", "x")])), None);
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

    #[test]
    fn value_source_says_where_the_value_comes_from() {
        let none = BTreeMap::new();
        assert_eq!(
            value_source(TocField::Category, &none, Some("Combat")),
            TocValueSource::Addon
        );
        let mine = overrides(&[("Category", "Mine"), ("Title", "|cff00ff00My Foo|r")]);
        assert_eq!(
            value_source(TocField::Category, &mine, Some("Mine")),
            TocValueSource::Override
        );
        assert_eq!(
            value_source(TocField::Category, &mine, Some("Combat")),
            TocValueSource::OverrideNotApplied
        );
        assert_eq!(
            value_source(TocField::Category, &mine, None),
            TocValueSource::OverrideNotApplied
        );
        // A Title is compared without its color codes.
        assert_eq!(
            value_source(TocField::Title, &mine, Some("My Foo")),
            TocValueSource::Override
        );
        assert_eq!(
            value_source(TocField::Title, &mine, Some("Foo")),
            TocValueSource::OverrideNotApplied
        );
        assert_eq!(
            TocValueSource::OverrideNotApplied.cli_name(),
            "override_not_applied"
        );
        assert_eq!(display_value(" |cffff0000Red|r Foo "), "Red Foo");
    }

    #[test]
    fn category_suggestions_match_used_and_standard_headings() {
        let used = ["Bags", "combat", "Unit Frames"];
        assert_eq!(
            category_suggestions("un", used, 5),
            vec!["Unit Frames", "Dungeons & Raids"]
        );
        // Each heading once, whatever its case; the used spelling wins.
        let combat = category_suggestions("COMBAT", used, 5);
        assert_eq!(combat, vec!["combat"]);
        assert_eq!(category_suggestions("", used, 5).len(), 5);
        assert_eq!(
            category_suggestions("b", used, 3),
            vec!["Bags", "Buffs & Debuffs", "Action Bars"]
        );
        assert!(category_suggestions("zzz", used, 5).is_empty());
    }
}
