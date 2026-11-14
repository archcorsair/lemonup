use std::path::PathBuf;

/// Converts a raw zip entry name into a safe relative path.
///
/// Some archives are produced on Windows and store `\` as the path separator.
/// The `zip` crate's `enclosed_name` keeps those backslashes, which on Unix
/// yields a single file with backslashes in its name. This normalizes both
/// separators, drops empty and `.` components, and rejects anything that could
/// escape the extraction root (`..`, absolute paths, drive or UNC prefixes,
/// NUL bytes, or `:` in any component). Returns `None` for unsafe or empty
/// names.
pub(crate) fn safe_entry_relative_path(raw_name: &str) -> Option<PathBuf> {
    if raw_name.contains('\0') {
        return None;
    }

    let normalized = raw_name.replace('\\', "/");
    // Leading `/` covers absolute paths and UNC (`\\server\share`) prefixes.
    if normalized.starts_with('/') {
        return None;
    }

    let mut path = PathBuf::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => continue,
            ".." => return None,
            other if other.contains(':') => return None,
            other => path.push(other),
        }
    }

    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::safe_entry_relative_path;

    fn p(parts: &[&str]) -> Option<PathBuf> {
        Some(parts.iter().collect())
    }

    #[test]
    fn forward_slashes_are_unchanged() {
        assert_eq!(
            safe_entry_relative_path("Foo/Locales/Locales.xml"),
            p(&["Foo", "Locales", "Locales.xml"])
        );
        assert_eq!(safe_entry_relative_path("Foo.toc"), p(&["Foo.toc"]));
    }

    #[test]
    fn backslashes_become_nested_paths() {
        assert_eq!(
            safe_entry_relative_path("AtlasLootClassic\\Locales\\Locales.xml"),
            p(&["AtlasLootClassic", "Locales", "Locales.xml"])
        );
    }

    #[test]
    fn mixed_separators_are_normalized() {
        assert_eq!(
            safe_entry_relative_path("Foo\\Sub/a.lua"),
            p(&["Foo", "Sub", "a.lua"])
        );
    }

    #[test]
    fn trailing_separators_and_current_dir_components_are_dropped() {
        assert_eq!(safe_entry_relative_path("Foo\\Sub\\"), p(&["Foo", "Sub"]));
        assert_eq!(safe_entry_relative_path("Foo/Sub/"), p(&["Foo", "Sub"]));
        assert_eq!(safe_entry_relative_path("./a"), p(&["a"]));
        assert_eq!(safe_entry_relative_path(".\\a\\.\\b"), p(&["a", "b"]));
        assert_eq!(safe_entry_relative_path("a//b"), p(&["a", "b"]));
    }

    #[test]
    fn parent_traversal_is_rejected() {
        assert_eq!(safe_entry_relative_path("..\\..\\evil"), None);
        assert_eq!(safe_entry_relative_path("../evil"), None);
        assert_eq!(safe_entry_relative_path("a\\..\\b"), None);
        assert_eq!(safe_entry_relative_path("a/../b"), None);
    }

    #[test]
    fn absolute_drive_and_unc_paths_are_rejected() {
        assert_eq!(safe_entry_relative_path("/abs"), None);
        assert_eq!(safe_entry_relative_path("\\abs"), None);
        assert_eq!(safe_entry_relative_path("C:\\x"), None);
        assert_eq!(safe_entry_relative_path("C:/x"), None);
        assert_eq!(safe_entry_relative_path("\\\\server\\share\\x"), None);
        assert_eq!(safe_entry_relative_path("//server/share/x"), None);
        assert_eq!(safe_entry_relative_path("Foo\\bar:stream"), None);
    }

    #[test]
    fn empty_and_nul_names_are_rejected() {
        assert_eq!(safe_entry_relative_path(""), None);
        assert_eq!(safe_entry_relative_path("."), None);
        assert_eq!(safe_entry_relative_path("./"), None);
        assert_eq!(safe_entry_relative_path("a\0b"), None);
    }
}
