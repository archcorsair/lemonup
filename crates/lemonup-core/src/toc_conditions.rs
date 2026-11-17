//! Inline conditions on TOC directive lines, such as
//! `## Title: ElvUI Forever [AllowLoadGameType camelot][ExcludeLoadGameType standard, classic]`.
//!
//! The client skips a directive line whose conditions do not match it, and of the lines that
//! are left the last one wins (warcraft.wiki.gg, "TOC format": Inline directives, Addon
//! directives). Every condition on a line must match. Condition names and values are matched
//! without case.
//!
//! - `AllowLoadGameType` and `ExcludeLoadGameType` take game types (`standard`, `camelot` or
//!   `forever`, `vanilla`, ...) or families (`mainline`, `classic`). An `AllowLoadGameType` that
//!   names no game type the client knows still matches, as the wiki warns.
//! - `AllowLoadTextLocale` depends on the client's language, which LemonUp does not know: such a
//!   line never counts, the same as a localized `## Title-deDE:` line.
//! - `AllowLoad` (`Game`, `Glue`, `Both`), `AllowLoadEnvironment` (`Global`, `Secure`) and
//!   `Bootstrap` say where a line applies; only the in-game, global, non-bootstrap case counts.
//! - `LoadIntoEnvironment` is not a condition and is only taken out of the value.
//!
//! Bracketed text that is not one of these directives is part of the value.

use crate::domain::GameFlavor;

/// Game types and families the clients know (warcraft.wiki.gg, Template:GameTypes).
const KNOWN_GAME_TYPES: &[&str] = &[
    "standard",
    "forever",
    "camelot",
    "mists",
    "cata",
    "wrath",
    "tbc",
    "vanilla",
    "plunderstorm",
    "wowlabs",
    "wowhack",
    "mainline",
    "classic",
];

/// The game type names that match `flavor`'s client: its game type, aliases and family.
fn client_game_types(flavor: GameFlavor) -> &'static [&'static str] {
    match flavor {
        GameFlavor::Retail => &["standard", "mainline"],
        GameFlavor::Forever => &["camelot", "forever", "mainline"],
        GameFlavor::Classic => &["vanilla", "classic"],
        GameFlavor::Cata => &["cata", "classic"],
    }
}

/// A directive line's value without its inline conditions, and whether the line applies to the
/// client. With `flavor` `None` (game version not known), a line with a game type condition
/// never applies, so only lines that apply to every game version count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TocLineValue {
    pub value: String,
    pub applies: bool,
}

/// Splits a directive's raw value (the text after `## Key:`) into its value and whether the line
/// applies to `flavor`'s client. The value is trimmed.
pub(crate) fn evaluate(raw_value: &str, flavor: Option<GameFlavor>) -> TocLineValue {
    let mut value = String::with_capacity(raw_value.len());
    let mut applies = true;
    let mut rest = raw_value;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|close| open + close) else {
            break;
        };
        match condition_applies(&rest[open + 1..close], flavor) {
            Some(matches) => {
                value.push_str(&rest[..open]);
                applies &= matches;
            }
            None => value.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    value.push_str(rest);
    TocLineValue {
        value: value.trim().to_string(),
        applies,
    }
}

