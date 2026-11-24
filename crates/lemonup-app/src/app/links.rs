//! Links on screen that open with a click (OSC 8), where the terminal has links.
//!
//! A ratatui cell can't hold the escape sequence: its width would count the address. So the
//! frame is drawn as usual, then each link's cells are written again, inside the sequence,
//! straight to the terminal. Cells ratatui doesn't redraw keep the link; when the links
//! change, the cells of the old ones are written again without it and the new ones with it.

use std::io::{self, Write};

use ratatui::backend::{Backend as _, CrosstermBackend};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

use crate::event::TerminalEvent;

/// The text shown on screen and the address it opens.
const LINKS: [(&str, &str); 2] = [
    (
        "lemonup.org/github-token",
        "https://lemonup.org/github-token",
    ),
    ("addons.wago.io/patreon", "https://addons.wago.io/patreon"),
];

/// The longest address written as a link. Some terminals drop a longer OSC 8 sequence.
const MAX_ADDRESS_LEN: usize = 2048;

/// `url` as an address safe to write inside OSC 8: an http or https address, as `Url` writes
/// it, with no control or non-ASCII characters left. `None` for anything else, for a host
/// that is an IP address or has no dot, and for an address with a user name or password, which can make it look
/// like another site (`https://github.com@example.com`).
pub(super) fn link_address(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.host_str().is_some_and(is_named_host)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let address = String::from(parsed);
    (address.len() <= MAX_ADDRESS_LEN && address.chars().all(|letter| letter.is_ascii_graphic()))
        .then_some(address)
}

/// A host name of two or more parts (`github.com`), not an IP address.
fn is_named_host(host: &str) -> bool {
    !host.starts_with('[')
        && host.parse::<std::net::Ipv4Addr>().is_err()
        && host.split('.').count() > 1
        && host.split('.').all(|part| !part.is_empty())
}

/// One row of a link on screen: where its text starts, the address, and its cells as drawn.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DrawnLink {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) url: String,
    cells: Vec<Cell>,
}

/// The links in a drawn frame, at the size it was drawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct DrawnLinks {
    area: Rect,
    links: Vec<DrawnLink>,
}

