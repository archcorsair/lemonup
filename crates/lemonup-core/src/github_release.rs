//! Which zip of a GitHub release to install for a game.
//!
//! Addon authors attach packaged zips to their GitHub releases. A packaged zip has the folder
//! names the game needs (BigWigs' `Core` becomes `BigWigs_Core`) and only the TOC files for its
//! game (OpenMap ships one zip with `OpenMap.toc` for WoW Forever and one with
//! `OpenMap_Mainline.toc` for Retail). The repo's source zipball has neither, so LemonUp
//! installs a release zip whenever one fits the game, in this order:
//!
//! 1. The release has `release.json` (the BigWigs packager format): the zip whose metadata lists
//!    the game's flavor, preferring a zip with its libraries (`nolib: false`).
//! 2. The zip whose name ends in the game's flavor (`-forever`, `-retail`, `-mainline`, ...).
//! 3. The only zip left once `-nolib` zips and zips whose name says another game are set aside.
//!
//! Otherwise, or with several zips and none of them for this game, the caller installs the
//! source zipball as before.
//!
//! WoW Forever is in the Mainline family: its client loads `_Forever.toc` or `_Camelot.toc`,
//! then `_Mainline.toc`, then the base `.toc`. So when a release has nothing for Forever,
//! LemonUp takes the Mainline zip, which is what the game would load from a source copy too.

use serde::Deserialize;

use crate::GameFlavor;

/// The file name of the BigWigs packager's release metadata.
pub const RELEASE_JSON_NAME: &str = "release.json";

/// A file attached to a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
}

/// `release.json` as the BigWigs packager writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseJson {
    #[serde(default)]
    pub releases: Vec<ReleaseJsonEntry>,
}

/// One packaged zip in `release.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseJsonEntry {
    pub filename: String,
    #[serde(default)]
    pub nolib: bool,
    #[serde(default)]
    pub metadata: Vec<ReleaseJsonMetadata>,
}

/// A flavor (and interface number) a packaged zip supports.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseJsonMetadata {
    pub flavor: String,
}

impl ReleaseJson {
    /// Parses `release.json`; `None` when it is not in the packager's format.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
    }
}

/// The `release.json` asset of a release, if it has one.
pub fn release_json_asset(assets: &[ReleaseAsset]) -> Option<&ReleaseAsset> {
    assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(RELEASE_JSON_NAME))
}

/// The release zip to install for `flavor`, or `None` to install the source zipball.
pub fn choose_release_asset<'a>(
    flavor: GameFlavor,
    assets: &'a [ReleaseAsset],
    release_json: Option<&ReleaseJson>,
) -> Option<&'a ReleaseAsset> {
    let zips = assets
        .iter()
        .filter(|asset| zip_stem(&asset.name).is_some())
        .collect::<Vec<_>>();
    if zips.is_empty() {
        return None;
    }

    if let Some(release_json) = release_json
        && let Some(choice) = choose_by_release_json(flavor, &zips, release_json)
    {
        return choice;
    }

    let packaged = zips
        .iter()
        .copied()
        .filter(|asset| !is_nolib(&asset.name))
        .collect::<Vec<_>>();

    for names in flavor_family(flavor).iter().map(|tier| tier.names) {
        let mut matching = packaged
            .iter()
            .copied()
            .filter(|asset| names.contains(&name_flavor_suffix(&asset.name).unwrap_or("")));
        if let (Some(only), None) = (matching.next(), matching.next()) {
            return Some(only);
        }
    }

    // Zips whose name says another game are set aside too, so the unsuffixed Mainline zip
    // next to `-classic` and `-cata` zips is the one left.
    let mut left = packaged
        .iter()
        .copied()
        .filter(|asset| match name_flavor_suffix(&asset.name) {
            Some(suffix) => family_names(flavor).any(|name| name == suffix),
            None => true,
        });
    match (left.next(), left.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Rule 1. `None` when `release.json` names none of the release's zips (so it says nothing
/// about them); `Some(None)` when it does but none is for this game.
fn choose_by_release_json<'a>(
    flavor: GameFlavor,
    zips: &[&'a ReleaseAsset],
    release_json: &ReleaseJson,
) -> Option<Option<&'a ReleaseAsset>> {
    let listed = release_json
        .releases
        .iter()
        .filter_map(|entry| {
            zips.iter()
                .copied()
                .find(|asset| asset.name == entry.filename)
                .map(|asset| (entry, asset))
        })
        .collect::<Vec<_>>();
    if listed.is_empty() {
        return None;
    }

    for tier in flavor_family(flavor) {
        let mut supported = listed
            .iter()
            .filter(|(entry, _)| {
                entry.metadata.iter().any(|metadata| {
                    tier.release_json
                        .iter()
                        .any(|name| metadata.flavor.eq_ignore_ascii_case(name))
                })
            })
            .collect::<Vec<_>>();
        supported.sort_by_key(|(entry, _)| entry.nolib);
        if let Some((_, asset)) = supported.first() {
            return Some(Some(asset));
        }
    }
    Some(None)
}