/// Whether one inline directive (the text inside its brackets) lets the line apply, or `None`
/// when the text is not an inline directive.
fn condition_applies(inner: &str, flavor: Option<GameFlavor>) -> Option<bool> {
    let inner = inner.trim();
    let (name, args) = match inner.find(char::is_whitespace) {
        Some(space) => (&inner[..space], &inner[space..]),
        None => (inner, ""),
    };
    let values = || {
        args.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
    };
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        "allowloadgametype" => {
            let listed = values();
            let Some(flavor) = flavor else {
                return Some(false);
            };
            let client = client_game_types(flavor);
            !listed
                .iter()
                .any(|value| KNOWN_GAME_TYPES.contains(&value.as_str()))
                || listed.iter().any(|value| client.contains(&value.as_str()))
        }
        "excludeloadgametype" => {
            let listed = values();
            let Some(flavor) = flavor else {
                return Some(false);
            };
            let client = client_game_types(flavor);
            !listed.iter().any(|value| client.contains(&value.as_str()))
        }
        "allowloadtextlocale" => false,
        "allowload" => values()
            .iter()
            .any(|value| value == "game" || value == "both"),
        "allowloadenvironment" => values().iter().any(|value| value == "global"),
        "bootstrap" => false,
        "loadintoenvironment" => true,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{TocLineValue, evaluate};
    use crate::domain::GameFlavor;

    fn applies(raw: &str, flavor: Option<GameFlavor>) -> bool {
        evaluate(raw, flavor).applies
    }

    #[test]
    fn conditions_are_taken_out_of_the_value() {
        assert_eq!(
            evaluate(
                " |cff1784d1ElvUI|r Forever [AllowLoadGameType camelot][ExcludeLoadGameType standard, classic]",
                Some(GameFlavor::Forever)
            ),
            TocLineValue {
                value: "|cff1784d1ElvUI|r Forever".to_string(),
                applies: true
            }
        );
        // Conditions can appear anywhere in the line; other brackets are part of the value.
        assert_eq!(
            evaluate(
                "Foo [AllowLoadGameType standard] [Beta] Bar",
                Some(GameFlavor::Retail)
            )
            .value,
            "Foo  [Beta] Bar"
        );
        assert_eq!(evaluate("Foo [unclosed", None).value, "Foo [unclosed");
        assert_eq!(evaluate("Foo", None), {
            TocLineValue {
                value: "Foo".to_string(),
                applies: true,
            }
        });
    }

    #[test]
    fn game_types_match_the_client_and_its_family() {
        let forever = Some(GameFlavor::Forever);
        let retail = Some(GameFlavor::Retail);
        assert!(applies("x [AllowLoadGameType camelot]", forever));
        assert!(applies("x [ALLOWLOADGAMETYPE Forever]", forever));
        assert!(applies("x [AllowLoadGameType mainline]", forever));
        assert!(!applies("x [AllowLoadGameType camelot]", retail));
        assert!(applies("x [AllowLoadGameType standard, vanilla]", retail));
        assert!(!applies("x [AllowLoadGameType classic]", retail));
        assert!(applies(
            "x [AllowLoadGameType vanilla]",
            Some(GameFlavor::Classic)
        ));
        assert!(applies(
            "x [AllowLoadGameType cata]",
            Some(GameFlavor::Cata)
        ));
        // A list with no game type the client knows still matches, as in the client.
        assert!(applies("x [AllowLoadGameType someday]", retail));
        assert!(!applies("x [AllowLoadGameType someday, mists]", retail));

        assert!(!applies("x [ExcludeLoadGameType standard]", retail));
        assert!(!applies("x [ExcludeLoadGameType mainline]", forever));
        assert!(applies(
            "x [ExcludeLoadGameType standard, classic]",
            forever
        ));
        // Every condition on the line must match.
        assert!(!applies(
            "x [AllowLoadGameType camelot][ExcludeLoadGameType camelot]",
            forever
        ));
    }

    #[test]
    fn without_a_known_game_version_game_type_conditions_never_apply() {
        assert!(!applies("x [AllowLoadGameType standard]", None));
        assert!(!applies("x [ExcludeLoadGameType vanilla]", None));
        assert!(applies("x", None));
        assert!(applies("x [LoadIntoEnvironment Global]", None));
    }

    #[test]
    fn conditions_the_game_version_does_not_decide() {
        let retail = Some(GameFlavor::Retail);
        assert!(!applies("x [AllowLoadTextLocale enUS]", retail));
        assert!(applies("x [AllowLoad Game]", retail));
        assert!(applies("x [AllowLoad both]", retail));
        assert!(!applies("x [AllowLoad Glue]", retail));
        assert!(applies("x [AllowLoadEnvironment Global]", retail));
        assert!(!applies("x [AllowLoadEnvironment Secure]", retail));
        assert!(!applies("x [Bootstrap]", retail));
        assert!(applies("x [LoadIntoEnvironment Secure]", retail));
    }
}
