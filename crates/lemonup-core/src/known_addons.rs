//! Addons LemonUp knows by name, from the hand-vetted `known_addons.toml`.
//!
//! A GitHub entry names the repository LemonUp installs it from and GitHub's
//! numeric id for it, so a repository that changed hands can be told apart
//! from the one that was vetted. A CurseForge-only entry is listed so Install
//! search can show it, but it has nothing to install. It needs no network.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::domain::GameFlavor;
use crate::search_rank::{GAME_WORDS, any_name_has_every_word, parse_query};

const KNOWN_ADDONS_TOML: &str = include_str!("../known_addons.toml");

/// An addon on the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownAddon {
    pub name: String,
    pub source: KnownSource,
    /// The games the addon is listed for.
    pub games: Vec<GameFlavor>,
}

/// Where a known addon comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnownSource {
    /// A GitHub repository, with GitHub's numeric id for it.
    GitHub {
        owner: String,
        repo: String,
        id: u64,
    },
    /// Only on CurseForge: shown in Install search, not installable.
    CurseForgeOnly,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownAddonsFile {
    #[serde(default)]
    addon: Vec<RawAddon>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAddon {
    name: String,
    github: Option<String>,
    github_id: Option<u64>,
    #[serde(default)]
    curseforge_only: bool,
    #[serde(default)]
    games: Vec<GameFlavor>,
}

/// Parses and checks a list. Any entry that is not valid is an error that
/// names it.
pub fn parse_known_addons(text: &str) -> Result<Vec<KnownAddon>, String> {
    let file: KnownAddonsFile =
        toml::from_str(text).map_err(|error| format!("known addons list: {error}"))?;
    let mut list: Vec<KnownAddon> = Vec::with_capacity(file.addon.len());
    for raw in file.addon {
        let entry = check_entry(raw)?;
        if let Some(other) = list
            .iter()
            .find(|other| other.name.eq_ignore_ascii_case(&entry.name))
        {
            return Err(format!(
                "known addons list: '{}' and '{}' have the same name",
                other.name, entry.name
            ));
        }
        if let KnownSource::GitHub { owner, repo, .. } = &entry.source
            && let Some(other) = list.iter().find(|other| {
                matches!(
                    &other.source,
                    KnownSource::GitHub { owner: other_owner, repo: other_repo, .. }
                        if other_owner.eq_ignore_ascii_case(owner)
                            && other_repo.eq_ignore_ascii_case(repo)
                )
            })
        {
            return Err(format!(
                "known addons list: '{}' and '{}' have the same repository {owner}/{repo}",
                other.name, entry.name
            ));
        }
        list.push(entry);
    }
    Ok(list)
}

fn check_entry(raw: RawAddon) -> Result<KnownAddon, String> {
    let name = raw.name.trim().to_string();
    if name.is_empty() {
        return Err("known addons list: an entry has no name".to_string());
    }
    if raw.games.is_empty() {
        return Err(format!("known addons list: '{name}' has no games"));
    }
    let source = match (raw.github, raw.curseforge_only) {
        (Some(_), true) => {
            return Err(format!(
                "known addons list: '{name}' is both a GitHub and a CurseForge-only entry"
            ));
        }
        (None, false) => {
            return Err(format!(
                "known addons list: '{name}' has no github and is not curseforge_only"
            ));
        }
        (None, true) => {
            if raw.github_id.is_some() {
                return Err(format!(
                    "known addons list: '{name}' is CurseForge-only and has a github_id"
                ));
            }
            KnownSource::CurseForgeOnly
        }
        (Some(github), false) => {
            let (owner, repo) = split_repository(&github).ok_or_else(|| {
                format!("known addons list: '{name}' github '{github}' is not owner/repo")
            })?;
            let id = match raw.github_id {
                Some(id) if id > 0 => id,
                _ => {
                    return Err(format!(
                        "known addons list: '{name}' needs a github_id above 0"
                    ));
                }
            };
            KnownSource::GitHub { owner, repo, id }
        }
    };
    Ok(KnownAddon {
        name,
        source,
        games: raw.games,
    })
}

/// `owner/repo` with two parts of ASCII letters, digits, `-`, `_` and `.`,
/// neither only dots, and a repo that does not end in `.git`: the names a
/// GitHub link can carry as they are.
fn split_repository(text: &str) -> Option<(String, String)> {
    let (owner, repo) = text.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
            && !part.chars().all(|character| character == '.')
    };
    let git_suffix = repo.to_ascii_lowercase().ends_with(".git");
    (valid(owner) && valid(repo) && !git_suffix).then(|| (owner.to_string(), repo.to_string()))
}

