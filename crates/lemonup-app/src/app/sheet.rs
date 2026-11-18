//! The sheet: the one container every overlay draws in.
//!
//! A sheet has its title in the top border and a body that scrolls. Its keys
//! show in the frame's key line, not inside the sheet. It is as tall as its
//! content, up to the window minus the status line and the key line; past
//! that the body scrolls and says "↓ N more". Below 100 x 30 a sheet fills
//! the whole body. The list behind a sheet is dimmed.
//!
//! A sheet whose content changes while it is open reserves the tallest body
//! it can need (`SheetBody::reserve`), so it keeps its size and place: it
//! sits where a body that tall would, and shorter states are padded at the
//! bottom. A sheet with several layouts picks one with `SheetRoom::pick`.
//! The pick and the reserve hold while the sheet stays open at one window
//! size, so the layout never flips between states.

use std::cell::Cell;

use ratatui::buffer::Buffer;
use ratatui::widgets::{StatefulWidget, Widget};

use super::*;
use crate::theme::ColorRoles;

/// A window narrower than this shows sheets full screen.
pub(super) const SHEET_FULL_BELOW_WIDTH: u16 = 100;
/// A window shorter than this shows sheets full screen.
pub(super) const SHEET_FULL_BELOW_HEIGHT: u16 = 30;
/// How much of the list's color shows through behind a sheet.
const DIM_AMOUNT: f32 = 0.34;

/// Widths from the design, for windows wide enough to show a sheet over the
/// list. Narrower windows clamp them.
pub(super) fn sheet_width(kind: OverlayKind) -> u16 {
    match kind {
        OverlayKind::Inspect => 76,
        OverlayKind::Install | OverlayKind::Search | OverlayKind::Find => 104,
        OverlayKind::Config => 100,
        OverlayKind::Backup | OverlayKind::TocEdit => 84,
        OverlayKind::Help => 108,
        OverlayKind::Update | OverlayKind::Confirm | OverlayKind::Games => 76,
    }
}

/// Draws one widget part of a sheet body into its rows.
pub(super) type DrawPart<'a> = Box<dyn FnOnce(Rect, &mut Buffer) + 'a>;

/// One piece of a sheet body, top to bottom.
pub(super) enum SheetPart<'a> {
    /// Text that wraps at the body width.
    Text {
        lines: Vec<Line<'static>>,
        alignment: Alignment,
    },
    /// Blank rows.
    Gap(u16),
    /// Anything else, with a fixed height.
    Widget { height: u16, draw: DrawPart<'a> },
}

/// The content of a sheet, and the part that holds the cursor.
#[derive(Default)]
pub(super) struct SheetBody<'a> {
    parts: Vec<SheetPart<'a>>,
    focus: Option<usize>,
    /// Rows pinned under the body: they stay in place while it scrolls.
    footer: Vec<Line<'static>>,
    /// The footer sits on the body's last rows rather than right under the content.
    footer_at_bottom: bool,
    /// Lines in a column on the right of the body, and the columns it takes with its gap and
    /// left border: it holds still at the top while the body scrolls beside it.
    rail: Option<(u16, Vec<Line<'static>>)>,
    /// The tallest the body gets while the sheet is open, footer included.
    reserve: Option<u16>,
}

impl<'a> SheetBody<'a> {
    pub(super) fn new() -> Self {
        Self {
            parts: Vec::new(),
            focus: None,
            footer: Vec::new(),
            footer_at_bottom: false,
            rail: None,
            reserve: None,
        }
    }

    /// The most rows, footer included, this body takes in any state while
    /// the sheet is open. The sheet is placed for a body this tall, and a
    /// shorter one is padded at the bottom, so it keeps its size and place
    /// while its content changes.
    pub(super) fn reserve(&mut self, rows: u16) -> &mut Self {
        self.reserve = Some(rows);
        self
    }

    /// Text pinned to the bottom of the sheet, wrapped at the body width,
    /// such as a description of the row under the cursor.
    pub(super) fn footer(&mut self, lines: Vec<Line<'static>>) -> &mut Self {
        self.footer = lines;
        self
    }

    /// Puts the footer on the body's last rows, however short the content is.
    pub(super) fn footer_at_bottom(&mut self) -> &mut Self {
        self.footer_at_bottom = true;
        self
    }

