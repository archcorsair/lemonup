//! The one single-line text box every text field uses: the `/` filter, the
//! Install box, a Settings value being edited, and Setup's folder and
//! Wago.io key boxes.
//!
//! It holds the text and a cursor. The cursor counts characters, not bytes,
//! so `é` and `中` move and delete as one. The box draws itself into a given
//! width and scrolls sideways so the cursor is always on screen.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

/// Pasted text goes into single-line fields: drop newlines and other control
/// characters (a trailing newline from the clipboard must not submit or
/// corrupt a path or API key). Backslashes and quotes are ordinary
/// characters, so a pasted Windows path arrives as it was copied.
pub(crate) fn sanitize_single_line_paste(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .collect()
}

/// An edit a key asks for, other than typing a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEdit {
    Left,
    Right,
    Home,
    End,
    Delete,
    /// Ctrl+U: empty the box.
    Clear,
}

/// A single line of text and a cursor, which sits before the character at
/// `cursor` (or after the last one when it equals the length).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl From<&str> for TextInput {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for TextInput {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl PartialEq<&str> for TextInput {
    fn eq(&self, other: &&str) -> bool {
        self.value == *other
    }
}

impl PartialEq<str> for TextInput {
    fn eq(&self, other: &str) -> bool {
        self.value == other
    }
}

impl TextInput {
    /// A box holding `value`, with the cursor at the end.
    pub(crate) fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    /// The same box with every character drawn as `mask`, cursor kept: a
    /// secret is edited without being shown.
    pub(crate) fn masked(&self, mask: char) -> Self {
        Self {
            value: std::iter::repeat_n(mask, self.value.chars().count()).collect(),
            cursor: self.cursor,
        }
    }

    /// The cursor, in characters from the start.
    #[cfg(test)]
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    /// The same text with the cursor at the end.
    pub(crate) fn with_cursor_at_end(mut self) -> Self {
        self.cursor = self.len();
        self
    }

    /// The byte offset of character `index` (the end for `index == len`).
    fn byte_at(&self, index: usize) -> usize {
        self.value
            .char_indices()
            .nth(index)
            .map_or(self.value.len(), |(offset, _)| offset)
    }

    pub(crate) fn insert_char(&mut self, character: char) {
        if character.is_control() {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.value.insert(at, character);
        self.cursor += 1;
    }

    /// Inserts pasted text at the cursor, without newlines or other control
    /// characters, and leaves the cursor after it.
    pub(crate) fn insert_str(&mut self, text: &str) {
        let text = sanitize_single_line_paste(text);
        let at = self.byte_at(self.cursor);
        self.value.insert_str(at, &text);
        self.cursor += text.chars().count();
    }

    pub(crate) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte_at(self.cursor);
        self.value.remove(at);
    }

    pub(crate) fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_at(self.cursor);
            self.value.remove(at);
        }
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.len();
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn apply(&mut self, edit: TextEdit) {
        match edit {
            TextEdit::Left => self.move_left(),
            TextEdit::Right => self.move_right(),
            TextEdit::Home => self.move_home(),
            TextEdit::End => self.move_end(),
            TextEdit::Delete => self.delete(),
            TextEdit::Clear => self.clear(),
        }
    }

    /// How many columns the text takes, without the cursor.
    pub(crate) fn width(&self) -> usize {
        Span::raw(self.value.as_str()).width()
    }

    /// The box drawn into `width` columns. With focus the cursor shows: `▌`
    /// in `caret` after the last character, or the character under it in
    /// reverse video. Text that does not fit is cut with `…`, from the left
    /// when the cursor is near the end (or without focus), else around the
    /// cursor, so the cursor is always on screen. With focus the text takes
    /// at most `width - 1` columns, the last one kept for the end cursor.
    pub(crate) fn spans(
        &self,
        width: usize,
        text: Style,
        caret: Style,
        focused: bool,
    ) -> Vec<Span<'static>> {
        let view = self.view(width, focused);
        let mut spans = Vec::new();
        if view.cut_left {
            spans.push(Span::styled("…", text));
        }
        if !view.before.is_empty() {
            spans.push(Span::styled(view.before, text));
        }
        if let Some(under) = view.under {
            spans.push(Span::styled(
                under.to_string(),
                caret.add_modifier(Modifier::REVERSED),
            ));
        }
        if !view.after.is_empty() {
            spans.push(Span::styled(view.after, text));
        }
        if view.cut_right {
            spans.push(Span::styled("…", text));
        }
        if view.end_caret {
            spans.push(Span::styled("▌", caret));
        }
        spans
    }

    /// Which part of the text shows in `width` columns, and where the
    /// cursor is in it. See `spans`. A box too narrow for the "…" marks
    /// drops them before it drops the cursor.
    fn view(&self, width: usize, focused: bool) -> TextView {
        let mut view = self.window(width, focused);
        if view.width() > width {
            view.cut_left = false;
        }
        if view.width() > width {
            view.cut_right = false;
        }
        view
    }

    fn window(&self, width: usize, focused: bool) -> TextView {
        let chars: Vec<char> = self.value.chars().collect();
        let widths: Vec<usize> = chars
            .iter()
            .map(|&character| char_width(character))
            .collect();
        let len = chars.len();
        let cursor = self.cursor.min(len);
        let at_end = cursor == len;
        let slot = usize::from(focused);
        let collect = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
        let span_width = |range: std::ops::Range<usize>| widths[range].iter().sum::<usize>();
        let make = |start: usize, end: usize| {
            let (before, under, after) = if focused && !at_end {
                (
                    collect(start..cursor),
                    Some(chars[cursor]),
                    collect(cursor + 1..end),
                )
            } else {
                (collect(start..end), None, String::new())
            };
            TextView {
                cut_left: start > 0,
                before,
                under,
                after,
                cut_right: end < len,
                end_caret: focused && at_end,
            }
        };

        if width == 0 {
            return TextView::default();
        }
        if span_width(0..len) + slot <= width {
            return make(0, len);
        }

        // The longest end of the text that fits after a "…".
        let room = width.saturating_sub(1 + slot);
        let mut start = len;
        while start > 0 && span_width(start - 1..len) <= room {
            start -= 1;
        }
        if !focused || cursor >= start {
            return make(start, len);
        }

        // The cursor is further left: show it in the middle, with a "…" on
        // the right and, unless the window starts at the beginning, on the
        // left too.
        let mut start = cursor;
        let half = width.saturating_sub(2) / 2;
        while start > 0 && span_width(start - 1..cursor) < half {
            start -= 1;
        }
        let room = width.saturating_sub(usize::from(start > 0) + 1);
        let mut end = cursor + 1;
        while end < len && span_width(start..end + 1) <= room {
            end += 1;
        }
        // A wide character under the cursor in a very narrow box: keep the
        // cursor and drop the rest.
        if span_width(start..end) > room {
            start = cursor;
            end = cursor + 1;
        }
        make(start, end)
    }
}

