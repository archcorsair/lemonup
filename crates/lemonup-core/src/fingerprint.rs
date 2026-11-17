//! Wago.io-compatible fingerprints of addon folders.
//!
//! Wago.io (like WowUp and CurseBreaker) identifies an installed addon folder by a hash of the
//! files the game loads from it: the folder's root `.toc` files (`<Folder>.toc` and
//! `<Folder>-<flavor>.toc` / `<Folder>_<flavor>.toc`) and its root `Bindings.xml`, plus every
//! `.lua` and `.xml` file a TOC lists and every `<Include file=...>` / `<Script file=...>` an
//! included XML file names, followed recursively. Paths compare without case, like the game, and
//! backslashes are separators. Each file's bytes are hashed with MD5, the lowercase hex digests
//! are sorted and joined, and the joined text is hashed again.
//!
//! The rules (including their quirks) follow the reference script whose hashes Wago.io matched:
//! a listed file that is missing is skipped, `Bindings.xml` is hashed but never followed, and
//! only files whose extension is exactly `.toc` or `.xml` (lowercase) are followed.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

/// How deep the walk goes into an addon folder, so a symlink loop cannot recurse forever.
const MAX_DEPTH: usize = 32;

/// The flavor suffixes Wago.io accepts after `<Folder>-` or `<Folder>_` in a root TOC name.
const TOC_FLAVORS: &[&str] = &[
    "mainline", "bcc", "tbc", "classic", "vanilla", "wrath", "wotlkc", "cata", "mists", "forever",
    "camelot",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFingerprint {
    /// The folder's hash, lowercase hex.
    pub hash: String,
    /// How many files went into the hash. Zero means the folder has no TOC or `Bindings.xml`.
    pub files: usize,
}

/// One file of the addon folder: its path relative to the AddOns folder (components as on
/// disk) and where it is.
struct FolderFile {
    components: Vec<String>,
    path: PathBuf,
}

/// The Wago.io fingerprint of `folder` inside `addons_dir`. Fails only when a file that goes
/// into the hash cannot be read.
pub fn folder_fingerprint(addons_dir: &Path, folder: &str) -> io::Result<FolderFingerprint> {
    let mut files = Vec::new();
    walk(
        &addons_dir.join(folder),
        &mut vec![folder.to_string()],
        &mut files,
        0,
    );
    let by_key = files
        .iter()
        .enumerate()
        .map(|(index, file)| (lowercase_key(&file.components), index))
        .collect::<HashMap<_, _>>();

    let mut matched = BTreeSet::new();
    let mut tocs = Vec::new();
    for (index, file) in files.iter().enumerate() {
        if is_root_toc(&file.components) {
            tocs.push(index);
        } else if is_root_bindings(&file.components) {
            matched.insert(index);
        }
    }
    let mut pending = tocs;
    while let Some(index) = pending.pop() {
        if !matched.insert(index) {
            continue;
        }
        let file = &files[index];
        let name = file.components.last().map(String::as_str).unwrap_or("");
        let includes = if name.ends_with(".toc") {
            toc_includes(&decode(&fs::read(&file.path)?))
        } else if name.ends_with(".xml") {
            xml_includes(&decode(&fs::read(&file.path)?))
        } else {
            continue;
        };
        let directory = &file.components[..file.components.len() - 1];
        for include in includes {
            if let Some(key) = resolve_include(directory, &include)
                && let Some(&found) = by_key.get(&key)
                && !matched.contains(&found)
            {
                pending.push(found);
            }
        }
    }

    let mut digests = Vec::with_capacity(matched.len());
    for index in &matched {
        digests.push(hex(&Md5::digest(fs::read(&files[index.to_owned()].path)?)));
    }
    digests.sort();
    Ok(FolderFingerprint {
        hash: hex(&Md5::digest(digests.concat().as_bytes())),
        files: matched.len(),
    })
}

/// Collects every file under `directory`, following symlinks. Unreadable directories are
/// skipped, as the reference walk does.
fn walk(directory: &Path, components: &mut Vec<String>, files: &mut Vec<FolderFile>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries = entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        components.push(name);
        if path.is_dir() {
            walk(&path, components, files, depth + 1);
        } else {
            files.push(FolderFile {
                components: components.clone(),
                path,
            });
        }
        components.pop();
    }
}

fn lowercase_key(components: &[String]) -> String {
    components.join("/").to_lowercase()
}

