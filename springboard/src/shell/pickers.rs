//! The media picker list and the route picker menu, drawn by the shell on the menu material. The media picker's rows
//! come from `bifrost_sdk`'s `media_picker_view` (the model is `bifrost_ui::media_picker`); the route menu's rows are
//! `bifrost_ui::route_picker::Destination`. Row height and corner radius follow the shared list and menu tokens.

use super::list::ROW_PT;
use super::music::icons;
use super::ui::{Ui, Weight, LABEL_2, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, menu_highlight, MENU_HIGHLIGHT_TEXT, MENU_RADIUS_PT, MENU_TINT, SECONDARY, accent_now, WHITE};
use crate::gfx::Surface;
use bifrost_ui::route_picker::Destination;
use bifrost_ui::Rect;

/// Height of the media picker's footer (the chosen count) in pt.
pub const FOOTER_PT: f32 = 26.0;
/// The check mark's width in pt; titles start after it.
const CHECK_PT: f32 = 22.0;
/// The space before a row's text, in pt.
const PAD_PT: f32 = 14.0;

/// One media picker row as the shell draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerRow {
    pub title: String,
    pub subtitle: String,
    pub chosen: bool,
    /// False for items listed but not playable: drawn dimmed.
    pub enabled: bool,
}

/// The rectangle of row `index` in a list that starts at the top of `panel`.
pub fn row_rect(panel: Rect, index: usize, row_h: f32) -> Rect {
    Rect { x: panel.x, y: panel.y + index as f32 * row_h, w: panel.w, h: row_h }
}

/// The row under (x, y) in a list of `count` rows of `row_h` px at the top of `panel`; None outside the rows.
pub fn row_at(panel: Rect, row_h: f32, count: usize, x: f32, y: f32) -> Option<usize> {
    if !panel.contains(x, y) || row_h <= 0.0 {
        return None;
    }
    let index = ((y - panel.y) / row_h) as usize;
    (index < count).then_some(index)
}

/// A check mark centred on (cx, cy), `size` px wide.
fn check(s: &mut Surface, (cx, cy): (f32, f32), size: f32, color: u32) {
    let pts = [(-0.5, 0.02), (-0.38, -0.1), (-0.15, 0.12), (0.38, -0.4), (0.5, -0.28), (-0.15, 0.35)];
    let pts: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (cx + x * size, cy + y * size)).collect();
    icons::fill_poly(s, &pts, color);
}

