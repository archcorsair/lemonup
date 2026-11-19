//! How Install search orders what a source returns.
//!
//! Wago.io matches any word of a query and sorts by downloads, so a search
//! for three words can bring back addons that share only one. This module
//! turns the query into words and ranks the names so the ones that have
//! every word come first. Words that only name a game client ("forever",
//! "classic", ...) still count, but a name that has the other words ranks
//! ahead of one that only has a game word. It needs no network.

/// Words that name a game client. A name with a typed game word is still a
/// match, but it ranks below a name with more of the other typed words.
pub const GAME_WORDS: [&str; 4] = ["forever", "classic", "retail", "cata"];

/// A query split into the words to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    /// The typed words that are not game words, lowercase, each once. When
    /// only game words were typed, they are the words and `game` is empty.
    pub words: Vec<String>,
    /// The typed game words, lowercase, each once.
    pub game: Vec<String>,
}

impl ParsedQuery {
    /// How many words a name needs to have every word of the query.
    fn total(&self) -> usize {
        self.words.len() + self.game.len()
    }
}

/// One result to rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchItem<'a> {
    pub name: &'a str,
    pub downloads: u64,
}

/// The ranked order of a list of results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRanking {
    /// Indexes into the input, best match first.
    pub order: Vec<usize>,
    /// Whether at least one name has every typed word, game words included.
    /// True when there were no words to look for.
    pub any_has_every_word: bool,
}

/// Splits `query` into its words. A token that is a game word goes to
/// `game`, unless nothing else was typed.
pub fn parse_query(query: &str, game_words: &[&str]) -> ParsedQuery {
    let mut words = Vec::new();
    let mut game = Vec::new();
    for token in query.split_whitespace() {
        let token_words = words_of(token, false);
        let is_game = token_words.len() == 1
            && game_words
                .iter()
                .any(|word| word.eq_ignore_ascii_case(&token_words[0]));
        let target = if is_game { &mut game } else { &mut words };
        for word in token_words {
            if !target.contains(&word) {
                target.push(word);
            }
        }
    }
    if words.is_empty() {
        words = std::mem::take(&mut game);
    }
    ParsedQuery { words, game }
}

/// Orders `items` for `query`: names with every typed word first, then by
/// how many of the other words the name has, then by how many game words it
/// has, then downloads (most first). Equal items keep their input order.
pub fn rank_results(query: &str, game_words: &[&str], items: &[SearchItem<'_>]) -> SearchRanking {
    let parsed = parse_query(query, game_words);
    let scores: Vec<NameScore> = items
        .iter()
        .map(|item| score_name(item.name, &parsed))
        .collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&left, &right| {
        scores[right]
            .words
            .cmp(&scores[left].words)
            .then_with(|| scores[right].game.cmp(&scores[left].game))
            .then_with(|| items[right].downloads.cmp(&items[left].downloads))
    });
    let any_has_every_word =
        parsed.total() == 0 || scores.iter().any(|score| score.total() == parsed.total());
    SearchRanking {
        order,
        any_has_every_word,
    }
}

/// Whether at least one of `names` has every typed word of `query`, game
/// words included. True when the query has no words to look for, false when
/// there are no names.
pub fn any_name_has_every_word(query: &str, game_words: &[&str], names: &[&str]) -> bool {
    let parsed = parse_query(query, game_words);
    !names.is_empty()
        && (parsed.total() == 0
            || names
                .iter()
                .any(|name| score_name(name, &parsed).total() == parsed.total()))
}

/// How many of the typed words `name` has. A typed word matches a word of
/// the name when the name word starts with it ("sparkle" finds "Sparkles"),
/// ignoring case and WoW color codes. A glued name also counts as its parts:
/// "ForeverAuras" has "forever" and "auras".
pub fn matched_word_count(name: &str, query: &str, game_words: &[&str]) -> usize {
    score_name(name, &parse_query(query, game_words)).total()
}

struct NameScore {
    /// Typed words that are not game words, found in the name.
    words: usize,
    /// Typed game words found in the name.
    game: usize,
}

impl NameScore {
    fn total(&self) -> usize {
        self.words + self.game
    }
}

fn score_name(name: &str, parsed: &ParsedQuery) -> NameScore {
    let name_words = words_of(&strip_color_codes(name), true);
    let count = |typed: &[String]| {
        typed
            .iter()
            .filter(|word| {
                name_words
                    .iter()
                    .any(|candidate| candidate.starts_with(word.as_str()))
            })
            .count()
    };
    NameScore {
        words: count(&parsed.words),
        game: count(&parsed.game),
    }
}

