//! Geometry of a preferences window: a full-size-content split window (`NativeWindow` with
//! `NativeWindowStyleMaskFullSizeContentView`, `NativeSplitViewController` sidebar + detail) and the grouped list in its detail
//! pane (`NativeCollectionLayoutListConfiguration` inset-grouped). Pure model in points; renderers supply colour and glyphs.
//!
//! Values are measured from the Golden Gate Settings reference at 2x (see `docs/settings-layout-spec.md`).
use crate::Rect;

/// Measured metrics, in points.
pub mod m {
    pub const SIDEBAR_W: f32 = 232.0;
    pub const SIDEBAR_INSET: f32 = 10.0;
    pub const SIDEBAR_ROW_PITCH: f32 = 32.0;
    pub const SIDEBAR_ROW_H: f32 = 30.0;
    pub const SIDEBAR_ICON: f32 = 20.0;
    pub const SIDEBAR_ICON_X: f32 = 19.0;
    pub const SIDEBAR_LABEL_X: f32 = 46.0;
    pub const SIDEBAR_GROUP_GAP: f32 = 13.0;
    pub const TRAFFIC_Y: f32 = 26.0;
    pub const TRAFFIC_X0: f32 = 26.0;
    pub const TRAFFIC_PITCH: f32 = 23.0;
    pub const SEARCH_Y: f32 = 61.0;
    pub const SEARCH_H: f32 = 28.0;
    pub const ACCOUNT_Y: f32 = 100.0;
    pub const ACCOUNT_H: f32 = 40.0;
    pub const ACCOUNT_AVATAR: f32 = 38.0;
    pub const DETAIL_MARGIN: f32 = 20.0;
    pub const TOOLBAR_Y: f32 = 8.0;
    pub const TOOLBAR_X: f32 = 8.0;
    pub const TOOLBAR_PILL_W: f32 = 73.0;
    pub const TOOLBAR_PILL_H: f32 = 36.0;
    pub const HEADER_Y: f32 = 64.0;
    pub const HEADER_H: f32 = 166.0;
    pub const HEADER_ICON: f32 = 49.0;
    /// The compact header card (icon, title and description in one row): icon tile, its inset and the text column's left edge
    /// inside the card, and the padding above and below the text.
    pub const HEADER_COMPACT_ICON: f32 = 27.0;
    pub const HEADER_COMPACT_ICON_X: f32 = 11.0;
    pub const HEADER_COMPACT_TEXT_X: f32 = 48.0;
    pub const HEADER_COMPACT_PAD: f32 = 12.0;
    /// The pane title in the toolbar starts this far to the right of the back and forward pill.
    pub const TOOLBAR_TITLE_GAP: f32 = 12.0;
    /// A segmented control inside a row: its height, and the inset from the card edge.
    pub const SEGMENT_H: f32 = 28.0;
    pub const SEGMENT_INSET: f32 = 11.0;
    pub const CARD_GAP: f32 = 11.0;
    pub const ROW_H: f32 = 49.0;
    pub const ROW_ICON: f32 = 23.0;
    pub const ROW_ICON_X: f32 = 12.5;
    pub const ROW_LABEL_X: f32 = 48.5;
    pub const CHEVRON_INSET: f32 = 11.0;
    pub const WINDOW_RADIUS: f32 = 17.0;
    pub const CARD_RADIUS: f32 = 13.0;
    pub const SELECTION_RADIUS: f32 = 9.0;
    pub const ICON_RADIUS: f32 = 5.0;
}

/// The regions of the window for a given size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub sidebar: Rect,
    pub detail: Rect,
    pub search: Rect,
    pub account: Rect,
    pub toolbar_pill: Rect,
    /// the grouped content column of the detail pane (margin applied)
    pub column: Rect,
}

impl Frame {
    /// Lays out a window of `w` x `h` points. The sidebar never takes more than 40% of a narrow window.
    pub fn new(w: f32, h: f32) -> Self {
        let side = m::SIDEBAR_W.min(w * 0.4).max(0.0);
        let sidebar = Rect { x: 0.0, y: 0.0, w: side, h };
        let detail = Rect { x: side, y: 0.0, w: (w - side).max(0.0), h };
        let search = Rect { x: m::SIDEBAR_INSET, y: m::SEARCH_Y, w: (side - 2.0 * m::SIDEBAR_INSET).max(0.0), h: m::SEARCH_H };
        let account = Rect { x: m::SIDEBAR_INSET, y: m::ACCOUNT_Y, w: search.w, h: m::ACCOUNT_H };
        let toolbar_pill = Rect { x: side + m::TOOLBAR_X, y: m::TOOLBAR_Y, w: m::TOOLBAR_PILL_W, h: m::TOOLBAR_PILL_H };
        let column = Rect { x: side + m::DETAIL_MARGIN, y: m::HEADER_Y, w: (detail.w - 2.0 * m::DETAIL_MARGIN).max(0.0), h: (h - m::HEADER_Y).max(0.0) };
        Frame { sidebar, detail, search, account, toolbar_pill, column }
    }

