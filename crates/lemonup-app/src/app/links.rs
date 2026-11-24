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

/// One link on screen: where its text starts, the address, and its cells as drawn.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DrawnLink {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) url: &'static str,
    cells: Vec<Cell>,
}

/// The links in a drawn frame, at the size it was drawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct DrawnLinks {
    area: Rect,
    links: Vec<DrawnLink>,
}

impl DrawnLinks {
    /// Every place a link's text is drawn whole on one row of `buffer`.
    pub(super) fn find(buffer: &Buffer) -> Self {
        let area = buffer.area;
        let mut links = Vec::new();
        for y in area.top()..area.bottom() {
            let row: Vec<&str> = (area.left()..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            for (text, url) in LINKS {
                let chars: Vec<char> = text.chars().collect();
                let mut start = 0;
                while start + chars.len() <= row.len() {
                    let whole = chars.iter().enumerate().all(|(offset, letter)| {
                        let mut symbol = row[start + offset].chars();
                        symbol.next() == Some(*letter) && symbol.next().is_none()
                    });
                    if whole {
                        let x = area.left() + start as u16;
                        links.push(DrawnLink {
                            x,
                            y,
                            url,
                            cells: (0..chars.len() as u16)
                                .map(|offset| buffer[(x + offset, y)].clone())
                                .collect(),
                        });
                        start += chars.len();
                    } else {
                        start += 1;
                    }
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
        let found = DrawnLinks::find(&buffer);
        let places: Vec<_> = found
            .links()
            .iter()
            .map(|link| (link.x, link.y, link.url))
            .collect();
        assert_eq!(
            places,
            vec![
                (17, 0, "https://lemonup.org/github-token"),
                (20, 1, "https://addons.wago.io/patreon"),
            ]
        );
        assert!(DrawnLinks::find(&buffer_with(&["no links here"])).is_empty());
    }

    #[test]
    fn writes_the_link_text_inside_osc_8_and_puts_the_cursor_back() {
        let buffer = buffer_with(&["see lemonup.org/github-token"]);
        let mut out = Vec::new();
        DrawnLinks::find(&buffer)
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
        DrawnLinks::find(&buffer)
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
        let written = DrawnLinks::find(&before);
        // Same style everywhere, as under NO_COLOR: ratatui would skip the unchanged cells.
        let after = buffer_with(&["see lemonup.org/github-tokXX"]);
        let found = DrawnLinks::find(&after);
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
        let written = DrawnLinks::find(&before);
        let after = buffer_with(&["                        ", "lemonup.org/github-token"]);
        let found = DrawnLinks::find(&after);
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
        let written = DrawnLinks::find(&before);
        let after = buffer_with(&["small"]);
        let mut out = Vec::new();
        DrawnLinks::find(&after)
            .write(&written, &after, &mut CrosstermBackend::new(&mut out))
            .expect("write");
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(strip_escapes(&text), "");
    }

    #[test]
    fn a_resize_forgets_the_written_links() {
        let mut written = DrawnLinks::find(&buffer_with(&["lemonup.org/github-token"]));
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