/// Lowercase runs of letters and digits. Apostrophes join a word ("Don't"
/// is "dont").
///
/// With `split_case`, a run is also split where the case changes, so a glued
/// name gives its parts as well as the whole run: "AuraDurationsForever" is
/// "auradurationsforever", "aura", "durations" and "forever", and "XMLParser"
/// is "xmlparser", "xml" and "parser".
fn words_of(text: &str, split_case: bool) -> Vec<String> {
    let mut runs: Vec<Vec<char>> = Vec::new();
    let mut current = Vec::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.push(character);
        } else if character != '\'' && character != '\u{2019}' && !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }

    let lower = |chars: &[char]| {
        chars
            .iter()
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    };
    let mut words = Vec::new();
    for run in runs {
        words.push(lower(&run));
        if !split_case {
            continue;
        }
        let mut start = 0;
        let mut parts = Vec::new();
        for index in 1..run.len() {
            let after_lower = run[index - 1].is_lowercase() && run[index].is_uppercase();
            let before_word = run[index - 1].is_uppercase()
                && run[index].is_uppercase()
                && run.get(index + 1).is_some_and(|next| next.is_lowercase());
            if after_lower || before_word {
                parts.push(lower(&run[start..index]));
                start = index;
            }
        }
        if start > 0 {
            parts.push(lower(&run[start..]));
            words.extend(parts);
        }
    }
    words
}

