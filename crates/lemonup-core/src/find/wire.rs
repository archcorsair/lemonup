//! What "find" sends to and reads from the sources, without the network: the Wago.io
//! `_match` request and answer and the WoWInterface file list. Both are read leniently: an
//! entry that doesn't fit is skipped, but an answer that isn't the expected shape at all is an
//! error, never "nothing found".

use std::collections::BTreeMap;

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use super::{WagoLookup, WagoMatch, WowiFile, WowiFolder};

#[derive(Serialize)]
struct MatchRequest<'a> {
    game_version: &'a str,
    addons: Vec<MatchAddon<'a>>,
}

#[derive(Serialize)]
struct MatchAddon<'a> {
    name: &'a str,
    hash: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    wago: Option<&'a str>,
}

/// The body of a Wago.io `_match` request: each folder's name, hash and TOC Wago ID.
pub fn wago_match_body(game_version: &str, lookups: &[WagoLookup]) -> String {
    let request = MatchRequest {
        game_version,
        addons: lookups
            .iter()
            .map(|lookup| MatchAddon {
                name: &lookup.folder,
                hash: &lookup.hash,
                wago: lookup.wago_id.as_deref(),
            })
            .collect(),
    };
    serde_json::to_string(&request).expect("a match request always serializes")
}

/// An entry that is skipped when it doesn't have the expected shape.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Item(T),
    Other(IgnoredAny),
}

impl<T> Lenient<T> {
    fn item(self) -> Option<T> {
        match self {
            Self::Item(item) => Some(item),
            Self::Other(_) => None,
        }
    }
}

/// An ID or version the sources send as a string or a number.
#[derive(Deserialize)]
#[serde(untagged)]
enum Text {
    Text(String),
    Number(serde_json::Number),
}

impl Text {
    fn into_string(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Number(number) => number.to_string(),
        }
    }
}

fn text(value: Option<Text>) -> Option<String> {
    value
        .map(Text::into_string)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Deserialize)]
struct MatchResponse {
    addons: Vec<Lenient<RawMatch>>,
}

#[derive(Deserialize)]
struct RawMatch {
    id: Text,
    name: String,
    website_url: Option<String>,
    authors: Option<Vec<String>>,
    matched_release: Option<RawRelease>,
    modules: Option<BTreeMap<String, RawModule>>,
    wowi: Option<Text>,
    recent_releases: Option<RawReleases>,
}

#[derive(Deserialize)]
struct RawRelease {
    label: Option<String>,
}

#[derive(Deserialize)]
struct RawReleases {
    stable: Option<RawRelease>,
    beta: Option<RawRelease>,
    alpha: Option<RawRelease>,
}

#[derive(Deserialize)]
struct RawModule {
    hash: String,
}

/// A Wago.io `_match` answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WagoAnswer {
    pub matches: Vec<WagoMatch>,
    /// Entries LemonUp couldn't read (an unexpected shape, or no folders). Which folders they
    /// were for is unknown, so any of them makes the answer incomplete.
    pub unreadable: usize,
}

/// Reads a Wago.io `_match` answer (`{"addons": [...]}`).
pub fn parse_wago_match(bytes: &[u8]) -> Result<WagoAnswer, String> {
    let response = serde_json::from_slice::<MatchResponse>(bytes)
        .map_err(|error| format!("Wago.io sent an answer LemonUp can't read: {error}"))?;
    let entries = response.addons.len();
    let matches = response
        .addons
        .into_iter()
        .filter_map(Lenient::item)
        .filter_map(|raw| {
            let modules = raw
                .modules?
                .into_iter()
                .map(|(folder, module)| (folder, module.hash))
                .collect::<Vec<_>>();
            if modules.is_empty() {
                return None;
            }
            let label = |release: Option<RawRelease>| {
                release
                    .and_then(|release| release.label)
                    .filter(|label| !label.trim().is_empty())
            };
            let latest = raw.recent_releases.and_then(|releases| {
                label(releases.stable)
                    .or_else(|| label(releases.beta))
                    .or_else(|| label(releases.alpha))
            });
            Some(WagoMatch {
                id: raw.id.into_string(),
                name: raw.name,
                website_url: raw.website_url,
                author: raw.authors.and_then(|authors| authors.into_iter().next()),
                modules,
                matched_version: label(raw.matched_release),
                latest_version: latest,
                wowinterface_id: text(raw.wowi),
            })
        })
        .collect::<Vec<_>>();
    Ok(WagoAnswer {
        unreadable: entries - matches.len(),
        matches,
    })
}

#[derive(Deserialize)]
struct RawWowiFile {
    id: Text,
    title: String,
    version: Option<Text>,
    author: Option<String>,
    addons: Option<Vec<Lenient<RawWowiAddon>>>,
}

#[derive(Deserialize)]
struct RawWowiAddon {
    path: String,
    #[serde(rename = "addOnVersion")]
    add_on_version: Option<Text>,
    #[serde(rename = "apiVersion")]
    api_version: Option<Text>,
}