impl DrawnLinks {
    /// Every place a link's text is drawn whole in `buffer`: the fixed links, and `extra`
    /// (text and address) for the ones that change, such as the web page in Details. A text
    /// may be wrapped over rows below one another, starting in the same column, each row but
    /// the last followed by a blank cell or the edge. An `extra` text, which a source can
    /// choose, also needs a blank cell or the edge before each row and after its end. A text
    /// that starts on a link already found is skipped. Text with wide letters, which take two
    /// cells, is never found.
    pub(super) fn find(buffer: &Buffer, extra: &[(String, String)]) -> Self {
        let area = buffer.area;
        let rows: Vec<Vec<&str>> = (area.top()..area.bottom())
            .map(|y| {
                (area.left()..area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect();
        // The given links first: a fixed link's text can be the start of one.
        let texts: Vec<(Vec<char>, &str, bool)> = extra
            .iter()
            .map(|(text, url)| (text.as_str(), url.as_str(), true))
            .chain(LINKS.iter().map(|(text, url)| (*text, *url, false)))
            .filter(|(text, _, _)| !text.is_empty())
            .map(|(text, url, apart)| (text.chars().collect(), url, apart))
            .collect();
        let mut links = Vec::new();
        for y in 0..rows.len() {
            for (chars, url, apart) in &texts {
                let mut start = 0;
                while start < rows[y].len() {
                    let x = area.left() + start as u16;
                    let on_a_link = || {
                        links.iter().any(|link: &DrawnLink| {
                            link.y == area.top() + y as u16
                                && (link.x..link.x + link.cells.len() as u16).contains(&x)
                        })
                    };
                    let starts_here = rows[y][start].starts_with(chars[0]);
                    let Some(pieces) = starts_here
                        .then(|| place(&rows, start, y, chars, *apart))
                        .flatten()
                        .filter(|_| !on_a_link())
                    else {
                        start += 1;
                        continue;
                    };
                    for (row, len) in &pieces {
                        let y = area.top() + *row as u16;
                        links.push(DrawnLink {
                            x,
                            y,
                            url: (*url).to_string(),
                            cells: (0..*len as u16)
                                .map(|offset| buffer[(x + offset, y)].clone())
                                .collect(),
                        });
                    }
                    start += pieces[0].1;
                }
            }
        }
        Self { area, links }
    }

    /// Forgets the written links after a resize, which clears the screen, so the next frame
    /// writes every link again.
    pub(super) fn forget_after(&mut self, event: &TerminalEvent) {
        if matches!(event, TerminalEvent::Resize(..)) {
            *self = Self::default();
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    #[cfg(test)]
    pub(super) fn links(&self) -> &[DrawnLink] {
        &self.links
    }

    /// Writes each link's cells again inside the OSC 8 sequence. The cursor is saved first and
    /// put back after, so a text box's cursor stays where ratatui left it.
    ///
    /// The terminal keeps the link on cells ratatui doesn't redraw, and ratatui skips a cell
    /// that looks the same as before. So the cells of every link in `previous` are written
    /// again from `buffer` outside the sequence first, which ends a link that moved or went
    /// away.
    pub(super) fn write<W: Write>(
        &self,
        previous: &DrawnLinks,
        buffer: &Buffer,
        backend: &mut CrosstermBackend<W>,
    ) -> io::Result<()> {
        if self.links.is_empty() && previous.links.is_empty() {
            return Ok(());
        }
        backend.write_all(b"\x1b7")?;
        let area = buffer.area;
        for link in &previous.links {
            if link.y < area.top() || link.y >= area.bottom() {
                continue;
            }
            let start = link.x.max(area.left());
            let end = link
                .x
                .saturating_add(link.cells.len() as u16)
                .min(area.right());
            backend.draw((start..end).map(|x| (x, link.y, &buffer[(x, link.y)])))?;
        }
        for link in &self.links {
            write!(backend, "\x1b]8;;{}\x1b\\", link.url)?;
            backend.draw(
                link.cells
                    .iter()
                    .enumerate()
                    .map(|(offset, cell)| (link.x + offset as u16, link.y, cell)),
            )?;
            backend.write_all(b"\x1b]8;;\x1b\\")?;
        }
        backend.write_all(b"\x1b8")?;
        Write::flush(backend)
    }
}

/// The rows `chars` takes when drawn from column `x` of row `y`: each row's index and how many
/// letters it holds. A row that doesn't hold the rest must be followed by a blank cell or the
/// edge, and the rest goes on in the same column of the next row. `apart` also asks for a blank
/// cell or the edge before each row and after the end.
fn place(
    rows: &[Vec<&str>],
    x: usize,
    y: usize,
    chars: &[char],
    apart: bool,
) -> Option<Vec<(usize, usize)>> {
    let blank_at = |row: &Vec<&str>, column: usize| {
        row.get(column)
            .is_none_or(|symbol: &&str| symbol.trim().is_empty())
    };
    let mut pieces = Vec::new();
    let mut rest = chars;
    for (index, row) in rows.iter().enumerate().skip(y) {
        if apart && x > 0 && !blank_at(row, x - 1) {
            return None;
        }
        let held = rest
            .iter()
            .zip(row.iter().skip(x))
            .take_while(|(letter, symbol)| {
                let mut symbol = symbol.chars();
                symbol.next() == Some(**letter) && symbol.next().is_none()
            })
            .count();
        if held == 0 {
            return None;
        }
        let end = held == rest.len();
        if (!end || apart) && !blank_at(row, x + held) {
            return None;
        }
        pieces.push((index, held));
        if end {
            return Some(pieces);
        }
        rest = &rest[held..];
    }
    None
}

#[cfg(test)]
mod tests {
    use ratatui::backend::CrosstermBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};

    use super::DrawnLinks;
    use crate::event::TerminalEvent;

    fn buffer_with(lines: &[&str]) -> Buffer {
        let width = lines.iter().map(|line| line.len()).max().unwrap_or(0) as u16;
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, lines.len() as u16));
        for (y, line) in lines.iter().enumerate() {
            buffer.set_string(0, y as u16, line, Style::default().fg(Color::Blue));
        }
        buffer
    }

    #[test]
    fn finds_each_link_drawn_whole_on_a_row() {
        let buffer = buffer_with(&[
            "How to make one: lemonup.org/github-token. Press enter",
            "Patreon supporters: addons.wago.io/patreon",
            "cut at the edge: lemonup.org/github-to",
        ]);
        let found = DrawnLinks::find(&buffer, &[]);
        let places: Vec<_> = found
            .links()
            .iter()
            .map(|link| (link.x, link.y, link.url.as_str()))
            .collect();
        assert_eq!(
            places,
            vec![
                (17, 0, "https://lemonup.org/github-token"),
                (20, 1, "https://addons.wago.io/patreon"),
            ]
        );
        assert!(DrawnLinks::find(&buffer_with(&["no links here"]), &[]).is_empty());
    }

    fn web_page(text: &str, url: &str) -> Vec<(String, String)> {
        vec![(text.to_string(), url.to_string())]
    }

    #[test]
    fn finds_a_link_given_by_the_screen() {
        let buffer = buffer_with(&["Web page        github.com/owner/repo"]);
        let extra = web_page("github.com/owner/repo", "https://github.com/owner/repo");
        let found = DrawnLinks::find(&buffer, &extra);
        let places: Vec<_> = found
            .links()
            .iter()
            .map(|link| (link.x, link.y, link.url.as_str()))
            .collect();
        assert_eq!(places, vec![(16, 0, "https://github.com/owner/repo")]);
        assert!(DrawnLinks::find(&buffer, &[]).is_empty());
    }

    #[test]
    fn finds_a_link_wrapped_over_rows_in_one_column() {
        let buffer = buffer_with(&[
            " Web page              ",
            "   addons.wago.io/     ",
            "   addons/VBNBxKx5     ",
            "   x                   ",
        ]);
        let url = "https://addons.wago.io/addons/VBNBxKx5";
        let found = DrawnLinks::find(&buffer, &web_page("addons.wago.io/addons/VBNBxKx5", url));
        let places: Vec<_> = found
            .links()
            .iter()
            .map(|link| (link.x, link.y, link.url.as_str(), link.cells.len()))
            .collect();
        assert_eq!(places, vec![(3, 1, url, 15), (3, 2, url, 15)]);
    }

    #[test]
    fn a_wrapped_link_needs_every_row_in_place() {
        let text = "addons.wago.io/addons/VBNBxKx5";
        let url = "https://addons.wago.io/addons/VBNBxKx5";
        for lines in [
            // The rest starts in another column.
            &["   addons.wago.io/ ", "    addons/VBNBxKx5"][..],
            // The first row goes on with other text.
            &["   addons.wago.io/x", "   addons/VBNBxKx5 "][..],
            // The rest is cut off.
            &["   addons.wago.io/ ", "   addons/VBN      "][..],
            // Only the end, without its start.
            &["   something else  ", "   addons/VBNBxKx5 "][..],
        ] {
            let found = DrawnLinks::find(&buffer_with(lines), &web_page(text, url));
            assert!(found.is_empty(), "{lines:?}: {:?}", found.links());
        }
    }

    #[test]
    fn a_link_given_by_the_screen_stands_apart_from_other_text() {
        let up = web_page("Up", "https://up.example/");
        let buffer = buffer_with(&["Update ready  xUp  Up:", "   Up"]);
        let places: Vec<_> = DrawnLinks::find(&buffer, &up)
            .links()
            .iter()
            .map(|link| (link.x, link.y))
            .collect();
        assert_eq!(places, vec![(3, 1)]);

        // One link per place, even when a given text is a fixed one's.
        let same = web_page(
            "lemonup.org/github-token",
            "http://lemonup.org/github-token",
        );
        let buffer = buffer_with(&["see lemonup.org/github-token"]);
        assert_eq!(DrawnLinks::find(&buffer, &same).links().len(), 1);

        // A fixed link's text can start a given one: the given one wins.
        let longer = web_page(
            "addons.wago.io/patreon-plus",
            "https://addons.wago.io/patreon-plus",
        );
        let buffer = buffer_with(&["   addons.wago.io/patreon-plus"]);
        let urls: Vec<_> = DrawnLinks::find(&buffer, &longer)
            .links()
            .iter()
            .map(|link| (link.x, link.url.clone()))
            .collect();
        assert_eq!(
            urls,
            vec![(3, "https://addons.wago.io/patreon-plus".to_string())]
        );

        // Each row of a wrapped one too.
        let wrapped = web_page("addons.wago.io/addons/x", "https://addons.wago.io/addons/x");
        let buffer = buffer_with(&["   addons.wago.io/ ", "  :addons/x        "]);
        assert!(DrawnLinks::find(&buffer, &wrapped).is_empty());
    }

    #[test]
    fn only_web_addresses_are_written_as_links() {
        assert_eq!(
            super::link_address(" https://github.com/owner/repo "),
            Some("https://github.com/owner/repo".to_string())
        );
        assert_eq!(
            super::link_address("http://tukui.org/elvui"),
            Some("http://tukui.org/elvui".to_string())
        );
        // Escape and control characters never reach the terminal.
        let escaped = super::link_address("https://example.com/a\x1b]8;;evil\x07b")
            .expect("still an address");
        assert!(
            escaped.chars().all(|letter| letter.is_ascii_graphic()),
            "{escaped:?}"
        );
        assert!(
            !escaped.contains('\x1b') && !escaped.contains('\x07'),
            "{escaped:?}"
        );
        let wide = super::link_address("https://example.com/caf\u{e9}").expect("an address");
        assert_eq!(wide, "https://example.com/caf%C3%A9");
        for url in [
            "",
            "github.com/owner/repo",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://example.com/addon",
            "https://exa\u{1b}mple.com/",
            "https://github.com@example.com/owner/repo",
            "https://user:secret@example.com/",
            // A one-word host would be a link text that matches words on screen.
            "https://Up",
            &format!("https://example.com/{}", "a".repeat(2048)),
            "https://.",
            "https://www./",
            "https://example..com/",
            // IP addresses, which a short label like "1.2" would be on screen.
            "https://1.2",
            "http://0x7f.1/",
            "https://[::1]/",
        ] {
            assert_eq!(super::link_address(url), None, "{url:?}");
        }
    }

    #[test]
    fn writes_the_link_text_inside_osc_8_and_puts_the_cursor_back() {
        let buffer = buffer_with(&["see lemonup.org/github-token"]);
        let mut out = Vec::new();
        DrawnLinks::find(&buffer, &[])
            .write(
                &DrawnLinks::default(),
                &buffer,
                &mut CrosstermBackend::new(&mut out),
            )
            .expect("write");
        let written = String::from_utf8(out).expect("utf8");
        assert!(written.starts_with("\x1b7\x1b]8;;https://lemonup.org/github-token\x1b\\"));
        assert!(written.ends_with("\x1b]8;;\x1b\\\x1b8"));
        let plain: String = strip_escapes(&written);
        assert!(plain.contains("lemonup.org/github-token"), "{plain:?}");
        assert!(!plain.contains("see"), "{plain:?}");
    }

    #[test]
    fn nothing_is_written_without_a_link() {
        let mut out = Vec::new();
        let buffer = buffer_with(&["plain text"]);
        DrawnLinks::find(&buffer, &[])
            .write(
                &DrawnLinks::default(),
                &buffer,
                &mut CrosstermBackend::new(&mut out),
            )
            .expect("write");
        assert!(out.is_empty());
    }

    #[test]
    fn a_link_that_goes_away_is_written_again_without_osc_8() {
        let before = buffer_with(&["see lemonup.org/github-token"]);
        let written = DrawnLinks::find(&before, &[]);
        // Same style everywhere, as under NO_COLOR: ratatui would skip the unchanged cells.
        let after = buffer_with(&["see lemonup.org/github-tokXX"]);
        let found = DrawnLinks::find(&after, &[]);
        assert!(found.is_empty());
        let mut out = Vec::new();
        found
            .write(&written, &after, &mut CrosstermBackend::new(&mut out))
            .expect("write");
        let text = String::from_utf8(out).expect("utf8");
        assert!(!text.contains("\x1b]8;;https"), "{text:?}");
        assert_eq!(strip_escapes(&text), "lemonup.org/github-tokXX");
        assert!(
            text.starts_with("\x1b7") && text.ends_with("\x1b8"),
            "{text:?}"
        );
    }

    #[test]
    fn a_link_that_moves_is_closed_at_the_old_place_and_opened_at_the_new_one() {
        let before = buffer_with(&["lemonup.org/github-token", "                        "]);
        let written = DrawnLinks::find(&before, &[]);
        let after = buffer_with(&["                        ", "lemonup.org/github-token"]);
        let found = DrawnLinks::find(&after, &[]);
        let mut out = Vec::new();
        found
            .write(&written, &after, &mut CrosstermBackend::new(&mut out))
            .expect("write");
        let text = String::from_utf8(out).expect("utf8");
        let open = text
            .find("\x1b]8;;https://lemonup.org/github-token\x1b\\")
            .expect("opened");
        // The old row is written first, outside the link, with what is there now.
        let (old_row, new_link) = text.split_at(open);
        assert!(!old_row.contains("\x1b]8;"), "{old_row:?}");
        assert_eq!(strip_escapes(old_row).trim(), "");
        assert_eq!(strip_escapes(old_row).len(), 24);
        assert!(new_link.ends_with("\x1b]8;;\x1b\\\x1b8"), "{new_link:?}");
        assert_eq!(strip_escapes(new_link), "lemonup.org/github-token");
    }

    #[test]
    fn an_old_link_outside_a_smaller_screen_is_cut_to_fit() {
        let before = buffer_with(&["      lemonup.org/github-token"]);
        let written = DrawnLinks::find(&before, &[]);
        let after = buffer_with(&["small"]);
        let mut out = Vec::new();
        DrawnLinks::find(&after, &[])
            .write(&written, &after, &mut CrosstermBackend::new(&mut out))
            .expect("write");
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(strip_escapes(&text), "");
    }

    #[test]
    fn a_resize_forgets_the_written_links() {
        let mut written = DrawnLinks::find(&buffer_with(&["lemonup.org/github-token"]), &[]);
        written.forget_after(&TerminalEvent::Tick);
        assert!(!written.is_empty());
        written.forget_after(&TerminalEvent::Resize(80, 24));
        assert_eq!(written, DrawnLinks::default());
    }

    /// The printed text without escape sequences (CSI, OSC, and the two-byte cursor ones).
    fn strip_escapes(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(letter) = chars.next() {
            if letter != '\x1b' {
                out.push(letter);
                continue;
            }
            match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}
