//! How Install search orders what a source returns.
//!
//! Wago.io matches any word of a query and sorts by downloads, so a search
//! for three words can bring back addons that share only one. This module
//! turns the query into words, drops the game words that only name a client
//! ("forever", "classic", ...), and ranks the names so the ones that have
//! every word come first. It needs no network.

/// Words that name a game client. They are left out of the query sent to
/// the source and out of the word count, unless nothing else was typed.
pub const GAME_WORDS: [&str; 4] = ["forever", "classic", "retail", "cata"];

/// A query split into what to send and what to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    /// The text to send to the source.
    pub send: String,
    /// The words a name is matched against, lowercase, each once.
    pub words: Vec<String>,
    /// The game words that were left out, lowercase, each once.
    pub dropped: Vec<String>,
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
    /// Whether at least one name has every search word. True when there
    /// were no words to look for.
    pub any_has_every_word: bool,
}

/// Splits `query` into the text to send and the words to match.
///
/// A game word is dropped only when at least one other word remains. A
/// query of only game words is sent unchanged.
pub fn parse_query(query: &str, game_words: &[&str]) -> ParsedQuery {
    let trimmed = query.trim();
    let tokens: Vec<(&str, Vec<String>)> = trimmed
        .split_whitespace()
        .map(|token| (token, words_of(token)))
        .collect();
    let is_game_token = |words: &[String]| {
        words.len() == 1
            && game_words
                .iter()
                .any(|game| game.eq_ignore_ascii_case(&words[0]))
    };
    let has_game_token = tokens.iter().any(|(_, words)| is_game_token(words));
    let has_other_word = tokens
        .iter()
        .any(|(_, words)| !words.is_empty() && !is_game_token(words));
    let dropping = has_game_token && has_other_word;

    let mut words = Vec::new();
    let mut dropped = Vec::new();
    let mut kept_tokens = Vec::new();
    for (token, token_words) in &tokens {
        let target = if dropping && is_game_token(token_words) {
            &mut dropped
        } else {
            kept_tokens.push(*token);
            &mut words
        };
        for word in token_words {
            if !target.contains(word) {
                target.push(word.clone());
            }
        }
    }
    ParsedQuery {
        send: if dropping {
            kept_tokens.join(" ")
        } else {
            trimmed.to_string()
        },
        words,
        dropped,
    }
}

/// The text to send to the source for `query`.
pub fn source_query(query: &str, game_words: &[&str]) -> String {
    parse_query(query, game_words).send
}

/// Orders `items` for `query`: names with every search word first, then by
/// how many search words the name has, then names that have a game word the
/// query left out, then downloads (most first). Equal items keep their
/// input order.
pub fn rank_results(query: &str, game_words: &[&str], items: &[SearchItem<'_>]) -> SearchRanking {
    let parsed = parse_query(query, game_words);
    let scores: Vec<NameScore> = items
        .iter()
        .map(|item| score_name(item.name, &parsed))
        .collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&left, &right| {
        scores[right]
            .matched
            .cmp(&scores[left].matched)
            .then_with(|| scores[right].has_dropped.cmp(&scores[left].has_dropped))
            .then_with(|| items[right].downloads.cmp(&items[left].downloads))
    });
    let any_has_every_word = parsed.words.is_empty()
        || scores
            .iter()
            .any(|score| score.matched == parsed.words.len());
    SearchRanking {
        order,
        any_has_every_word,
    }
}

/// Whether at least one of `names` has every search word of `query`. True
/// when the query has no words to look for, false when there are no names.
pub fn any_name_has_every_word(query: &str, game_words: &[&str], names: &[&str]) -> bool {
    let parsed = parse_query(query, game_words);
    !names.is_empty()
        && (parsed.words.is_empty()
            || names
                .iter()
                .any(|name| score_name(name, &parsed).matched == parsed.words.len()))
}

/// How many of the search words `name` has. A search word matches a word of
/// the name when the name word starts with it ("sparkle" finds "Sparkles"),
/// ignoring case and WoW color codes.
pub fn matched_word_count(name: &str, query: &str, game_words: &[&str]) -> usize {
    score_name(name, &parse_query(query, game_words)).matched
}

struct NameScore {
    matched: usize,
    has_dropped: bool,
}

fn score_name(name: &str, parsed: &ParsedQuery) -> NameScore {
    let name_words = words_of(&strip_color_codes(name));
    let has = |word: &String| {
        name_words
            .iter()
            .any(|candidate| candidate.starts_with(word.as_str()))
    };
    NameScore {
        matched: parsed.words.iter().filter(|word| has(word)).count(),
        has_dropped: parsed.dropped.iter().any(has),
    }
}

/// Lowercase runs of letters and digits. Apostrophes join a word ("Don't"
/// is "dont").
fn words_of(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if character != '\'' && character != '\u{2019}' && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
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
        assert_eq!(matched_word_count("WeakAuras", "aura", &GAME_WORDS), 0);
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
    fn game_words_are_dropped_from_the_query_when_other_words_remain() {
        let parsed = parse_query("Forever Loot Sparkles", &GAME_WORDS);
        assert_eq!(parsed.send, "Loot Sparkles");
        assert_eq!(parsed.words, ["loot", "sparkles"]);
        assert_eq!(parsed.dropped, ["forever"]);
        assert_eq!(source_query("loot CLASSIC, retail", &GAME_WORDS), "loot");
    }

    #[test]
    fn a_query_of_only_game_words_is_sent_unchanged() {
        let parsed = parse_query("  Forever  Classic ", &GAME_WORDS);
        assert_eq!(parsed.send, "Forever  Classic");
        assert_eq!(parsed.words, ["forever", "classic"]);
        assert!(parsed.dropped.is_empty());
        assert_eq!(source_query("cata", &GAME_WORDS), "cata");
    }

    #[test]
    fn a_query_without_game_words_is_sent_as_typed() {
        assert_eq!(
            source_query(" WeakAuras-Options ", &GAME_WORDS),
            "WeakAuras-Options"
        );
        // A game word inside a longer word is not a game word.
        assert_eq!(
            source_query("Classically Loot", &GAME_WORDS),
            "Classically Loot"
        );
    }

    #[test]
    fn a_name_with_a_dropped_game_word_ranks_above_one_without() {
        let items = [
            item("Loot Sparkles", 1_000),
            item("Forever Loot Sparkles", 5),
        ];
        assert_eq!(
            ranked("Forever Loot Sparkles", &items),
            ["Forever Loot Sparkles", "Loot Sparkles"]
        );
        // The game word is not a search word: both names have every word.
        assert!(rank_results("Forever Loot Sparkles", &GAME_WORDS, &items).any_has_every_word);
        // It only breaks ties between equal ranks.
        let items = [item("Forever Loot", 5), item("Loot Sparkles", 1)];
        assert_eq!(
            ranked("Forever Loot Sparkles", &items),
            ["Loot Sparkles", "Forever Loot"]
        );
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
    fn any_name_has_every_word_ignores_the_game_word() {
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