/// The list LemonUp ships with, parsed once. Empty if the embedded file does
/// not parse; a test keeps that from happening in a release.
pub fn known_addons() -> &'static [KnownAddon] {
    static LIST: OnceLock<Vec<KnownAddon>> = OnceLock::new();
    LIST.get_or_init(|| parse_known_addons(KNOWN_ADDONS_TOML).unwrap_or_default())
}

/// The entries listed for `flavor` whose name has every word of `query`,
/// matched the way Install search matches names. An empty query matches
/// nothing. Keeps the list's order.
pub fn matching_known_addons<'a>(
    list: &'a [KnownAddon],
    query: &str,
    flavor: GameFlavor,
) -> Vec<&'a KnownAddon> {
    if parse_query(query, &GAME_WORDS).words.is_empty() {
        return Vec::new();
    }
    list.iter()
        .filter(|addon| addon.games.contains(&flavor))
        .filter(|addon| any_name_has_every_word(query, &GAME_WORDS, &[addon.name.as_str()]))
        .collect()
}

/// The GitHub entry for `owner/repo`, ignoring case.
pub fn find_known_github_repo<'a>(
    list: &'a [KnownAddon],
    owner: &str,
    repo: &str,
) -> Option<&'a KnownAddon> {
    list.iter().find(|addon| match &addon.source {
        KnownSource::GitHub {
            owner: known_owner,
            repo: known_repo,
            ..
        } => known_owner.eq_ignore_ascii_case(owner) && known_repo.eq_ignore_ascii_case(repo),
        KnownSource::CurseForgeOnly => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(text: &str) -> Vec<KnownAddon> {
        parse_known_addons(text).expect("test list parses")
    }

    fn sample() -> Vec<KnownAddon> {
        list(
            r#"
[[addon]]
name = "OpenMap"
github = "openmap-world/openmap"
github_id = 10
games = ["forever", "retail"]

[[addon]]
name = "Forever Loot Sparkles"
github = "svengabr/ForeverLootSparkles"
github_id = 20
games = ["forever"]

[[addon]]
name = "Leatrix Plus"
curseforge_only = true
games = ["forever", "retail"]
"#,
        )
    }

    #[test]
    fn the_embedded_file_parses_and_is_not_empty() {
        let parsed = parse_known_addons(include_str!("../known_addons.toml"))
            .expect("known_addons.toml is valid");
        assert!(!parsed.is_empty());
        assert_eq!(known_addons().len(), parsed.len());
    }

    #[test]
    fn a_valid_entry_is_read() {
        let parsed = sample();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name, "OpenMap");
        assert_eq!(
            parsed[0].source,
            KnownSource::GitHub {
                owner: "openmap-world".to_string(),
                repo: "openmap".to_string(),
                id: 10,
            }
        );
        assert_eq!(
            parsed[0].games,
            vec![GameFlavor::Forever, GameFlavor::Retail]
        );
        assert_eq!(parsed[2].source, KnownSource::CurseForgeOnly);
    }

    fn error_of(text: &str) -> String {
        parse_known_addons(text).expect_err("the list is not valid")
    }

    #[test]
    fn an_entry_with_github_and_curseforge_only_is_refused() {
        let error = error_of(
            "[[addon]]\nname = \"A\"\ngithub = \"o/r\"\ngithub_id = 1\ncurseforge_only = true\n\
             games = [\"forever\"]\n",
        );
        assert!(error.contains("'A'"), "{error}");
    }

    #[test]
    fn an_entry_with_neither_source_is_refused() {
        let error = error_of("[[addon]]\nname = \"A\"\ngames = [\"forever\"]\n");
        assert!(error.contains("'A'"), "{error}");
    }

    #[test]
    fn a_github_value_must_be_owner_slash_repo() {
        for bad in ["owner", "owner/", "/repo", "a/b/c", "", "/", " /repo"] {
            let text = format!(
                "[[addon]]\nname = \"A\"\ngithub = \"{bad}\"\ngithub_id = 1\ngames = [\"forever\"]\n"
            );
            assert!(parse_known_addons(&text).is_err(), "'{bad}' was accepted");
        }
    }

    #[test]
    fn a_github_entry_needs_an_id_above_zero() {
        let missing = "[[addon]]\nname = \"A\"\ngithub = \"o/r\"\ngames = [\"forever\"]\n";
        let zero =
            "[[addon]]\nname = \"A\"\ngithub = \"o/r\"\ngithub_id = 0\ngames = [\"forever\"]\n";
        assert!(parse_known_addons(missing).is_err());
        assert!(parse_known_addons(zero).is_err());
    }

    #[test]
    fn an_entry_needs_a_game() {
        let none = "[[addon]]\nname = \"A\"\ncurseforge_only = true\n";
        let empty = "[[addon]]\nname = \"A\"\ncurseforge_only = true\ngames = []\n";
        assert!(parse_known_addons(none).is_err());
        assert!(parse_known_addons(empty).is_err());
    }

    #[test]
    fn a_game_must_be_a_known_flavor() {
        let text = "[[addon]]\nname = \"A\"\ncurseforge_only = true\ngames = [\"tbc\"]\n";
        assert!(parse_known_addons(text).is_err());
    }

    fn github_entry(github: &str) -> String {
        format!(
            "[[addon]]\nname = \"A\"\ngithub = \"{github}\"\ngithub_id = 1\ngames = [\"forever\"]\n"
        )
    }

    #[test]
    fn a_repository_may_have_letters_digits_dashes_underscores_and_dots() {
        let list = list(&github_entry("Some-Owner_2/repo.name-3_x"));
        assert!(matches!(
            &list[0].source,
            KnownSource::GitHub { owner, repo, .. }
                if owner == "Some-Owner_2" && repo == "repo.name-3_x"
        ));
    }

    #[test]
    fn a_repository_with_another_character_is_refused() {
        for github in [
            "own er/repo",
            "owner/re po",
            "owner/repo!",
            "ow+ner/repo",
            "owner/rep\u{e9}",
        ] {
            assert!(
                parse_known_addons(&github_entry(github)).is_err(),
                "'{github}' was accepted"
            );
        }
    }

    #[test]
    fn a_repository_ending_in_git_is_refused() {
        for github in ["owner/repo.git", "owner/repo.GIT", "owner/.git"] {
            assert!(
                parse_known_addons(&github_entry(github)).is_err(),
                "'{github}' was accepted"
            );
        }
    }

    #[test]
    fn a_repository_part_of_only_dots_is_refused() {
        for github in ["./repo", "../repo", "owner/.", "owner/.."] {
            assert!(
                parse_known_addons(&github_entry(github)).is_err(),
                "'{github}' was accepted"
            );
        }
    }

    #[test]
    fn an_entry_needs_a_name() {
        for name in ["", "   "] {
            let text = format!(
                "[[addon]]\nname = \"{name}\"\ncurseforge_only = true\ngames = [\"forever\"]\n"
            );
            assert!(parse_known_addons(&text).is_err(), "'{name}' was accepted");
        }
    }

    #[test]
    fn two_entries_cannot_share_a_name_ignoring_case() {
        let error = error_of(
            "[[addon]]\nname = \"Leatrix Plus\"\ncurseforge_only = true\ngames = [\"forever\"]\n\n\
             [[addon]]\nname = \"LEATRIX plus\"\ncurseforge_only = true\ngames = [\"retail\"]\n",
        );
        assert!(error.contains("same name"), "{error}");
    }

    #[test]
    fn two_entries_cannot_share_a_repository_ignoring_case() {
        let error = error_of(
            "[[addon]]\nname = \"A\"\ngithub = \"Owner/Repo\"\ngithub_id = 1\ngames = [\"forever\"]\n\n\
             [[addon]]\nname = \"B\"\ngithub = \"owner/repo\"\ngithub_id = 1\ngames = [\"forever\"]\n",
        );
        assert!(error.contains("same repository"), "{error}");
    }

    #[test]
    fn unknown_keys_are_refused() {
        let in_entry =
            "[[addon]]\nname = \"A\"\ncurseforge_only = true\ngames = [\"forever\"]\nurl = \"x\"\n";
        let top_level = "extra = 1\n";
        assert!(parse_known_addons(in_entry).is_err());
        assert!(parse_known_addons(top_level).is_err());
    }

    #[test]
    fn matching_filters_by_game() {
        let list = sample();
        let names = |flavor| -> Vec<&str> {
            matching_known_addons(&list, "loot", flavor)
                .into_iter()
                .map(|addon| addon.name.as_str())
                .collect()
        };
        assert_eq!(names(GameFlavor::Forever), vec!["Forever Loot Sparkles"]);
        assert!(names(GameFlavor::Retail).is_empty());
        assert_eq!(
            matching_known_addons(&list, "leatrix", GameFlavor::Retail).len(),
            1
        );
    }

    #[test]
    fn matching_needs_every_typed_word() {
        let list = sample();
        let found = matching_known_addons(&list, "loot sparkles", GameFlavor::Forever);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Forever Loot Sparkles");
        assert!(matching_known_addons(&list, "loot bags", GameFlavor::Forever).is_empty());
    }

    #[test]
    fn matching_follows_install_search_for_prefixes_and_case() {
        let list = sample();
        let found = matching_known_addons(&list, "openm", GameFlavor::Forever);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "OpenMap");
        assert_eq!(
            matching_known_addons(&list, "OPENMAP", GameFlavor::Retail).len(),
            1
        );
        assert!(matching_known_addons(&list, "map open", GameFlavor::Retail).len() == 1);
        assert!(matching_known_addons(&list, "penmap", GameFlavor::Retail).is_empty());
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        let list = sample();
        for query in ["", "   ", "\t"] {
            assert!(matching_known_addons(&list, query, GameFlavor::Forever).is_empty());
        }
    }

    #[test]
    fn a_game_word_matches_as_install_search_says() {
        let list = sample();
        for query in ["forever", "forever openmap", "retail loot"] {
            for flavor in [GameFlavor::Forever, GameFlavor::Retail] {
                let expected: Vec<&str> = list
                    .iter()
                    .filter(|addon| addon.games.contains(&flavor))
                    .filter(|addon| {
                        any_name_has_every_word(query, &GAME_WORDS, &[addon.name.as_str()])
                    })
                    .map(|addon| addon.name.as_str())
                    .collect();
                let found: Vec<&str> = matching_known_addons(&list, query, flavor)
                    .into_iter()
                    .map(|addon| addon.name.as_str())
                    .collect();
                assert_eq!(found, expected, "query '{query}'");
            }
        }
        // Only "Forever Loot Sparkles" has the word "forever" in its name.
        let found = matching_known_addons(&list, "forever", GameFlavor::Forever);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Forever Loot Sparkles");
        // "retail" is in no name, so nothing has every word.
        assert!(matching_known_addons(&list, "retail", GameFlavor::Retail).is_empty());
    }

    #[test]
    fn matching_keeps_list_order() {
        let list = list(
            "[[addon]]\nname = \"Zed Map\"\ncurseforge_only = true\ngames = [\"forever\"]\n\n\
             [[addon]]\nname = \"Alpha Map\"\ncurseforge_only = true\ngames = [\"forever\"]\n",
        );
        let names: Vec<&str> = matching_known_addons(&list, "map", GameFlavor::Forever)
            .into_iter()
            .map(|addon| addon.name.as_str())
            .collect();
        assert_eq!(names, vec!["Zed Map", "Alpha Map"]);
    }

    #[test]
    fn a_repository_is_found_ignoring_case() {
        let list = sample();
        let found = find_known_github_repo(&list, "SvenGabr", "foreverlootsparkles")
            .expect("the repository is listed");
        assert_eq!(found.name, "Forever Loot Sparkles");
        assert!(find_known_github_repo(&list, "svengabr", "other").is_none());
    }

    #[test]
    fn a_curseforge_only_entry_is_never_a_repository() {
        let list = sample();
        assert!(find_known_github_repo(&list, "Leatrix", "Leatrix Plus").is_none());
        assert!(find_known_github_repo(&list, "leatrix", "leatrix-plus").is_none());
    }
}
