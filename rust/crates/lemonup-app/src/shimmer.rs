use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

fn elapsed_since_start() -> Duration {
    let start = PROCESS_START.get_or_init(Instant::now);
    start.elapsed()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShimmerConfig {
    pub(crate) padding: usize,
    pub(crate) sweep_seconds: f32,
    pub(crate) band_half_width: f32,
    pub(crate) peak: f32,
}

impl ShimmerConfig {
    pub(crate) const fn action() -> Self {
        Self {
            padding: 8,
            sweep_seconds: 1.5,
            band_half_width: 4.0,
            peak: 0.6,
        }
    }
}

pub(crate) fn shimmer_text_spans(
    text: &str,
    base: Color,
    highlight: Color,
    config: ShimmerConfig,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let intensities = shimmer_levels(
        chars.iter().filter(|ch| !ch.is_whitespace()).count(),
        config,
    );
    let mut spans = Vec::with_capacity(chars.len());
    let mut visible_index = 0usize;

    for ch in chars {
        if ch.is_whitespace() {
            spans.push(Span::raw(ch.to_string()));
            continue;
        }

        let intensity = intensities.get(visible_index).copied().unwrap_or_default();
        let color = blend_color(base, highlight, intensity);
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        visible_index = visible_index.wrapping_add(1);
    }

    spans
}

pub(crate) fn shimmer_levels(visible_count: usize, config: ShimmerConfig) -> Vec<f32> {
    if visible_count == 0 {
        return Vec::new();
    }

    let period = visible_count + config.padding * 2;
    let sweep_seconds = config.sweep_seconds.max(0.1);
    let pos_f =
        (elapsed_since_start().as_secs_f32() % sweep_seconds) / sweep_seconds * (period as f32);
    let pos = pos_f as isize;

    (0..visible_count)
        .map(|index| {
            let i_pos = index as isize + config.padding as isize;
            let dist = (i_pos - pos).abs() as f32;

            if dist <= config.band_half_width {
                let x = std::f32::consts::PI * (dist / config.band_half_width.max(0.5));
                (0.5 * (1.0 + x.cos()) * config.peak).clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .collect()
}

fn blend_color(base: Color, highlight: Color, intensity: f32) -> Color {
    let (r1, g1, b1) = color_to_rgb(base);
    let (r2, g2, b2) = color_to_rgb(highlight);
    let t = intensity.clamp(0.0, 1.0);
    Color::Rgb(
        lerp_channel(r1, r2, t),
        lerp_channel(g1, g2, t),
        lerp_channel(b1, b2, t),
    )
}

fn lerp_channel(from: u8, to: u8, t: f32) -> u8 {
    let start = from as f32;
    let end = to as f32;
    (start + ((end - start) * t)).round().clamp(0.0, 255.0) as u8
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 255, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (96, 96, 96),
        Color::LightRed => (255, 102, 102),
        Color::LightGreen => (102, 255, 102),
        Color::LightYellow => (255, 255, 153),
        Color::LightBlue => (102, 178, 255),
        Color::LightMagenta => (255, 102, 255),
        Color::LightCyan => (153, 255, 255),
        Color::White => (255, 255, 255),
        _ => (192, 192, 192),
    }
}

#[cfg(test)]
mod tests {
    use super::{ShimmerConfig, shimmer_levels, shimmer_text_spans};
    use ratatui::style::Color;

    #[test]
    fn shimmer_levels_match_visible_character_count() {
        let levels = shimmer_levels(8, ShimmerConfig::action());
        assert_eq!(levels.len(), 8);
    }

    #[test]
    fn shimmer_text_spans_preserve_whitespace() {
        let spans = shimmer_text_spans("A B", Color::Blue, Color::White, ShimmerConfig::action());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content.as_ref(), " ");
    }
}