    /// A column `width` columns wide on the right of the body: a gap, a left border line, a
    /// space, then `lines`, which are not wrapped. It is only as tall as its lines and holds
    /// still while the rest of the body, drawn to its left, scrolls.
    pub(super) fn rail(&mut self, width: u16, lines: Vec<Line<'static>>) -> &mut Self {
        self.rail = Some((width, lines));
        self
    }

    pub(super) fn text(&mut self, lines: Vec<Line<'static>>) -> &mut Self {
        self.parts.push(SheetPart::Text {
            lines,
            alignment: Alignment::Left,
        });
        self
    }

    pub(super) fn gap(&mut self, rows: u16) -> &mut Self {
        self.parts.push(SheetPart::Gap(rows));
        self
    }

    /// Drops a gap the body ends with, so the footer alone sets the space above it.
    pub(super) fn trim_trailing_gap(&mut self) -> &mut Self {
        if matches!(self.parts.last(), Some(SheetPart::Gap(_))) {
            self.parts.pop();
        }
        self
    }

    pub(super) fn widget(
        &mut self,
        height: u16,
        draw: impl FnOnce(Rect, &mut Buffer) + 'a,
    ) -> &mut Self {
        self.parts.push(SheetPart::Widget {
            height,
            draw: Box::new(draw),
        });
        self
    }

    /// The part added last holds the cursor: the sheet scrolls to keep it in
    /// view whenever it moves.
    pub(super) fn focus_last(&mut self) -> &mut Self {
        self.focus = self.parts.len().checked_sub(1);
        self
    }

    /// Rows each part takes at `width`.
    fn part_heights(&self, width: u16) -> Vec<u16> {
        self.parts
            .iter()
            .map(|part| match part {
                SheetPart::Text { lines, .. } => text_height(lines, width),
                SheetPart::Gap(rows) => *rows,
                SheetPart::Widget { height, .. } => *height,
            })
            .collect()
    }
}

/// Rows `lines` take once wrapped at `width`.
pub(super) fn text_height(lines: &[Line<'static>], width: u16) -> u16 {
    u16::try_from(wrap_lines(lines, width).len()).unwrap_or(u16::MAX)
}

/// Word wraps `lines` at `width` columns. The sheet measures and draws the
/// same wrapped rows, so its height is always exact. A word longer than the
/// width is broken mid-word; the space a row breaks at is dropped.
pub(super) fn wrap_lines(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    wrap_lines_hanging(lines, width, 0)
}

/// `wrap_lines`, with every row after a line's first indented by `indent`
/// columns, so a long value lines up under the text after its label.
pub(super) fn wrap_lines_hanging(
    lines: &[Line<'static>],
    width: u16,
    indent: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    // A hanging indent only when continuation rows keep some room.
    let indent = if usize::from(indent) < width / 2 {
        usize::from(indent)
    } else {
        0
    };
    let mut out = Vec::new();
    for line in lines {
        let cells: Vec<(String, Style, usize)> = line
            .spans
            .iter()
            .flat_map(|span| {
                span.styled_graphemes(line.style).map(|grapheme| {
                    let symbol = grapheme.symbol.to_string();
                    let cells = Span::raw(symbol.as_str()).width();
                    (symbol, grapheme.style, cells)
                })
            })
            .collect();
        let mut rows: Vec<Vec<(String, Style, usize)>> = vec![Vec::new()];
        let mut used = 0usize;
        for cell in cells {
            let room = if rows.len() == 1 {
                width
            } else {
                width - indent
            };
            if used + cell.2 > room && !rows.last().expect("a row").is_empty() {
                let row = rows.last_mut().expect("a row");
                // Break after the last space, if the row has one.
                let carry = match row.iter().rposition(|(symbol, _, _)| symbol == " ") {
                    Some(space) if cell.0 != " " => {
                        let tail = row.split_off(space + 1);
                        row.pop();
                        tail
                    }
                    _ => Vec::new(),
                };
                used = carry.iter().map(|cell| cell.2).sum();
                rows.push(carry);
                if cell.0 == " " && rows.last().expect("a row").is_empty() {
                    continue;
                }
            }
            used += cell.2;
            rows.last_mut().expect("a row").push(cell);
        }
        for (index, row) in rows.into_iter().enumerate() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if index > 0 && indent > 0 {
                spans.push(Span::raw(" ".repeat(indent)));
            }
            for (symbol, style, _) in row {
                match spans.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push_str(&symbol),
                    _ => spans.push(Span::styled(symbol, style)),
                }
            }
            let mut wrapped = Line::from(spans);
            wrapped.alignment = line.alignment;
            out.push(wrapped);
        }
    }
    out
}

