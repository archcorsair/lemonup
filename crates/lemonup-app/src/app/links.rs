//! Links on screen that open with a click (OSC 8), where the terminal has links.
//!
//! A ratatui cell can't hold the escape sequence: its width would count the address. So the
//! frame is drawn as usual, then each link's cells are written again, inside the sequence,
//! straight to the terminal. Cells ratatui doesn't redraw keep the link; when they change,
//! the link is written again.

use std::io::{self, Write};

use ratatui::backend::{Backend as _, CrosstermBackend};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

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
    pub(super) fn write<W: Write>(&self, backend: &mut CrosstermBackend<W>) -> io::Result<()> {
        if self.links.is_empty() {
            return Ok(());
        }
        backend.write_all(b"\x1b7")?;
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
            .write(&mut CrosstermBackend::new(&mut out))
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
        DrawnLinks::find(&buffer_with(&["plain text"]))
            .write(&mut CrosstermBackend::new(&mut out))
            .expect("write");
        assert!(out.is_empty());
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