/// `<Folder>/<Folder>.toc` or `<Folder>/<Folder>[-_]<flavor>.toc`, without case.
fn is_root_toc(components: &[String]) -> bool {
    let [folder, name] = components else {
        return false;
    };
    let folder = folder.to_lowercase();
    let name = name.to_lowercase();
    let Some(rest) = name.strip_prefix(&folder) else {
        return false;
    };
    let Some(suffix) = rest.strip_suffix(".toc") else {
        return false;
    };
    suffix.is_empty()
        || suffix
            .strip_prefix(['-', '_'])
            .is_some_and(|flavor| TOC_FLAVORS.contains(&flavor))
}

fn is_root_bindings(components: &[String]) -> bool {
    matches!(components, [_, name] if name.eq_ignore_ascii_case("Bindings.xml"))
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The `.lua` and `.xml` files a TOC lists: every line that, without its `#` comment and
/// surrounding whitespace, ends in `.lua` or `.xml` (any case). A path that climbs with `..`
/// is ignored.
fn toc_includes(content: &str) -> Vec<String> {
    content
        .split('\n')
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or("").trim();
            let chars = line.chars().collect::<Vec<_>>();
            if chars.len() <= 4 {
                return None;
            }
            let (stem, extension) = chars.split_at(chars.len() - 4);
            let extension = extension.iter().collect::<String>().to_lowercase();
            if extension != ".lua" && extension != ".xml" {
                return None;
            }
            if climbs(stem) || stem.contains(&'\n') {
                return None;
            }
            Some(line.to_string())
        })
        .collect()
}

/// Whether a path's characters, except the last, contain `..` (the reference pattern rejects
/// any character that follows `..`).
fn climbs(stem: &[char]) -> bool {
    stem.windows(2)
        .take(stem.len().saturating_sub(2))
        .any(|pair| pair == ['.', '.'])
}

/// The files an XML file names in `<Include file="..."/>` and `<Script file="..."/>` (any
/// case), outside `<!-- -->` comments. Like the reference pattern, the value runs to the last
/// closing quote and `/>` on its line that it can reach.
fn xml_includes(content: &str) -> Vec<String> {
    let text = strip_xml_comments(content).chars().collect::<Vec<_>>();
    let mut includes = Vec::new();
    let mut start = 0;
    while start < text.len() {
        match xml_include_at(&text, start) {
            Some((value, end)) => {
                includes.push(value);
                start = end;
            }
            None => start += 1,
        }
    }
    includes
}

fn strip_xml_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(open) = rest.find("<!--") {
        let Some(close) = rest[open + 4..].find("-->") else {
            break;
        };
        out.push_str(&rest[..open]);
        rest = &rest[open + 4 + close + 3..];
    }
    out.push_str(rest);
    out
}

fn starts_with_ignore_case(text: &[char], at: usize, word: &str) -> bool {
    let word = word.chars().collect::<Vec<_>>();
    text.len() >= at + word.len()
        && text[at..at + word.len()]
            .iter()
            .zip(&word)
            .all(|(left, right)| left.to_lowercase().eq(right.to_lowercase()))
}

/// Matches `<(Include|Script)\s+file=["'](value)["']\s*/>` at `at`; returns the value and where
/// the match ends.
fn xml_include_at(text: &[char], at: usize) -> Option<(String, usize)> {
    if text.get(at) != Some(&'<') {
        return None;
    }
    let mut position = at + 1;
    if starts_with_ignore_case(text, position, "include") {
        position += "include".len();
    } else if starts_with_ignore_case(text, position, "script") {
        position += "script".len();
    } else {
        return None;
    }
    let spaces_start = position;
    while text.get(position).is_some_and(|ch| ch.is_whitespace()) {
        position += 1;
    }
    if position == spaces_start || !starts_with_ignore_case(text, position, "file=") {
        return None;
    }
    position += "file=".len();
    if !matches!(text.get(position), Some('"' | '\'')) {
        return None;
    }
    let value_start = position + 1;
    // The value may not cross a line break or continue past a `..`.
    let mut value_limit = value_start;
    while value_limit < text.len()
        && text[value_limit] != '\n'
        && !(value_limit >= 2 && text[value_limit - 2] == '.' && text[value_limit - 1] == '.')
    {
        value_limit += 1;
    }
    // Greedy: the longest value followed by a quote, optional whitespace and `/>`.
    for value_end in (value_start + 1..=value_limit).rev() {
        if !matches!(text.get(value_end), Some('"' | '\'')) {
            continue;
        }
        let mut after = value_end + 1;
        while text.get(after).is_some_and(|ch| ch.is_whitespace()) {
            after += 1;
        }
        if text.get(after) == Some(&'/') && text.get(after + 1) == Some(&'>') {
            let value = text[value_start..value_end].iter().collect();
            return Some((value, after + 2));
        }
    }
    None
}