/// The room a sheet body has, for the code that builds it.
pub(super) struct SheetRoom {
    /// Width of the text inside.
    pub(super) width: u16,
    /// The most body rows the sheet can show at once.
    pub(super) height: u16,
    /// Of those, the rows a floating sheet has under the header rows.
    pub(super) below_header: u16,
    /// Fills the body instead of floating over the list.
    full: bool,
    /// What the sheet decided when it opened at this window size.
    kept: Option<SheetKept>,
    /// The layout `pick` chose in this draw.
    picked: Cell<Option<usize>>,
}

impl SheetRoom {
    /// Picks one of a sheet's layouts, given the body rows each one
    /// reserves, roomiest first: the first that fits under the header, so
    /// the sheet only grows over the header when it has to, then the first
    /// that fits at all, else the last. The pick holds while the sheet stays
    /// open at this window size.
    pub(super) fn pick(&self, layouts: &[u16]) -> usize {
        let index = self
            .kept
            .and_then(|kept| kept.layout)
            .filter(|index| *index < layouts.len())
            .unwrap_or_else(|| {
                layouts
                    .iter()
                    .position(|rows| *rows <= self.below_header)
                    .or_else(|| layouts.iter().position(|rows| *rows <= self.height))
                    .unwrap_or(layouts.len().saturating_sub(1))
            });
        self.picked.set(Some(index));
        index
    }

    /// The rows the sheet reserved when it opened at this window size, so a
    /// reserve that takes work to measure is only measured once.
    pub(super) fn reserved(&self) -> Option<u16> {
        self.kept.and_then(|kept| kept.reserve)
    }

    /// The most body rows that keep the header in view, for a sheet that
    /// would rather scroll than cover it: all of them on a full screen
    /// sheet, which covers the header anyway.
    pub(super) fn clear_of_header(&self) -> u16 {
        if self.full {
            self.height
        } else {
            self.below_header.max(1)
        }
    }
}

/// What an open sheet decided at its first draw at one window size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SheetKept {
    /// The window, width and height.
    window: (u16, u16),
    /// The layout `SheetRoom::pick` chose.
    layout: Option<usize>,
    /// The body rows reserved.
    reserve: Option<u16>,
}

/// What the last drawn sheet looked like, so keys can scroll it and the next
/// draw can keep the cursor in view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SheetScroll {
    /// The overlay this belongs to; another overlay starts at the top.
    pub(super) kind: Option<OverlayKind>,
    /// First body row on screen.
    pub(super) offset: u16,
    /// Body rows holding the cursor at the last draw.
    pub(super) focus: Option<(u16, u16)>,
    /// The largest useful offset; zero when everything fits.
    pub(super) max_offset: u16,
    /// Body rows on screen.
    pub(super) viewport: u16,
    /// The layout and reserve the sheet keeps while it is open.
    pub(super) kept: Option<SheetKept>,
}

impl SheetScroll {
    /// True once a sheet of this kind has been drawn and its body is taller
    /// than the window.
    pub(super) fn overflows(self, kind: Option<OverlayKind>) -> bool {
        self.kind.is_some() && self.kind == kind && self.max_offset > 0
    }

    /// The offset after a scroll key. Before the first draw only the top is
    /// enforced; the draw clamps the rest.
    pub(super) fn moved(self, movement: SheetMove, kind: Option<OverlayKind>) -> u16 {
        let drawn = self.kind.is_some() && self.kind == kind;
        let current = if drawn {
            self.offset.min(self.max_offset)
        } else {
            0
        };
        let max = if drawn { self.max_offset } else { u16::MAX };
        // A page keeps the arrow lines in mind, so no row is ever skipped.
        let page = if drawn {
            self.viewport.saturating_sub(2).max(1)
        } else {
            10
        };
        let target = match movement {
            SheetMove::LineUp => i32::from(current) - 1,
            SheetMove::LineDown => i32::from(current) + 1,
            SheetMove::PageUp => i32::from(current) - i32::from(page),
            SheetMove::PageDown => i32::from(current) + i32::from(page),
            SheetMove::Top => 0,
            SheetMove::Bottom => i32::from(max),
        };
        target.clamp(0, i32::from(max)) as u16
    }
}