/// The flavor names one game accepts, best first.
struct FlavorTier {
    /// Flavor names in `release.json` metadata.
    release_json: &'static [&'static str],
    /// Lowercase asset name suffixes (`Foo-v1.0-retail.zip`).
    names: &'static [&'static str],
}

const MAINLINE: FlavorTier = FlavorTier {
    release_json: &["mainline"],
    names: &["mainline", "retail"],
};

fn flavor_family(flavor: GameFlavor) -> &'static [FlavorTier] {
    match flavor {
        GameFlavor::Retail => &[MAINLINE],
        GameFlavor::Forever => &[
            FlavorTier {
                release_json: &["forever", "camelot"],
                names: &["forever", "camelot"],
            },
            MAINLINE,
        ],
        GameFlavor::Classic => &[FlavorTier {
            release_json: &["classic", "vanilla"],
            names: &["classic", "vanilla", "era"],
        }],
        GameFlavor::Cata => &[FlavorTier {
            release_json: &["cata"],
            names: &["cata", "cataclysm"],
        }],
    }
}

fn family_names(flavor: GameFlavor) -> impl Iterator<Item = &'static str> {
    flavor_family(flavor)
        .iter()
        .flat_map(|tier| tier.names.iter().copied())
}

/// Every name suffix that says which game a zip is for.
const KNOWN_FLAVOR_SUFFIXES: &[&str] = &[
    "mainline",
    "retail",
    "forever",
    "camelot",
    "classic",
    "vanilla",
    "era",
    "tbc",
    "bcc",
    "wrath",
    "wotlk",
    "wotlkc",
    "cata",
    "cataclysm",
    "mists",
    "mop",
    "titan",
];

/// The asset name without `.zip`, for zip assets only.
fn zip_stem(name: &str) -> Option<&str> {
    let end = name.len().checked_sub(".zip".len())?;
    name.get(end..)
        .filter(|extension| extension.eq_ignore_ascii_case(".zip"))
        .and_then(|_| name.get(..end))
}

fn is_nolib(name: &str) -> bool {
    zip_stem(name).is_some_and(|stem| ends_with_token(stem, "nolib"))
}

/// The flavor a zip's name ends in (`OpenMap-v0.8.10-forever.zip` is `forever`), lowercase.
fn name_flavor_suffix(name: &str) -> Option<&'static str> {
    let stem = zip_stem(name)?;
    KNOWN_FLAVOR_SUFFIXES
        .iter()
        .copied()
        .find(|suffix| ends_with_token(stem, suffix))
}

/// Whether `stem` ends in `-token` or `_token`, ignoring case.
fn ends_with_token(stem: &str, token: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    lower
        .strip_suffix(token)
        .is_some_and(|rest| rest.ends_with('-') || rest.ends_with('_'))
}

#[cfg(test)]
mod tests {
    use super::{ReleaseAsset, ReleaseJson, choose_release_asset, release_json_asset};
    use crate::GameFlavor;

    fn assets(names: &[&str]) -> Vec<ReleaseAsset> {
        names
            .iter()
            .map(|name| ReleaseAsset {
                name: (*name).to_string(),
                download_url: format!("https://example.invalid/{name}"),
            })
            .collect()
    }

    fn chosen(
        flavor: GameFlavor,
        names: &[&str],
        release_json: Option<&ReleaseJson>,
    ) -> Option<String> {
        let assets = assets(names);
        choose_release_asset(flavor, &assets, release_json).map(|asset| asset.name.clone())
    }