/// The lowercase key of the file `include` names, relative to the AddOns folder, from the
/// directory (`directory`, components relative to the AddOns folder) of the file that names it.
fn resolve_include(directory: &[String], include: &str) -> Option<String> {
    let include = include.replace('\\', "/");
    if include.starts_with('/') {
        return None;
    }
    let mut components = directory.to_vec();
    for part in include.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            other => components.push(other.to_string()),
        }
    }
    Some(lowercase_key(&components))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        fs::write(path, content).expect("write file");
    }

    fn md5_hex(bytes: &[u8]) -> String {
        hex(&Md5::digest(bytes))
    }

    #[test]
    fn md5_of_empty_input_is_the_known_value() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    /// A folder whose expected hash was computed with the reference script
    /// (`scripts/fingerprint.py` from the slice 21 research) on these exact files.
    fn reference_fixture(root: &Path) {
        write(
            root,
            "Sample/Sample.toc",
            "## Interface: 11507\n## Title: Sample\n# a comment\nCore.lua\nLibs\\embeds.xml # trailing\n  ui/Frames.XML  \n..\\Other\\escape.lua\nMissing.lua\n",
        );
        write(
            root,
            "Sample/Sample_Forever.toc",
            "## Interface: 16001\nCore.lua\nForever.lua\n",
        );
        write(root, "Sample/Sample-Mainline.toc", "Mainline.lua\n");
        write(root, "Sample/Sample-Unknown.toc", "Ignored.lua\n");
        write(root, "Sample/Core.lua", "print('core')\n");
        write(root, "Sample/Forever.lua", "print('forever')\n");
        write(root, "Sample/Mainline.lua", "print('mainline')\n");
        write(root, "Sample/Ignored.lua", "print('ignored')\n");
        write(root, "Sample/Unlisted.lua", "print('unlisted')\n");
        write(
            root,
            "Sample/bindings.xml",
            "<Bindings><Include file=\"NotFollowed.lua\"/></Bindings>\n",
        );
        write(root, "Sample/NotFollowed.lua", "print('not followed')\n");
        write(
            root,
            "Sample/Libs/embeds.xml",
            "<Ui>\n<!-- <Script file=\"Commented.lua\"/> -->\n<Script file=\"LibStub\\LibStub.lua\"/>\n<Include FILE='Lib\\lib.xml' />\n</Ui>\n",
        );
        write(root, "Sample/Libs/LibStub/LibStub.lua", "-- libstub\n");
        write(root, "Sample/Libs/Commented.lua", "-- commented\n");
        write(
            root,
            "Sample/Libs/Lib/lib.xml",
            "<Ui><Script file=\"a.lua\"/><Script file=\"b.lua\"/>\n<Script file=\"c.lua\"/></Ui>\n",
        );
        write(root, "Sample/Libs/Lib/a.lua", "-- a\n");
        write(root, "Sample/Libs/Lib/b.lua", "-- b\n");
        write(root, "Sample/Libs/Lib/c.lua", "-- c\n");
        write(
            root,
            "Sample/UI/frames.xml",
            "<Ui><Script file=\"Frames.lua\"/></Ui>\n",
        );
        write(root, "Sample/UI/Frames.lua", "-- frames\n");
        write(root, "Other/escape.lua", "-- outside\n");
    }

    #[test]
    fn fingerprint_matches_the_reference_script() {
        let dir = tempfile::tempdir().expect("tempdir");
        reference_fixture(dir.path());

        let fingerprint = folder_fingerprint(dir.path(), "Sample").expect("fingerprint");

        assert_eq!(fingerprint.hash, "cd732e6b7bbc9c31207b18f9f7834baf");
        assert_eq!(fingerprint.files, 13);
    }

    #[test]
    fn a_folder_without_toc_or_bindings_hashes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "Empty/readme.txt", "hello");

        let fingerprint = folder_fingerprint(dir.path(), "Empty").expect("fingerprint");

        assert_eq!(fingerprint.files, 0);
        assert_eq!(fingerprint.hash, md5_hex(b""));
    }

    #[test]
    fn toc_lines_keep_paths_and_drop_comments_and_climbing_paths() {
        assert_eq!(
            toc_includes("## Title: x\nA.lua\n  B\\C.XML # note\n..\\D.lua\nE.txt\n#F.lua\n"),
            vec!["A.lua".to_string(), "B\\C.XML".to_string()]
        );
    }

    #[test]
    fn xml_values_run_to_the_last_quote_on_the_line_like_the_reference() {
        assert_eq!(
            xml_includes(
                "<Script file=\"a.lua\"/><Script file=\"b.lua\"/>\n<Include file='c.xml'/>"
            ),
            vec![
                "a.lua\"/><Script file=\"b.lua".to_string(),
                "c.xml".to_string()
            ]
        );
    }
}