/// A key that scrolls a sheet body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SheetMove {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// The title row of a sheet.
pub(super) struct SheetChrome {
    pub(super) title: String,
    /// A muted note before "esc close", such as "changes save right away".
    pub(super) note: Option<&'static str>,
    /// A delete or another step that cannot be undone: the outline is red.
    pub(super) danger: bool,
}

/// Where a sheet goes, before its content is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SheetFrame {
    /// Fills the body instead of floating over the list.
    pub(super) full: bool,
    /// The space a sheet may use.
    region: Rect,
    /// Header rows at the top of `region`.
    header: u16,
    /// Outer width.
    width: u16,
    /// Width of the text inside.
    pub(super) body_width: u16,
    /// The most body rows the sheet can show at once.
    pub(super) max_body_height: u16,
}

/// Rows a floating sheet uses around its body: the two borders and a blank
/// row above and below.
const FLOAT_CHROME_ROWS: u16 = 4;
/// Columns a floating sheet uses around its body: the borders and two
/// columns of padding on each side.
const FLOAT_CHROME_COLUMNS: u16 = 6;
/// Rows a full screen sheet uses above its body: the title, a rule and a
/// blank row. The frame's own rule sits below it.
const FULL_CHROME_ROWS: u16 = 3;
/// Columns of padding on each side of a full screen sheet.
const FULL_PADDING: u16 = 1;

impl SheetFrame {
    /// `window` is the whole terminal; `region` is everything above the
    /// status line and the key line.
    /// `header` is how many rows at the top of `region` the header uses: a
    /// floating sheet centers below them and only grows over them when its
    /// content needs the room.
    pub(super) fn new(window: Rect, region: Rect, header: u16, preferred_width: u16) -> Self {
        let full = window.width < SHEET_FULL_BELOW_WIDTH || window.height < SHEET_FULL_BELOW_HEIGHT;
        if full {
            // The frame's bottom rule stays in place under a full sheet.
            let region = Rect {
                height: region.height.saturating_sub(1),
                ..region
            };
            Self {
                full,
                region,
                header,
                width: region.width,
                body_width: region.width.saturating_sub(FULL_PADDING * 2),
                max_body_height: region.height.saturating_sub(FULL_CHROME_ROWS).max(1),
            }
        } else {
            let width = preferred_width.min(region.width);
            Self {
                full,
                region,
                header,
                width,
                body_width: width.saturating_sub(FLOAT_CHROME_COLUMNS),
                max_body_height: region.height.saturating_sub(FLOAT_CHROME_ROWS).max(1),
            }
        }
    }

    /// Body rows that fit under the header rows.
    pub(super) fn below_header(self) -> u16 {
        self.max_body_height.saturating_sub(self.header)
    }

    /// The sheet's outline and body for a body `content` rows tall.
    pub(super) fn place(self, content: u16) -> SheetGeometry {
        self.place_reserved(content, content)
    }