/// The part of a box that shows.
#[derive(Debug, Default, PartialEq, Eq)]
struct TextView {
    cut_left: bool,
    before: String,
    under: Option<char>,
    after: String,
    cut_right: bool,
    end_caret: bool,
}

impl TextView {
    fn width(&self) -> usize {
        usize::from(self.cut_left)
            + Span::raw(self.before.as_str()).width()
            + self.under.map_or(0, char_width)
            + Span::raw(self.after.as_str()).width()
            + usize::from(self.cut_right)
            + usize::from(self.end_caret)
    }
}

pub(super) fn char_width(character: char) -> usize {
    let mut buffer = [0u8; 4];
    Span::raw(&*character.encode_utf8(&mut buffer)).width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn typed(text: &str) -> TextInput {
        let mut input = TextInput::default();
        for character in text.chars() {
            input.insert_char(character);
        }
        input
    }

    /// The drawn box as text, with the cursor shown as `[x]` for a reversed
    /// character.
    fn drawn(input: &TextInput, width: usize, focused: bool) -> String {
        input
            .spans(
                width,
                Style::default(),
                Style::default().fg(Color::Yellow),
                focused,
            )
            .iter()
            .map(|span| {
                if span.style.add_modifier.contains(Modifier::REVERSED) {
                    format!("[{}]", span.content)
                } else {
                    span.content.to_string()
                }
            })
            .collect()
    }

    fn drawn_width(input: &TextInput, width: usize, focused: bool) -> usize {
        input
            .spans(width, Style::default(), Style::default(), focused)
            .iter()
            .map(Span::width)
            .sum()
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut input = typed("ac");
        assert_eq!(input.cursor(), 2);
        input.move_left();
        input.insert_char('b');
        assert_eq!(input, "abc");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn control_characters_are_not_typed() {
        let mut input = typed("a");
        input.insert_char('\n');
        input.insert_char('\t');
        assert_eq!(input, "a");
        assert_eq!(input.cursor(), 1);
    }

    #[test]
    fn paste_goes_in_at_the_cursor_without_newlines() {
        let mut input = TextInput::new("C:\\Games\\AddOns");
        for _ in 0.."\\AddOns".chars().count() {
            input.move_left();
        }
        input.insert_str("\\World of \"Warcraft\"\r\n");
        assert_eq!(input, "C:\\Games\\World of \"Warcraft\"\\AddOns");
        assert_eq!(
            input.cursor(),
            "C:\\Games\\World of \"Warcraft\"".chars().count()
        );
    }

    #[test]
    fn paste_in_the_middle_of_multibyte_text() {
        let mut input = TextInput::new("é中");
        input.move_left();
        input.insert_str("ü文");
        assert_eq!(input, "éü文中");
        assert_eq!(input.cursor(), 3);
    }

    #[test]
    fn backspace_deletes_before_the_cursor() {
        let mut input = TextInput::new("abc");
        input.move_left();
        input.backspace();
        assert_eq!(input, "ac");
        assert_eq!(input.cursor(), 1);
        input.move_home();
        input.backspace();
        assert_eq!(input, "ac");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn delete_deletes_under_the_cursor() {
        let mut input = TextInput::new("abc");
        input.move_home();
        input.delete();
        assert_eq!(input, "bc");
        assert_eq!(input.cursor(), 0);
        input.move_end();
        input.delete();
        assert_eq!(input, "bc");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn multibyte_characters_move_and_delete_as_one() {
        let mut input = TextInput::new("aé中b");
        assert_eq!(input.cursor(), 4);
        input.move_left();
        input.backspace();
        assert_eq!(input, "aéb");
        input.move_left();
        input.delete();
        assert_eq!(input, "ab");
        input.insert_char('中');
        input.insert_char('é');
        assert_eq!(input, "a中éb");
        assert_eq!(input.cursor(), 3);
        assert_eq!(input.width(), 5);
    }

    #[test]
    fn arrows_home_and_end_stop_at_the_ends() {
        let mut input = TextInput::new("ab");
        input.move_right();
        assert_eq!(input.cursor(), 2);
        input.move_left();
        input.move_left();
        input.move_left();
        assert_eq!(input.cursor(), 0);
        input.move_end();
        assert_eq!(input.cursor(), 2);
        input.move_home();
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn clear_empties_the_box() {
        let mut input = TextInput::new("abc");
        input.move_left();
        input.apply(TextEdit::Clear);
        assert_eq!(input, TextInput::default());
        assert!(input.is_empty());
    }

    #[test]
    fn a_new_box_and_with_cursor_at_end_put_the_cursor_at_the_end() {
        assert_eq!(TextInput::new("é中").cursor(), 2);
        assert_eq!(TextInput::from("abc".to_string()).cursor(), 3);
        let mut moved = TextInput::new("xyz");
        moved.move_home();
        assert_eq!(moved.with_cursor_at_end().cursor(), 3);
    }

    #[test]
    fn apply_runs_each_edit() {
        let mut input = TextInput::new("abcd");
        for (edit, value, cursor) in [
            (TextEdit::Left, "abcd", 3),
            (TextEdit::Delete, "abc", 3),
            (TextEdit::Home, "abc", 0),
            (TextEdit::Delete, "bc", 0),
            (TextEdit::Right, "bc", 1),
            (TextEdit::End, "bc", 2),
            (TextEdit::Clear, "", 0),
        ] {
            input.apply(edit);
            assert_eq!((input.value(), input.cursor()), (value, cursor), "{edit:?}");
        }
    }

    #[test]
    fn draws_the_caret_at_the_end_and_reverses_the_character_under_it() {
        let mut input = TextInput::new("abc");
        assert_eq!(drawn(&input, 20, true), "abc▌");
        assert_eq!(drawn(&input, 20, false), "abc");
        input.move_left();
        assert_eq!(drawn(&input, 20, true), "ab[c]");
        input.move_home();
        assert_eq!(drawn(&input, 20, true), "[a]bc");
        assert_eq!(drawn(&TextInput::default(), 20, true), "▌");
    }

    #[test]
    fn long_text_cuts_from_the_left_while_the_cursor_is_near_the_end() {
        let input = TextInput::new("abcdefghij");
        assert_eq!(drawn(&input, 6, true), "…ghij▌");
        assert_eq!(drawn(&input, 6, false), "…fghij");
        let mut near_end = input.clone();
        near_end.move_left();
        near_end.move_left();
        assert_eq!(drawn(&near_end, 6, true), "…gh[i]j");
    }

    #[test]
    fn long_text_scrolls_to_keep_the_cursor_on_screen() {
        let mut input = TextInput::new("abcdefghij");
        input.move_home();
        assert_eq!(drawn(&input, 6, true), "[a]bcde…");
        for _ in 0..5 {
            input.move_right();
        }
        let shown = drawn(&input, 6, true);
        assert!(shown.contains("[f]"), "{shown}");
        assert!(shown.starts_with('…') && shown.ends_with('…'), "{shown}");
        for width in 1..12 {
            for cursor in 0..=10 {
                let mut moved = TextInput::new("abcdefghij");
                moved.move_home();
                for _ in 0..cursor {
                    moved.move_right();
                }
                assert!(
                    drawn_width(&moved, width, true) <= width,
                    "{width} {cursor}"
                );
                let shown = drawn(&moved, width, true);
                if cursor < 10 {
                    let under = moved.value().chars().nth(cursor).expect("char");
                    assert!(
                        shown.contains(&format!("[{under}]")),
                        "{width} {cursor} {shown}"
                    );
                } else {
                    assert!(shown.ends_with('▌'), "{width} {cursor} {shown}");
                }
            }
        }
    }

    #[test]
    fn wide_characters_count_two_columns_when_scrolling() {
        let mut input = TextInput::new("中文中文中文");
        assert_eq!(drawn_width(&input, 8, true), 8);
        assert_eq!(drawn(&input, 8, true), "…文中文▌");
        input.move_home();
        assert_eq!(drawn(&input, 8, true), "[中]文中…");
        for width in 1..14 {
            assert!(drawn_width(&input, width, true) <= width.max(2), "{width}");
        }
    }

    #[test]
    fn zero_width_draws_nothing() {
        assert!(
            TextInput::new("abc")
                .spans(0, Style::default(), Style::default(), true)
                .is_empty()
        );
    }
}