/// `text` without WoW color codes: `|cAARRGGBB` and `|r`.
fn strip_color_codes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '|' {
            match chars.get(index + 1) {
                Some('c' | 'C')
                    if chars.len() >= index + 10
                        && chars[index + 2..index + 10]
                            .iter()
                            .all(char::is_ascii_hexdigit) =>
                {
                    index += 10;
                    continue;
                }
                Some('r' | 'R') => {
                    index += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, downloads: u64) -> SearchItem<'_> {
        SearchItem { name, downloads }
    }

    fn ranked<'a>(query: &str, items: &[SearchItem<'a>]) -> Vec<&'a str> {
        rank_results(query, &GAME_WORDS, items)
            .order
            .into_iter()
            .map(|index| items[index].name)
            .collect()
    }

    #[test]
    fn names_with_every_word_come_before_more_downloaded_partial_matches() {
        let items = [
            item("Forever Tools", 900_000),
            item("Loot Helper", 500_000),
            item("Loot Sparkles", 10),
        ];
        assert_eq!(
            ranked("Loot Sparkles", &items),
            ["Loot Sparkles", "Loot Helper", "Forever Tools"]
        );
    }

    #[test]
    fn more_matched_words_rank_higher_before_downloads() {
        let items = [
            item("Alpha", 1_000),
            item("Alpha Beta", 10),
            item("Alpha Beta Gamma", 1),
        ];
        assert_eq!(
            ranked("alpha beta gamma delta", &items),
            ["Alpha Beta Gamma", "Alpha Beta", "Alpha"]
        );
    }

    #[test]
    fn downloads_break_ties_and_equal_items_keep_their_order() {
        let items = [
            item("Loot A", 5),
            item("Loot B", 50),
            item("Loot C", 5),
            item("Loot D", 5),
        ];
        assert_eq!(
            ranked("loot", &items),
            ["Loot B", "Loot A", "Loot C", "Loot D"]
        );
    }

    #[test]
    fn matching_ignores_case_and_matches_word_prefixes() {
        let items = [item("Other", 9), item("LOOT SPARKLES", 1)];
        assert_eq!(ranked("loot sparkle", &items), ["LOOT SPARKLES", "Other"]);
        // A search word inside a name word does not count, only its start.
        assert_eq!(matched_word_count("Weakauras", "aura", &GAME_WORDS), 0);
        assert_eq!(matched_word_count("Auras Plus", "aura", &GAME_WORDS), 1);
    }

    #[test]
    fn color_codes_do_not_hide_a_word() {
        assert_eq!(
            matched_word_count("|cff00ff00Loot|r Sparkles", "loot sparkles", &GAME_WORDS),
            2
        );
        // The code is not part of the name.
        assert_eq!(
            matched_word_count("|cff00ff00Loot|r", "cff00ff00loot", &GAME_WORDS),
            0
        );
    }

    #[test]
    fn game_words_are_kept_as_their_own_typed_words() {
        let parsed = parse_query("Forever Loot Sparkles", &GAME_WORDS);
        assert_eq!(parsed.words, ["loot", "sparkles"]);
        assert_eq!(parsed.game, ["forever"]);
        let parsed = parse_query("loot CLASSIC, retail", &GAME_WORDS);
        assert_eq!(parsed.words, ["loot"]);
        assert_eq!(parsed.game, ["classic", "retail"]);
    }

    #[test]
    fn a_query_of_only_game_words_uses_them_as_the_words() {
        let parsed = parse_query("  Forever  Classic ", &GAME_WORDS);
        assert_eq!(parsed.words, ["forever", "classic"]);
        assert!(parsed.game.is_empty());
        // A game word inside a longer word is not a game word.
        let parsed = parse_query("Classically Loot", &GAME_WORDS);
        assert_eq!(parsed.words, ["classically", "loot"]);
        assert!(parsed.game.is_empty());
    }

    #[test]
    fn a_name_with_a_game_word_ranks_above_one_without() {
        let items = [
            item("Loot Sparkles", 1_000),
            item("Forever Loot Sparkles", 5),
        ];
        assert_eq!(
            ranked("Forever Loot Sparkles", &items),
            ["Forever Loot Sparkles", "Loot Sparkles"]
        );
        // The game word is a typed word: only the second name has every word.
        assert!(rank_results("Forever Loot Sparkles", &GAME_WORDS, &items).any_has_every_word);
        let items = [item("Loot Sparkles", 1_000)];
        assert!(!rank_results("Forever Loot Sparkles", &GAME_WORDS, &items).any_has_every_word);
        // A game word only breaks ties between equal counts of other words.
        let items = [item("Forever Loot", 5), item("Loot Sparkles", 1)];
        assert_eq!(
            ranked("Forever Loot Sparkles", &items),
            ["Loot Sparkles", "Forever Loot"]
        );
    }

    #[test]
    fn a_glued_name_matches_the_game_word_and_the_other_word() {
        let items = [
            item("Forever Core", 3_505),
            item("Arc Auras", 1_004),
            item("ForeverAuras", 10),
        ];
        assert_eq!(
            ranked("forever auras", &items),
            ["ForeverAuras", "Arc Auras", "Forever Core"]
        );
    }

    #[test]
    fn real_words_rank_before_game_words() {
        let items = [item("Forever Core", 900_000), item("Questie", 10)];
        assert_eq!(
            ranked("questie forever", &items),
            ["Questie", "Forever Core"]
        );
    }

    #[test]
    fn a_glued_query_matches_the_whole_glued_name() {
        let items = [item("Arc Auras", 1_000), item("ForeverAuras", 1)];
        assert_eq!(
            ranked("foreverauras", &items),
            ["ForeverAuras", "Arc Auras"]
        );
    }

    #[test]
    fn names_are_split_where_the_case_changes() {
        let words = |name: &str| words_of(name, true);
        assert_eq!(words("ForeverAuras"), ["foreverauras", "forever", "auras"]);
        assert_eq!(
            words("AuraDurationsForever"),
            ["auradurationsforever", "aura", "durations", "forever"]
        );
        assert_eq!(words("RareScanner"), ["rarescanner", "rare", "scanner"]);
        assert_eq!(words("XMLParser"), ["xmlparser", "xml", "parser"]);
        assert_eq!(words("Loot Sparkles"), ["loot", "sparkles"]);
        assert_eq!(words("LOOT"), ["loot"]);
        // The typed query is not split by case.
        assert_eq!(words_of("ForeverAuras", false), ["foreverauras"]);
    }

    #[test]
    fn reports_when_no_name_has_every_word() {
        let items = [item("Loot Helper", 1), item("Forever Tools", 9)];
        let ranking = rank_results("Forever Loot Sparkles", &GAME_WORDS, &items);
        assert!(!ranking.any_has_every_word);
        assert_eq!(ranking.order, [0, 1]);
        let items = [item("Loot Sparkles Plus", 1)];
        assert!(rank_results("loot sparkles", &GAME_WORDS, &items).any_has_every_word);
    }

    #[test]
    fn any_name_has_every_word_counts_the_game_word() {
        let names = ["Loot Helper", "Sparkles Loot Forever"];
        assert!(any_name_has_every_word(
            "forever loot sparkles",
            &GAME_WORDS,
            &names
        ));
        assert!(!any_name_has_every_word(
            "forever loot sparkles",
            &GAME_WORDS,
            &names[..1]
        ));
        // Without the game word, "Loot Sparkles" has every typed word.
        assert!(!any_name_has_every_word(
            "forever loot sparkles",
            &GAME_WORDS,
            &["Loot Sparkles"]
        ));
        assert!(any_name_has_every_word(
            "forever auras",
            &GAME_WORDS,
            &["ForeverAuras"]
        ));
        assert!(!any_name_has_every_word("loot", &GAME_WORDS, &[]));
    }

    #[test]
    fn a_query_with_no_words_orders_by_downloads_and_reports_a_match() {
        let items = [item("B", 1), item("A", 2)];
        let ranking = rank_results("!!!", &GAME_WORDS, &items);
        assert!(ranking.any_has_every_word);
        assert_eq!(ranking.order, [1, 0]);
    }

    #[test]
    fn no_results_report_no_match() {
        let ranking = rank_results("loot", &GAME_WORDS, &[]);
        assert!(ranking.order.is_empty());
        assert!(!ranking.any_has_every_word);
    }
}