    /// The sheet's outline and body for a body `content` rows tall that
    /// reserves `reserve` rows: it sits where a body `reserve` rows tall
    /// would, with the rows the content leaves blank at the bottom. Taller
    /// content grows the sheet down, never moving its top.
    pub(super) fn place_reserved(self, content: u16, reserve: u16) -> SheetGeometry {
        let reserved = reserve.clamp(1, self.max_body_height);
        let viewport = content.max(reserve).clamp(1, self.max_body_height);
        if self.full {
            let outer = self.region;
            let body = Rect::new(
                outer.x + FULL_PADDING,
                outer.y + FULL_CHROME_ROWS,
                self.body_width,
                viewport.min(outer.height.saturating_sub(FULL_CHROME_ROWS)),
            );
            return SheetGeometry {
                outer,
                body,
                full: true,
            };
        }
        let height = (reserved + FLOAT_CHROME_ROWS).min(self.region.height);
        let x = self.region.x + (self.region.width.saturating_sub(self.width)) / 2;
        // Centered between the header and the status line; taller content
        // grows upward over the header rows, never over the status line.
        let header = self.header.min(self.region.height);
        let below_header = self.region.height - header;
        let y = if height <= below_header {
            self.region.y + header + (below_header - height) / 2
        } else {
            self.region.bottom().saturating_sub(height)
        };
        let height = (viewport + FLOAT_CHROME_ROWS).min(self.region.bottom() - y);
        let outer = Rect::new(x, y, self.width, height);
        let body = Rect::new(
            outer.x + FLOAT_CHROME_COLUMNS / 2,
            outer.y + FLOAT_CHROME_ROWS / 2,
            self.body_width,
            height.saturating_sub(FLOAT_CHROME_ROWS),
        );
        SheetGeometry {
            outer,
            body,
            full: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SheetGeometry {
    pub(super) outer: Rect,
    pub(super) body: Rect,
    pub(super) full: bool,
}

/// The scroll offset that keeps rows `focus` on screen, moving as little as
/// possible from `offset`. Row `offset` is the first body row on screen; an
/// arrow line replaces the first or last row when more is hidden there.
pub(super) fn offset_for_focus(offset: u16, focus: (u16, u16), total: u16, viewport: u16) -> u16 {
    let max = total.saturating_sub(viewport);
    let (start, end) = focus;
    let mut offset = offset.min(max);
    // First and one past the last content row on screen at `offset`.
    let visible = |offset: u16| {
        let top = if offset > 0 { offset + 1 } else { offset };
        let bottom = if offset < max {
            offset + viewport - 1
        } else {
            offset + viewport
        };
        (top, bottom)
    };
    let (top, bottom) = visible(offset);
    if start < top {
        offset = start.saturating_sub(1);
        if start == 0 {
            offset = 0;
        }
    } else if end > bottom {
        offset = (end + 1).saturating_sub(viewport).min(max);
        if visible(offset).1 < end {
            offset = max;
        }
    }
    offset.min(max)
}

impl App {
    /// Draws `kind` as a sheet inside `region`, the space above the status
    /// line and the key line. `build` gets the room the body has and returns
    /// the content.
    pub(super) fn render_sheet<'a>(
        &self,
        frame: &mut Frame<'_>,
        region: Rect,
        header: u16,
        kind: OverlayKind,
        chrome: SheetChrome,
        build: impl FnOnce(&SheetRoom) -> SheetBody<'a>,
    ) {
        let roles = self.ui_theme.roles;
        let window = frame.area();
        let sheet_frame = SheetFrame::new(window, region, header, sheet_width(kind));
        let previous = self.sheet_scroll.get();
        let size = (window.width, window.height);
        // What the sheet decided when it opened at this size, if it did.
        let kept = previous
            .kept
            .filter(|kept| previous.kind == Some(kind) && kept.window == size);
        let room = SheetRoom {
            width: sheet_frame.body_width,
            height: sheet_frame.max_body_height,
            below_header: sheet_frame.below_header(),
            full: sheet_frame.full,
            kept,
            picked: Cell::new(None),
        };
        let body = build(&room);
        let reserve = kept.and_then(|kept| kept.reserve).or(body.reserve);
        let kept = SheetKept {
            window: size,
            layout: kept.and_then(|kept| kept.layout).or(room.picked.get()),
            reserve,
        };
        // The rail takes its columns on the right; the parts get the rest.
        let rail_width = body
            .rail
            .as_ref()
            .map_or(0, |(width, _)| (*width).min(sheet_frame.body_width));
        let parts_width = sheet_frame.body_width - rail_width;
        let rail_rows = body.rail.as_ref().map_or(0, |(_, lines)| {
            u16::try_from(lines.len()).unwrap_or(u16::MAX)
        });
        let heights = body.part_heights(parts_width);
        let total = heights.iter().copied().fold(0u16, u16::saturating_add);
        let footer = wrap_lines(&body.footer, sheet_frame.body_width);
        let footer_rows = u16::try_from(footer.len()).unwrap_or(u16::MAX);
        let focus = body.focus.map(|index| {
            let start = heights[..index]
                .iter()
                .copied()
                .fold(0u16, u16::saturating_add);
            (start, start.saturating_add(heights[index].max(1)))
        });
        let content = total.max(rail_rows).saturating_add(footer_rows);
        let geometry = match reserve {
            Some(reserve) => sheet_frame.place_reserved(content, reserve),
            None => sheet_frame.place(content),
        };

        if !geometry.full {
            dim_area(frame.buffer_mut(), region, roles);
        }
        frame.render_widget(Clear, geometry.outer);
        frame.render_widget(
            Block::default().style(Style::default().bg(roles.sheet).fg(roles.text)),
            geometry.outer,
        );
        self.render_sheet_chrome(frame, geometry, &chrome);

        // Scroll: a new overlay starts at the top, and a moved cursor is
        // brought back into view.
        // The footer keeps its rows; the body scrolls in what is left, and
        // always keeps at least one. The footer sits right under the body;
        // rows a reserve leaves stay blank below it.
        let footer_rows = footer_rows.min(geometry.body.height.saturating_sub(1));
        let viewport = (geometry.body.height - footer_rows).min(total.max(1));
        let mut offset = if previous.kind == Some(kind) {
            previous.offset
        } else {
            0
        };
        let max_offset = total.saturating_sub(viewport);
        offset = offset.min(max_offset);
        if let Some(focus) = focus
            && (previous.kind != Some(kind) || previous.focus != Some(focus))
        {
            offset = offset_for_focus(offset, focus, total, viewport);
        }
        self.sheet_scroll.set(SheetScroll {
            kind: Some(kind),
            offset,
            focus,
            max_offset,
            viewport,
            kept: Some(kept),
        });

        // Draw the whole body off screen, then copy the rows on screen.
        let canvas_area = Rect::new(0, 0, parts_width, total.max(1));
        let mut canvas = Buffer::empty(canvas_area);
        canvas.set_style(canvas_area, Style::default().bg(roles.sheet).fg(roles.text));
        let mut y = 0u16;
        for (part, height) in body.parts.into_iter().zip(heights) {
            let area = Rect::new(0, y, parts_width, height);
            y = y.saturating_add(height);
            if height == 0 {
                continue;
            }
            match part {
                SheetPart::Text { lines, alignment } => {
                    Paragraph::new(wrap_lines(&lines, parts_width))
                        .alignment(alignment)
                        .render(area, &mut canvas)
                }
                SheetPart::Gap(_) => {}
                SheetPart::Widget { draw, .. } => draw(area, &mut canvas),
            }
        }

        let hidden_above = offset;
        let hidden_below = total.saturating_sub(offset.saturating_add(viewport));
        let show_up = hidden_above > 0 && viewport > 2;
        let show_down = hidden_below > 0 && viewport > 1;
        let buffer = frame.buffer_mut();
        for row in 0..viewport {
            let source_y = offset + row;
            if source_y >= total {
                break;
            }
            let target_y = geometry.body.y + row;
            for column in 0..parts_width {
                let target = (geometry.body.x + column, target_y);
                if buffer.area.contains(target.into()) {
                    buffer[target] = canvas[(column, source_y)].clone();
                }
            }
        }
        if footer_rows > 0 {
            let top = if body.footer_at_bottom {
                geometry.body.height - footer_rows
            } else {
                viewport
            };
            let area = Rect::new(
                geometry.body.x,
                geometry.body.y + top,
                geometry.body.width,
                footer_rows,
            );
            Clear.render(area, buffer);
            buffer.set_style(area, Style::default().bg(roles.sheet).fg(roles.text));
            Paragraph::new(footer[..usize::from(footer_rows)].to_vec()).render(area, buffer);
        }
        if let Some((_, lines)) = body.rail
            && rail_width > 1
        {
            let rows = rail_rows.min(geometry.body.height.saturating_sub(footer_rows));
            let area = Rect::new(
                geometry.body.x + parts_width + 1,
                geometry.body.y,
                rail_width - 1,
                rows,
            );
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::LEFT)
                        .border_style(Style::default().fg(roles.border))
                        .padding(ratatui::widgets::Padding::left(1)),
                )
                .render(area, buffer);
        }
        let arrow_style = Style::default().fg(roles.muted).bg(roles.sheet);
        if show_up {
            let line = Line::from(Span::styled(
                format!("↑ {} more", hidden_above + 1),
                arrow_style,
            ));
            let area = Rect::new(geometry.body.x, geometry.body.y, parts_width, 1);
            Clear.render(area, buffer);
            buffer.set_style(area, arrow_style);
            line.render(area, buffer);
        }
        if show_down {
            let line = Line::from(Span::styled(
                format!("↓ {} more", hidden_below + 1),
                arrow_style,
            ));
            let area = Rect::new(
                geometry.body.x,
                geometry.body.y + viewport - 1,
                parts_width,
                1,
            );
            Clear.render(area, buffer);
            buffer.set_style(area, arrow_style);
            line.render(area, buffer);
        }
    }