/// The media picker: one row per item with its title, subtitle and a check mark when chosen, then a footer with the
/// count. Rows that do not fit above the footer are not drawn. Disabled rows are dimmed.
pub fn draw_media_picker(s: &mut Surface, ui: &Ui, panel: Rect, rows: &[PickerRow], footer: &str) {
    s.fill_rrect(panel.x, panel.y, panel.w, panel.h, ui.pt(MENU_RADIUS_PT), MENU_TINT.now());
    let row_h = ui.pt(ROW_PT);
    let list_bottom = panel.y + panel.h - ui.pt(FOOTER_PT);
    for (i, row) in rows.iter().enumerate() {
        let r = row_rect(panel, i, row_h);
        if r.y + r.h > list_bottom {
            break;
        }
        let ink = if row.enabled { WHITE } else { SECONDARY };
        if row.chosen {
            check(s, (r.x + ui.pt(PAD_PT) + ui.pt(CHECK_PT) / 2.0, r.y + r.h / 2.0), ui.pt(12.0), accent_now());
        }
        let tx = (r.x + ui.pt(PAD_PT + CHECK_PT)) as i32;
        let title = ui.text(&row.title, MAC_HEADLINE_PT, Weight::Medium, ink);
        s.blit(&title, tx, (r.y + ui.pt(7.0)) as i32, 255, 255);
        let sub = ui.text(&row.subtitle, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY);
        s.blit(&sub, tx, (r.y + ui.pt(27.0)) as i32, 255, 255);
    }
    let foot = ui.text(footer, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
    s.blit(&foot, (panel.x + ui.pt(PAD_PT)) as i32, (panel.y + panel.h - ui.pt(FOOTER_PT) + ui.pt(7.0)) as i32, 255, 255);
}

/// The route picker menu: one row per destination (local outputs first, as the model orders them), a check mark on
/// the active route and a highlight under the row `hover` names.
pub fn draw_route_menu(s: &mut Surface, ui: &Ui, panel: Rect, rows: &[Destination], hover: Option<usize>) {
    s.fill_rrect(panel.x, panel.y, panel.w, panel.h, ui.pt(MENU_RADIUS_PT), MENU_TINT.now());
    let row_h = ui.pt(ROW_PT);
    for (i, row) in rows.iter().enumerate() {
        let r = row_rect(panel, i, row_h);
        if r.y + r.h > panel.y + panel.h {
            break;
        }
        let hovered = hover == Some(i);
        let ink = if hovered { MENU_HIGHLIGHT_TEXT } else { WHITE };
        if hovered {
            s.fill_rrect(r.x + ui.pt(4.0), r.y, r.w - ui.pt(8.0), r.h, ui.pt(5.0), menu_highlight());
        }
        if row.active {
            check(s, (r.x + ui.pt(PAD_PT) + ui.pt(CHECK_PT) / 2.0, r.y + r.h / 2.0), ui.pt(12.0), ink);
        }
        let label = ui.text(&row.name, MAC_HEADLINE_PT, Weight::Regular, ink);
        s.blit(&label, (r.x + ui.pt(PAD_PT + CHECK_PT)) as i32, (r.y + (r.h - label.h as f32) / 2.0) as i32, 255, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn panel() -> Rect {
        Rect { x: 10.0, y: 20.0, w: 240.0, h: 300.0 }
    }

    fn dest(name: &str, active: bool) -> Destination {
        Destination { id: name.to_lowercase(), name: name.to_string(), local: true, active }
    }

    #[test]
    fn row_at_maps_points_to_rows_and_misses_outside() {
        let p = panel();
        assert_eq!(row_at(p, 49.0, 3, 20.0, 20.0), Some(0));
        assert_eq!(row_at(p, 49.0, 3, 20.0, 20.0 + 49.0 * 2.5), Some(2));
        assert_eq!(row_at(p, 49.0, 2, 20.0, 20.0 + 49.0 * 2.5), None, "past the last row");
        assert_eq!(row_at(p, 49.0, 3, 0.0, 30.0), None, "left of the panel");
        assert_eq!(row_at(p, 0.0, 3, 20.0, 30.0), None, "no row height");
    }

    #[test]
    fn row_rect_stacks_rows_from_the_panel_top() {
        let r = row_rect(panel(), 2, 49.0);
        assert_eq!((r.x, r.y, r.w, r.h), (10.0, 20.0 + 98.0, 240.0, 49.0));
    }

    #[test]
    fn the_route_menu_fills_its_panel_and_leaves_the_rest_alone() {
        let ui = ui();
        let (w, h) = (300usize, 260usize);
        let rows = [dest("Speaker", true), dest("Headphones", false)];
        let mut px = vec![0u32; w * h];
        {
            let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
            draw_route_menu(&mut s, &ui, Rect { x: 10.0, y: 10.0, w: 240.0, h: 120.0 }, &rows, Some(1));
        }
        assert_eq!(px[0], 0, "outside the menu is untouched");
        assert_ne!(px[15 * w + 200], 0, "the menu material fills its panel");
    }

    #[test]
    fn the_media_picker_draws_rows_and_an_empty_list() {
        let ui = ui();
        let (w, h) = (300usize, 200usize);
        let mut px = vec![0u32; w * h];
        let rows = vec![PickerRow { title: "Song".into(), subtitle: "Artist".into(), chosen: true, enabled: true }];
        {
            let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
            draw_media_picker(&mut s, &ui, Rect { x: 0.0, y: 0.0, w: 300.0, h: 180.0 }, &rows, "1 selected");
            draw_media_picker(&mut s, &ui, Rect { x: 0.0, y: 0.0, w: 300.0, h: 180.0 }, &[], "0 selected");
        }
        assert_ne!(px[10 * w + 150], 0, "the picker material fills the panel");
    }
}