    /// Centre of traffic light `i` (0 close, 1 minimise, 2 zoom).
    pub fn traffic_light(i: usize) -> (f32, f32) {
        (m::TRAFFIC_X0 + m::TRAFFIC_PITCH * i as f32, m::TRAFFIC_Y)
    }
}

/// One sidebar group: `rows` rows after a gap. Returns each row's selection rect (inset, 30 pt tall) and the y where
/// the next group starts. `y` is the top of the group.
pub fn sidebar_group(frame: &Frame, y: f32, rows: usize) -> (Vec<Rect>, f32) {
    let out = (0..rows)
        .map(|i| Rect { x: m::SIDEBAR_INSET, y: y + m::SIDEBAR_ROW_PITCH * i as f32, w: (frame.sidebar.w - 2.0 * m::SIDEBAR_INSET).max(0.0), h: m::SIDEBAR_ROW_H })
        .collect();
    (out, y + m::SIDEBAR_ROW_PITCH * rows as f32 + m::SIDEBAR_GROUP_GAP)
}

/// A card of `rows` rows at `y` in the column: its rect and each row's rect. The header card is `Frame`-independent.
pub fn card(frame: &Frame, y: f32, rows: usize) -> (Rect, Vec<Rect>, f32) {
    let c = Rect { x: frame.column.x, y, w: frame.column.w, h: m::ROW_H * rows as f32 };
    let rs = (0..rows).map(|i| Rect { x: c.x, y: y + m::ROW_H * i as f32, w: c.w, h: m::ROW_H }).collect();
    (c, rs, y + c.h + m::CARD_GAP)
}

/// Where the grouped list starts, below the header card.
pub fn list_top() -> f32 {
    m::HEADER_Y + m::HEADER_H + m::CARD_GAP + 9.0
}

/// Separator span of row `i` in a card: from the label to the trailing edge, none under the last row.
pub fn separator(row: Rect, last: bool, has_icon: bool) -> Option<(f32, f32)> {
    if last {
        return None;
    }
    let x0 = row.x + if has_icon { m::ROW_LABEL_X } else { m::ROW_ICON_X };
    Some((x0, row.x + row.w))
}

/// Where the pane title starts in the toolbar: right of the back and forward pill, never centred.
pub fn toolbar_title_x() -> f32 {
    m::TOOLBAR_X + m::TOOLBAR_PILL_W + m::TOOLBAR_TITLE_GAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbar_title_starts_after_the_pill() {
        // measured: the pill ends at 81 pt and the title starts about 12 pt after it
        assert_eq!(toolbar_title_x(), 93.0);
        assert!(toolbar_title_x() > m::TOOLBAR_X + m::TOOLBAR_PILL_W);
    }

    #[test]
    fn frame_matches_the_reference_at_full_size() {
        let f = Frame::new(740.0, 690.0);
        assert_eq!(f.sidebar.w, 232.0);
        assert_eq!(f.detail.x, 232.0);
        assert_eq!(f.detail.w, 508.0);
        assert_eq!((f.search.x, f.search.w, f.search.h), (10.0, 212.0, 28.0));
        assert_eq!(f.column.w, 468.0, "pane 508 minus 2 x 20 margin");
        assert_eq!(f.toolbar_pill.w, 73.0);
    }

    #[test]
    fn narrow_window_caps_the_sidebar_and_never_goes_negative() {
        let f = Frame::new(300.0, 400.0);
        assert_eq!(f.sidebar.w, 120.0);
        assert!(f.column.w >= 0.0 && f.search.w >= 0.0);
        let z = Frame::new(0.0, 0.0);
        assert_eq!((z.sidebar.w, z.column.w, z.search.w), (0.0, 0.0, 0.0));
    }

    #[test]
    fn sidebar_rows_are_30_tall_on_a_32_pitch_with_a_group_gap() {
        let f = Frame::new(740.0, 690.0);
        let (rows, next) = sidebar_group(&f, 180.0, 5);
        assert_eq!(rows.len(), 5);
        assert_eq!((rows[0].h, rows[1].y - rows[0].y), (30.0, 32.0));
        assert_eq!(rows[0].w, 212.0);
        assert_eq!(next, 180.0 + 160.0 + 13.0);
    }

    #[test]
    fn cards_stack_with_the_card_gap_and_49pt_rows() {
        let f = Frame::new(740.0, 690.0);
        let (c, rows, next) = card(&f, 300.0, 3);
        assert_eq!((c.h, rows.len(), rows[2].y), (147.0, 3, 398.0));
        assert_eq!(next, 300.0 + 147.0 + 11.0);
    }

    #[test]
    fn separators_start_at_the_label_and_skip_the_last_row() {
        let f = Frame::new(740.0, 690.0);
        let (_, rows, _) = card(&f, 0.0, 2);
        assert_eq!(separator(rows[0], false, true), Some((f.column.x + 48.5, f.column.x + f.column.w)));
        assert_eq!(separator(rows[0], false, false).unwrap().0, f.column.x + 12.5);
        assert_eq!(separator(rows[1], true, true), None);
    }

    #[test]
    fn traffic_lights_are_23pt_apart() {
        assert_eq!(Frame::traffic_light(1).0 - Frame::traffic_light(0).0, 23.0);
    }
}