    /// `release.json` of BigWigs v427.9, as published.
    const BIGWIGS_RELEASE_JSON: &str = r#"{"releases":[{"name":"BigWigs","version":"v427.9","filename":"BigWigs-v427.9.zip","nolib":false,"metadata":[{"flavor":"mists","interface":50503},{"flavor":"mists","interface":50504},{"flavor":"titan","interface":38002},{"flavor":"mainline","interface":120100},{"flavor":"mainline","interface":120105},{"flavor":"mainline","interface":120107},{"flavor":"wrath","interface":30405},{"flavor":"cata","interface":40402},{"flavor":"classic","interface":11508},{"flavor":"classic","interface":11509},{"flavor":"bcc","interface":20505},{"flavor":"bcc","interface":20506},{"flavor":"forever","interface":16001}]}]}"#;

    fn release_json(text: &str) -> ReleaseJson {
        ReleaseJson::parse(text).expect("release.json parses")
    }

    #[test]
    fn release_json_picks_the_zip_listing_the_game() {
        let json = release_json(BIGWIGS_RELEASE_JSON);
        let names = ["BigWigs-v427.9.zip", "release.json"];
        for flavor in [GameFlavor::Forever, GameFlavor::Retail] {
            assert_eq!(
                chosen(flavor, &names, Some(&json)).as_deref(),
                Some("BigWigs-v427.9.zip")
            );
        }
        assert_eq!(
            release_json_asset(&assets(&names)).map(|asset| asset.name.as_str()),
            Some("release.json")
        );
    }

