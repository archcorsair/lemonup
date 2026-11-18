//! The scroll of a cut value on the selected row.

/// Where the visible part of a `text_len` characters long value starts after
/// `ticks` ticks, in a column `width` characters wide.
///
/// The start holds for `hold_ticks` ticks, then the value moves one character
/// per tick until its end shows, holds there for `hold_ticks` ticks, then
/// jumps back and repeats. A value that fits never moves.
pub fn marquee_offset(text_len: usize, width: usize, ticks: usize, hold_ticks: usize) -> usize {
    let max_offset = text_len.saturating_sub(width);
    if max_offset == 0 {
        return 0;
    }
    let hold = hold_ticks.max(1);
    let cycle = 2 * hold + max_offset - 1;
    let tick = ticks % cycle;
    if tick < hold {
        0
    } else {
        (tick - hold + 1).min(max_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_that_fits_never_moves() {
        assert_eq!(marquee_offset(10, 12, 0, 3), 0);
        assert_eq!(marquee_offset(12, 12, 500, 3), 0);
    }

    #[test]
    fn holds_scrolls_one_character_per_tick_holds_and_resets() {
        // 14 characters in 12 columns: two characters to scroll, hold of 3.
        let offsets: Vec<usize> = (0..12)
            .map(|tick| marquee_offset(14, 12, tick, 3))
            .collect();
        assert_eq!(offsets, [0, 0, 0, 1, 2, 2, 2, 0, 0, 0, 1, 2]);
    }

    #[test]
    fn the_end_holds_as_long_as_the_start() {
        let (hold, max) = (20, 6);
        let cycle = 2 * hold + max - 1;
        let at_start = (0..cycle)
            .filter(|&tick| marquee_offset(12 + max, 12, tick, hold) == 0)
            .count();
        let at_end = (0..cycle)
            .filter(|&tick| marquee_offset(12 + max, 12, tick, hold) == max)
            .count();
        assert_eq!((at_start, at_end), (hold, hold));
    }
}