    /// The title in the top border (floating) or on the first row (full).
    fn render_sheet_chrome(
        &self,
        frame: &mut Frame<'_>,
        geometry: SheetGeometry,
        chrome: &SheetChrome,
    ) {
        let roles = self.ui_theme.roles;
        let outline = if chrome.danger {
            roles.red
        } else {
            roles.border
        };
        let title_style = Style::default()
            .fg(if chrome.danger { roles.red } else { roles.text })
            .add_modifier(Modifier::BOLD);
        let key_style = Style::default().fg(roles.blue).add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(roles.muted);
        let outer = geometry.outer;
        let width = usize::from(outer.width);

        if geometry.full {
            let title = truncate_text(&chrome.title, width.saturating_sub(13));
            let mut used = 1 + title.chars().count() + "esc close".len() + 1;
            let mut spans = vec![Span::raw(" "), Span::styled(title, title_style)];
            let note = chrome
                .note
                .filter(|note| used + note.chars().count() + 4 <= width);
            if let Some(note) = note {
                used += note.chars().count() + 3;
            }
            if used <= width {
                spans.push(Span::raw(" ".repeat(width - used)));
                if let Some(note) = note {
                    spans.push(Span::styled(format!("{note} · "), label_style));
                }
                spans.push(Span::styled("esc", key_style));
                spans.push(Span::styled(" close", label_style));
            }
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(outer.x, outer.y, outer.width, 1),
            );
            if outer.height > 1 {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "─".repeat(width),
                        Style::default().fg(roles.dim),
                    ))),
                    Rect::new(outer.x, outer.y + 1, outer.width, 1),
                );
            }
            return;
        }

        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_set(symbols::border::ROUNDED)
                .border_style(Style::default().fg(outline)),
            outer,
        );
        // "╭─ Title ───── esc close ─╮"
        let title = truncate_text(&chrome.title, width.saturating_sub(6));
        let left = 4 + title.chars().count();
        let note = chrome.note.filter(|note| {
            left + 1 + note.chars().count() + 3 + " esc close ─╮".chars().count() <= width
        });
        let right =
            " esc close ─╮".chars().count() + note.map_or(0, |note| note.chars().count() + 3);
        let border = Style::default().fg(outline);
        let mut spans = vec![
            Span::styled("╭─ ", border),
            Span::styled(title, title_style),
            Span::styled(" ", border),
        ];
        if left + 1 + right <= width {
            spans.push(Span::styled("─".repeat(width - left - right), border));
            spans.push(Span::styled(" ", border));
            if let Some(note) = note {
                spans.push(Span::styled(format!("{note} · "), label_style));
            }
            spans.push(Span::styled("esc", key_style));
            spans.push(Span::styled(" close", label_style));
            spans.push(Span::styled(" ─╮", border));
        } else {
            spans.push(Span::styled(
                "─".repeat(width.saturating_sub(left + 1)),
                border,
            ));
            spans.push(Span::styled("╮", border));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(outer.x, outer.y, outer.width, 1),
        );
    }
}

/// Fades everything in `area` towards the background, for the list behind a
/// sheet.
pub(super) fn dim_area(buffer: &mut Buffer, area: Rect, roles: ColorRoles) {
    let area = area.intersection(buffer.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buffer[(x, y)];
            cell.fg = dim_color(cell.fg, roles.background, roles.dim);
            if cell.bg != Color::Reset {
                cell.bg = dim_color(cell.bg, roles.background, roles.background);
            }
        }
    }
}

fn dim_color(color: Color, toward: Color, fallback: Color) -> Color {
    match (color, toward) {
        (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) => {
            let mix = |value: u8, base: u8| {
                (f32::from(base) + (f32::from(value) - f32::from(base)) * DIM_AMOUNT).round() as u8
            };
            Color::Rgb(mix(r, tr), mix(g, tg), mix(b, tb))
        }
        _ => fallback,
    }
}

/// Lets an overlay draw a stateful widget into a sheet body.
pub(super) fn render_stateful<W: StatefulWidget>(
    widget: W,
    area: Rect,
    buffer: &mut Buffer,
    state: &mut W::State,
) {
    StatefulWidget::render(widget, area, buffer, state);
}