/// Reads the WoWInterface v4 file list (a JSON array of files).
pub fn parse_wowi_filelist(bytes: &[u8]) -> Result<Vec<WowiFile>, String> {
    let files = serde_json::from_slice::<Vec<Lenient<RawWowiFile>>>(bytes)
        .map_err(|error| format!("WoWInterface sent a file list LemonUp can't read: {error}"))?;
    Ok(files
        .into_iter()
        .filter_map(Lenient::item)
        .filter_map(|raw| {
            Some(WowiFile {
                id: raw.id.into_string(),
                title: raw.title,
                version: text(raw.version)?,
                author: raw.author,
                folders: raw
                    .addons
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(Lenient::item)
                    .filter(|addon| !addon.path.trim().is_empty())
                    .map(|addon| WowiFolder {
                        folder: addon.path,
                        version: text(addon.add_on_version),
                        api_version: text(addon.api_version),
                    })
                    .collect(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real Forever answer (download links removed).
    const WAGO_ANSWER: &str = r#"{"addons":[
      {"id":"WL6JqQNv","name":"Narcissus","authors":["Peterodox","Raenore"],
       "website_url":"https://addons.wago.io/addons/narcissus",
       "matched_release":{"id":"ldbM5RYQ","label":"v1.8.7-c","patch":"1.60.1"},
       "modules":{"Narcissus_Achievements":{"hash":"a7152381bf9afb0a86182e15c79718a1"},
                  "Narcissus":{"hash":"e9e5ab849b3985d9166d41d4ee48d74d"}},
       "cf":null,"wowi":null,"wago":"WL6JqQNv",
       "recent_releases":{"stable":{"label":"v1.8.7-c"}}},
      {"id":"XKq9xXKy","name":"Azeroth Pilot Reloaded","authors":["Neogeekmo"],
       "website_url":"https://addons.wago.io/addons/azeroth-pilot-reloaded",
       "matched_release":{"label":"v7.7.1"},
       "modules":{"APR":{"hash":"0123456789abcdef0123456789abcdef"}},
       "cf":618667,"wowi":26457,
       "recent_releases":{"stable":{"label":"v7.8.0"},"alpha":{"label":"v7.5.2-3-g88b6dcd"}}},
      {"id":"Only","name":"Beta only","modules":{"Beta":{"hash":"00"}},
       "recent_releases":{"beta":{"label":"b2"},"alpha":{"label":"a3"}}},
      {"unexpected":"entry"},
      {"id":"Null","name":"No folders","modules":null},
      {"id":"Empty","name":"Empty folders","modules":{}}
    ]}"#;

    #[test]
    fn a_wago_answer_becomes_matches_and_counts_what_it_cannot_read() {
        let answer = parse_wago_match(WAGO_ANSWER.as_bytes()).expect("parse");

        assert_eq!(answer.unreadable, 3);
        let matches = answer.matches;
        assert_eq!(matches.len(), 3);
        let narcissus = &matches[0];
        assert_eq!(narcissus.id, "WL6JqQNv");
        assert_eq!(narcissus.author.as_deref(), Some("Peterodox"));
        assert_eq!(
            narcissus
                .modules
                .iter()
                .map(|(folder, _)| folder.as_str())
                .collect::<Vec<_>>(),
            ["Narcissus", "Narcissus_Achievements"]
        );
        assert_eq!(narcissus.matched_version.as_deref(), Some("v1.8.7-c"));
        assert_eq!(narcissus.latest_version.as_deref(), Some("v1.8.7-c"));
        assert_eq!(narcissus.wowinterface_id, None);
        let apr = &matches[1];
        assert_eq!(apr.latest_version.as_deref(), Some("v7.8.0"));
        assert_eq!(apr.wowinterface_id.as_deref(), Some("26457"));
        assert_eq!(matches[2].latest_version.as_deref(), Some("b2"));
    }

    #[test]
    fn an_answer_of_the_wrong_shape_is_an_error() {
        assert!(parse_wago_match(b"[]").is_err());
        assert!(parse_wago_match(b"<html>").is_err());
        assert!(parse_wowi_filelist(b"{\"files\":[]}").is_err());
    }

    #[test]
    fn the_match_body_names_each_folder_hash_and_wago_id() {
        let body = wago_match_body(
            "forever",
            &[
                WagoLookup {
                    folder: "APR".to_string(),
                    hash: "abc".to_string(),
                    wago_id: Some("XKq9xXKy".to_string()),
                },
                WagoLookup {
                    folder: "Pawn".to_string(),
                    hash: "def".to_string(),
                    wago_id: None,
                },
            ],
        );

        assert_eq!(
            body,
            r#"{"game_version":"forever","addons":[{"name":"APR","hash":"abc","wago":"XKq9xXKy"},{"name":"Pawn","hash":"def"}]}"#
        );
    }

    /// Trimmed from the real v4 file list.
    const WOWI_LIST: &str = r#"[
      {"id":7032,"title":"TomTom","version":"v4.4.1-release","author":"Cladhaire",
       "gameVersions":["12.0.5"],"checksum":"x",
       "addons":[{"path":"TomTom","addOnVersion":"v4.4.1-release","apiVersion":"120005 16001 11508"}]},
      {"id":3826,"title":"CTMod","version":"11.0.2.3","author":"DahkCeles",
       "addons":[{"path":"CT_BarMod","addOnVersion":"11.0.2.3","apiVersion":"110002 40400 11503"},
                 {"path":"CT_BottomBar","apiVersion":"n/a 40400 11503"}]},
      {"id":1,"title":"No version"},
      "junk"
    ]"#;

    #[test]
    fn the_wowinterface_list_becomes_files() {
        let files = parse_wowi_filelist(WOWI_LIST.as_bytes()).expect("parse");

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].id, "7032");
        assert_eq!(files[0].folders[0].folder, "TomTom");
        assert_eq!(
            files[0].folders[0].version.as_deref(),
            Some("v4.4.1-release")
        );
        assert_eq!(files[1].folders[1].version, None);
        assert_eq!(
            files[1].folders[1].api_version.as_deref(),
            Some("n/a 40400 11503")
        );
    }
}
