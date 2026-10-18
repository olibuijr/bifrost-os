//! The note list as a gallery: a grid of note cards (the first image of the note, or a text preview, then the title
//! and the date). The number of columns follows the width of the list column, so nothing is tied to a fixed count.
//! Pure functions, tested on the host; the choice is saved in Prefs (`layout=`).

/// How the note list shows its notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    List,
    Gallery,
}

impl Layout {
    /// The value saved in Prefs.
    pub fn name(self) -> &'static str {
        match self {
            Layout::List => "list",
            Layout::Gallery => "gallery",
        }
    }

    /// The layout a saved value names, or `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        [Layout::List, Layout::Gallery].into_iter().find(|l| l.name() == text)
    }
}

/// The narrowest a card may be, in points.
pub const CARD_MIN_PT: f32 = 150.0;
/// The space between cards, in points.
pub const CARD_GAP_PT: f32 = 12.0;

/// How many cards of at least `min_pt` fit across `width_pt` with `gap_pt` between them. Never fewer than one, so a
/// narrow pane still shows a card.
pub fn columns(width_pt: f32, min_pt: f32, gap_pt: f32) -> usize {
    let per = ((width_pt + gap_pt) / (min_pt + gap_pt)).floor();
    if per.is_finite() && per >= 1.0 {
        per as usize
    } else {
        1
    }
}

/// The width of one card when `cols` cards share `width_pt` with `gap_pt` between them.
pub fn card_width(width_pt: f32, cols: usize, gap_pt: f32) -> f32 {
    let cols = cols.max(1) as f32;
    ((width_pt - gap_pt * (cols - 1.0)) / cols).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_follow_the_width_and_never_drop_below_one() {
        assert_eq!(columns(600.0, 150.0, 12.0), 3, "3 cards of 150 and 2 gaps fit in 600");
        assert_eq!(columns(312.0, 150.0, 12.0), 2, "two cards and one gap take exactly 312");
        assert_eq!(columns(311.0, 150.0, 12.0), 1);
        assert_eq!(columns(100.0, 150.0, 12.0), 1, "a narrow pane still shows one card");
        assert_eq!(columns(0.0, 150.0, 12.0), 1);
        assert_eq!(columns(f32::NAN, 150.0, 12.0), 1);
        assert_eq!(columns(1200.0, 150.0, 12.0), 7);
    }

    #[test]
    fn cards_share_the_width_left_after_the_gaps() {
        assert_eq!(card_width(600.0, 3, 12.0), 192.0);
        assert_eq!(card_width(600.0, 1, 12.0), 600.0);
        assert_eq!(card_width(600.0, 0, 12.0), 600.0, "zero columns is treated as one");
        assert_eq!(card_width(10.0, 3, 12.0), 0.0, "a width too small for the gaps gives no width");
        let cols = columns(777.0, CARD_MIN_PT, CARD_GAP_PT);
        let w = card_width(777.0, cols, CARD_GAP_PT);
        assert!(w >= CARD_MIN_PT, "the cards keep at least their minimum width");
        assert!((w * cols as f32 + CARD_GAP_PT * (cols as f32 - 1.0) - 777.0).abs() < 0.01, "the row fills the width");
    }

    #[test]
    fn layouts_round_trip_their_saved_names() {
        for layout in [Layout::List, Layout::Gallery] {
            assert_eq!(Layout::parse(layout.name()), Some(layout));
        }
        assert_eq!(Layout::parse("grid"), None);
        assert_eq!(Layout::default(), Layout::List);
    }
}