    #[test]
    fn release_json_prefers_the_zip_with_its_libraries() {
        let json = release_json(
            r#"{"releases":[
                {"filename":"Foo-1.0-nolib.zip","nolib":true,"metadata":[{"flavor":"mainline"}]},
                {"filename":"Foo-1.0.zip","nolib":false,"metadata":[{"flavor":"mainline"}]}
            ]}"#,
        );
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo-1.0-nolib.zip", "Foo-1.0.zip", "release.json"],
                Some(&json)
            )
            .as_deref(),
            Some("Foo-1.0.zip")
        );
    }

    #[test]
    fn release_json_split_by_flavor_picks_the_game_s_zip() {
        let json = release_json(
            r#"{"releases":[
                {"filename":"Foo-1.0.zip","nolib":false,"metadata":[{"flavor":"mainline"}]},
                {"filename":"Foo-1.0-forever.zip","nolib":false,"metadata":[{"flavor":"forever"}]},
                {"filename":"Foo-1.0-classic.zip","nolib":false,"metadata":[{"flavor":"classic"}]}
            ]}"#,
        );
        let names = [
            "Foo-1.0.zip",
            "Foo-1.0-forever.zip",
            "Foo-1.0-classic.zip",
            "release.json",
        ];
        assert_eq!(
            chosen(GameFlavor::Forever, &names, Some(&json)).as_deref(),
            Some("Foo-1.0-forever.zip")
        );
        assert_eq!(
            chosen(GameFlavor::Retail, &names, Some(&json)).as_deref(),
            Some("Foo-1.0.zip")
        );
    }

    #[test]
    fn forever_takes_the_mainline_zip_when_release_json_has_no_forever_entry() {
        let json = release_json(
            r#"{"releases":[
                {"filename":"Foo-1.0.zip","nolib":false,"metadata":[{"flavor":"mainline"}]},
                {"filename":"Foo-1.0-classic.zip","nolib":false,"metadata":[{"flavor":"classic"}]}
            ]}"#,
        );
        assert_eq!(
            chosen(
                GameFlavor::Forever,
                &["Foo-1.0.zip", "Foo-1.0-classic.zip", "release.json"],
                Some(&json)
            )
            .as_deref(),
            Some("Foo-1.0.zip")
        );
    }

    #[test]
    fn release_json_without_the_game_means_the_source() {
        let json = release_json(
            r#"{"releases":[{"filename":"Foo-1.0.zip","nolib":false,"metadata":[{"flavor":"classic"}]}]}"#,
        );
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo-1.0.zip", "release.json"],
                Some(&json)
            ),
            None
        );
    }

    #[test]
    fn release_json_naming_no_asset_is_set_aside() {
        let json = release_json(
            r#"{"releases":[{"filename":"Other.zip","nolib":false,"metadata":[{"flavor":"classic"}]}]}"#,
        );
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo-1.0.zip", "release.json"],
                Some(&json)
            )
            .as_deref(),
            Some("Foo-1.0.zip")
        );
        assert!(ReleaseJson::parse("not json").is_none());
    }

    #[test]
    fn name_suffix_picks_the_game_s_zip() {
        // OpenMap v0.8.10.
        let names = ["OpenMap-v0.8.10-forever.zip", "OpenMap-v0.8.10-retail.zip"];
        assert_eq!(
            chosen(GameFlavor::Forever, &names, None).as_deref(),
            Some("OpenMap-v0.8.10-forever.zip")
        );
        assert_eq!(
            chosen(GameFlavor::Retail, &names, None).as_deref(),
            Some("OpenMap-v0.8.10-retail.zip")
        );
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo_Mainline.zip", "Foo_Classic.zip"],
                None
            )
            .as_deref(),
            Some("Foo_Mainline.zip")
        );
        assert_eq!(
            chosen(
                GameFlavor::Forever,
                &["Foo-camelot.zip", "Foo-cata.zip"],
                None
            )
            .as_deref(),
            Some("Foo-camelot.zip")
        );
    }

    #[test]
    fn forever_takes_the_retail_zip_when_no_name_says_forever() {
        assert_eq!(
            chosen(
                GameFlavor::Forever,
                &["Foo-1.0-retail.zip", "Foo-1.0-classic.zip"],
                None
            )
            .as_deref(),
            Some("Foo-1.0-retail.zip")
        );
    }

    #[test]
    fn the_only_zip_is_used_unless_it_names_another_game() {
        // ForeverAuras 1.4.3.
        assert_eq!(
            chosen(GameFlavor::Forever, &["ForeverAuras-1.4.3.zip"], None).as_deref(),
            Some("ForeverAuras-1.4.3.zip")
        );
        assert_eq!(
            chosen(GameFlavor::Retail, &["Foo-1.0-classic.zip"], None),
            None
        );
        assert_eq!(
            chosen(GameFlavor::Forever, &["Foo-1.0-retail.zip"], None).as_deref(),
            Some("Foo-1.0-retail.zip")
        );
    }

    #[test]
    fn nolib_zips_are_set_aside_before_counting() {
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo-1.0.zip", "Foo-1.0-nolib.zip"],
                None
            )
            .as_deref(),
            Some("Foo-1.0.zip")
        );
    }

    #[test]
    fn zips_named_for_other_games_are_set_aside_before_counting() {
        // AdvancedInterfaceOptions 2.1.14 without its release.json.
        let names = [
            "AdvancedInterfaceOptions-2.1.14-cata.zip",
            "AdvancedInterfaceOptions-2.1.14-classic.zip",
            "AdvancedInterfaceOptions-2.1.14.zip",
        ];
        assert_eq!(
            chosen(GameFlavor::Retail, &names, None).as_deref(),
            Some("AdvancedInterfaceOptions-2.1.14.zip")
        );
        assert_eq!(
            chosen(GameFlavor::Forever, &names, None).as_deref(),
            Some("AdvancedInterfaceOptions-2.1.14.zip")
        );
        assert_eq!(
            chosen(GameFlavor::Classic, &names, None).as_deref(),
            Some("AdvancedInterfaceOptions-2.1.14-classic.zip")
        );
    }

    #[test]
    fn several_zips_with_none_for_the_game_mean_the_source() {
        assert_eq!(
            chosen(GameFlavor::Retail, &["Foo-1.0.zip", "Bar-1.0.zip"], None),
            None
        );
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo-1.0-classic.zip", "Foo-1.0-forever.zip"],
                None
            ),
            None
        );
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["Foo-1.0-retail.zip", "Foo-1.0-Retail.zip"],
                None
            ),
            None
        );
    }

    #[test]
    fn no_zip_means_the_source() {
        assert_eq!(chosen(GameFlavor::Retail, &[], None), None);
        assert_eq!(
            chosen(GameFlavor::Retail, &["Foo-1.0.tar.gz", "notes.txt"], None),
            None
        );
    }

    #[test]
    fn a_name_that_merely_contains_a_flavor_is_not_a_suffix() {
        // `ClassicCastbars` names no game; `-classic` does, so that zip is set aside.
        assert_eq!(
            chosen(
                GameFlavor::Retail,
                &["ClassicCastbars-1.0.zip", "ClassicCastbars-1.0-classic.zip"],
                None
            )
            .as_deref(),
            Some("ClassicCastbars-1.0.zip")
        );
        assert_eq!(
            chosen(
                GameFlavor::Classic,
                &["ClassicCastbars-1.0.zip", "ClassicCastbars-1.0-classic.zip"],
                None
            )
            .as_deref(),
            Some("ClassicCastbars-1.0-classic.zip")
        );
    }
}
