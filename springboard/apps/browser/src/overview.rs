//! Tab overview (Browser): View > Show All Tabs (Cmd+Shift+backslash) covers the content area with a grid of cards, one
//! per open tab, showing its title and address. A card selects its tab and closes the overview; Escape closes it.
//! The engine keeps no snapshot of a page, so a card is text only, with the selected tab marked by a tint ring.
//! The layout is in points, so it follows the window size and the UI scale.
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{Weight, CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, TINT};
use bifrost_app::kit::{Surface, Ui};

pub const CARD_W_PT: f32 = 220.0;
pub const CARD_H_PT: f32 = 84.0;
const GAP_PT: f32 = 16.0;
const PAD_PT: f32 = 24.0;
const TEXT_PAD_PT: f32 = 12.0;
const RING_PT: f32 = 2.0;
/// Start page address; a tab on it is labelled "New Tab".
const BLANK: &str = "about:blank";

#[derive(Default)]
pub struct Overview {
    pub open: bool,
}

/// Cards per row for a content width in points; at least one.
pub fn columns(width_pt: f32) -> usize {
    let inner = width_pt - 2.0 * PAD_PT + GAP_PT;
    ((inner / (CARD_W_PT + GAP_PT)).floor().max(1.0)) as usize
}

/// The rectangle of card `i` in a grid of `cols` columns that starts at `top_pt`.
pub fn card_rect(i: usize, cols: usize, top_pt: f32) -> Rect {
    let (c, r) = (i % cols, i / cols);
    Rect {
        x: PAD_PT + c as f32 * (CARD_W_PT + GAP_PT),
        y: top_pt + PAD_PT + r as f32 * (CARD_H_PT + GAP_PT),
        w: CARD_W_PT,
        h: CARD_H_PT,
    }
}

/// The tab whose card is under the point (points), if any.
pub fn hit(count: usize, width_pt: f32, top_pt: f32, px: f32, py: f32) -> Option<usize> {
    let cols = columns(width_pt);
    (0..count).find(|&i| card_rect(i, cols, top_pt).contains(px, py))
}

/// The address without its scheme or a trailing slash ("example.com/docs").
pub fn address(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.strip_suffix('/').unwrap_or(rest).to_string()
}

/// The two lines of a card: the title (or "New Tab" on the start page, the address when the page has no title) and
/// the address.
pub fn label(title: &str, url: &str) -> (String, String) {
    let shown_url = address(url);
    let first = if url.is_empty() || url == BLANK {
        "New Tab".to_string()
    } else if title.trim().is_empty() {
        shown_url.clone()
    } else {
        title.trim().to_string()
    };
    (first, shown_url)
}

impl Overview {
    /// Draws the overview over the content area from `top_pt` down. `tabs` holds each tab's (title, URL) in tab order.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, width_pt: f32, top_pt: f32, tabs: &[(String, String)], current: usize) {
        if !self.open {
            return;
        }
        let y = ui.pt(top_pt).max(0.0) as usize;
        s.fill_rect(0, y, s.w, s.h.saturating_sub(y), CONTENT_BG.now());
        let cols = columns(width_pt);
        let rad = ui.pt(m::CARD_RADIUS);
        for (i, (title, url)) in tabs.iter().enumerate() {
            let r = card_rect(i, cols, top_pt);
            let selected = i == current;
            if selected {
                s.fill_rrect(ui.pt(r.x - RING_PT), ui.pt(r.y - RING_PT), ui.pt(r.w + 2.0 * RING_PT), ui.pt(r.h + 2.0 * RING_PT), rad + ui.pt(RING_PT), TINT);
            }
            s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), rad, CARD_BG.now());
            let (first, second) = label(title, url);
            let room = (r.w - 2.0 * TEXT_PAD_PT).max(0.0);
            let weight = if selected { Weight::Semibold } else { Weight::Regular };
            let head = ui.text(&ui.fit(&first, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, weight, LABEL.now());
            s.blit(&head, ui.pt(r.x + TEXT_PAD_PT) as i32, ui.pt(r.y + TEXT_PAD_PT) as i32, 255, 255);
            let line = ui.text(&ui.fit(&second, MAC_FOOTNOTE_PT, room), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(&line, ui.pt(r.x + TEXT_PAD_PT) as i32, ui.pt(r.y + r.h - TEXT_PAD_PT - 14.0) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fills_the_width_in_whole_cards() {
        assert_eq!(columns(0.0), 1);
        assert_eq!(columns(260.0), 1);
        assert_eq!(columns(700.0), 2);
        assert_eq!(columns(740.0), 3);
        assert_eq!(columns(1200.0), 4);
    }

    #[test]
    fn cards_wrap_into_rows_and_hit_their_own_rectangle() {
        let cols = 3;
        let second_row = card_rect(3, cols, 40.0);
        assert_eq!(second_row.x, PAD_PT);
        assert_eq!(second_row.y, 40.0 + PAD_PT + CARD_H_PT + GAP_PT);
        assert_eq!(hit(5, 740.0, 40.0, card_rect(1, 3, 40.0).x + 4.0, card_rect(1, 3, 40.0).y + 4.0), Some(1));
        assert_eq!(hit(2, 740.0, 40.0, 1.0, 1.0), None);
        assert_eq!(hit(5, 740.0, 40.0, second_row.x + 1.0, second_row.y + 1.0), Some(3));
        assert_eq!(hit(3, 740.0, 40.0, second_row.x + 1.0, second_row.y + 1.0), None);
    }

    #[test]
    fn addresses_drop_the_scheme_and_a_trailing_slash() {
        assert_eq!(address("https://example.com/"), "example.com");
        assert_eq!(address("https://example.com/docs"), "example.com/docs");
        assert_eq!(address("about:blank"), "about:blank");
    }

    #[test]
    fn labels_fall_back_to_the_address_and_the_start_page() {
        assert_eq!(label("", BLANK), ("New Tab".into(), "about:blank".into()));
        assert_eq!(label("", ""), ("New Tab".into(), String::new()));
        assert_eq!(label("", "https://a.test/x"), ("a.test/x".into(), "a.test/x".into()));
        assert_eq!(label(" Docs ", "https://a.test/"), ("Docs".into(), "a.test".into()));
    }
}
