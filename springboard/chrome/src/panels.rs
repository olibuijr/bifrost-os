//! The panels client: Notification Center, Control Center and search as overlay layer-shell surfaces with glass
//! materials from the window server. Owner: `chrome-panels`.
//!
//! This file holds the client's pure logic: which panel a `ShowPanel` name opens, where each panel sits on the
//! output, how notifications group by app, how Control Center tiles lay out, how a panel's slide and fade progress
//! from its open or close time, and how a control edit becomes a `FromChrome` message. The Wayland side (the layer
//! surfaces, the glass material on the panel layer and the server-run animations) uses `surface` and `link` from
//! `chrome-core` and is not wired yet, so `run` reports that instead of drawing.

use crate::model::{ControlModule, FromChrome, Notification, PanelsModel};
use bifrost_render::motion::{self, Curve};
use std::collections::BTreeMap;

/// Window corner radius (`prefs_layout` `m::*`, the window radius of the design rules).
pub const PANEL_RADIUS: f32 = 17.0;
/// Grouped card radius inside a panel.
pub const CARD_RADIUS: f32 = 13.0;
/// Gap between a panel and the output edges, in logical pixels.
pub const MARGIN: f32 = 8.0;
/// Notification Center width.
pub const NOTIFICATION_WIDTH: f32 = 360.0;
/// Control Center width.
pub const CONTROL_WIDTH: f32 = 340.0;
/// Search panel width and the height of its search field (a capsule).
pub const SEARCH_WIDTH: f32 = 560.0;
pub const SEARCH_HEIGHT: f32 = 52.0;
/// Open and close animation length, in milliseconds (the window server runs it).
pub const ANIMATION_MS: u32 = 240;
/// Control Center tile metrics.
pub const TILE_PAD: f32 = 12.0;
pub const TILE_GAP: f32 = 8.0;
pub const TOGGLE_HEIGHT: f32 = 56.0;
pub const SLIDER_HEIGHT: f32 = 44.0;

/// The three panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Notifications,
    Control,
    Search,
}

/// The panel a `ShowPanel` name opens. `""` (close) and unknown names give `None`.
pub fn parse_panel(name: &str) -> Option<Panel> {
    match name {
        "notifications" => Some(Panel::Notifications),
        "control" => Some(Panel::Control),
        "search" => Some(Panel::Search),
        _ => None,
    }
}

/// A rectangle in output logical pixels, origin top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Where a panel sits. `top` is the bottom edge of the menu bar (its exclusive zone); `control_height` is the
/// content height of Control Center from `control_layout`. Widths clamp to the output so a narrow screen keeps
/// `MARGIN` on each side.
pub fn panel_rect(panel: Panel, output: (f32, f32), top: f32, control_height: f32) -> Rect {
    let (out_w, out_h) = output;
    let max_w = (out_w - 2.0 * MARGIN).max(0.0);
    match panel {
        Panel::Notifications => {
            let w = NOTIFICATION_WIDTH.min(max_w);
            let y = top + MARGIN;
            Rect { x: out_w - w - MARGIN, y, w, h: (out_h - y - MARGIN).max(0.0) }
        }
        Panel::Control => {
            let w = CONTROL_WIDTH.min(max_w);
            let y = top + MARGIN;
            Rect { x: out_w - w - MARGIN, y, w, h: control_height.min((out_h - y - MARGIN).max(0.0)) }
        }
        Panel::Search => {
            let w = SEARCH_WIDTH.min(max_w);
            Rect { x: (out_w - w) / 2.0, y: top + 2.0 * MARGIN, w, h: SEARCH_HEIGHT }
        }
    }
}

/// Notifications grouped by app, as Notification Center shows them. Within a group the newest (highest id) comes
/// first; groups are ordered by their newest notification, newest first, with the app id breaking ties. The store
/// assigns increasing ids, so ids order the notifications (the model's `time` is display text).
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationGroup {
    pub app_id: String,
    pub items: Vec<Notification>,
}

pub fn group_notifications(items: &[Notification]) -> Vec<NotificationGroup> {
    let mut by_app: BTreeMap<&str, Vec<&Notification>> = BTreeMap::new();
    for n in items {
        by_app.entry(n.app_id.as_str()).or_default().push(n);
    }
    let mut groups: Vec<NotificationGroup> = by_app
        .into_iter()
        .map(|(app, mut list)| {
            list.sort_by(|a, b| b.id.cmp(&a.id));
            NotificationGroup { app_id: app.to_string(), items: list.into_iter().cloned().collect() }
        })
        .collect();
    groups.sort_by(|a, b| {
        let newest = |g: &NotificationGroup| g.items.first().map_or(0, |n| n.id);
        newest(b).cmp(&newest(a)).then_with(|| a.app_id.cmp(&b.app_id))
    });
    groups
}

/// Where each Control Center module sits inside the panel (relative to its top left) and the content height.
/// Toggles (`value < 0`) pair two to a row; sliders (`value >= 0`) take the full width on their own row.
pub fn control_layout(modules: &[ControlModule], width: f32) -> (Vec<Rect>, f32) {
    if modules.is_empty() {
        return (Vec::new(), 2.0 * TILE_PAD);
    }
    let inner = width - 2.0 * TILE_PAD;
    let half = (inner - TILE_GAP) / 2.0;
    let mut rects = Vec::with_capacity(modules.len());
    let mut y = TILE_PAD;
    let mut col = 0usize;
    for m in modules {
        if m.value < 0.0 {
            let x = TILE_PAD + col as f32 * (half + TILE_GAP);
            rects.push(Rect { x, y, w: half, h: TOGGLE_HEIGHT });
            if col == 1 {
                y += TOGGLE_HEIGHT + TILE_GAP;
                col = 0;
            } else {
                col = 1;
            }
        } else {
            if col == 1 {
                y += TOGGLE_HEIGHT + TILE_GAP;
                col = 0;
            }
            rects.push(Rect { x: TILE_PAD, y, w: inner, h: SLIDER_HEIGHT });
            y += SLIDER_HEIGHT + TILE_GAP;
        }
    }
    if col == 1 {
        y += TOGGLE_HEIGHT + TILE_GAP;
    }
    (rects, y - TILE_GAP + TILE_PAD)
}

/// Eased progress of an open or close animation: `elapsed_ms / duration_ms`, clamped to 0..1 (a zero duration is
/// already finished).
pub fn progress(elapsed_ms: u32, duration_ms: u32) -> f32 {
    if duration_ms == 0 {
        return 1.0;
    }
    (elapsed_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
}

/// How far a panel is still pushed off its edge at `progress` (0 = starts `distance` away, 1 = in place). Eased on
/// the shared `PanelReveal` curve (ease-out cubic), so a panel decelerates into place.
pub fn slide_offset(progress: f32, distance: f32) -> f32 {
    let eased = motion::ease(Curve::EaseOut, progress);
    distance * (1.0 - eased)
}

/// Opacity of a panel at `progress`: linear from 0 to 1 over the same animation.
pub fn fade_alpha(progress: f32) -> f32 {
    progress.clamp(0.0, 1.0)
}

/// Applies a toggle tap to Control Center's model and returns the message for the shell. `None` if no module has
/// that id.
pub fn toggle(model: &mut PanelsModel, id: u32) -> Option<FromChrome> {
    let m = model.controls.iter_mut().find(|m| m.id == id)?;
    if m.value >= 0.0 {
        return None;
    }
    m.on = !m.on;
    Some(FromChrome::ControlChanged { id, on: m.on, value: m.value })
}

/// Applies a slider move (clamped to 0..1) and returns the message for the shell. `None` if no slider has that id.
pub fn set_value(model: &mut PanelsModel, id: u32, value: f32) -> Option<FromChrome> {
    let m = model.controls.iter_mut().find(|m| m.id == id)?;
    if m.value < 0.0 {
        return None;
    }
    m.value = value.clamp(0.0, 1.0);
    Some(FromChrome::ControlChanged { id, on: m.on, value: m.value })
}

/// Runs the client until the shell or the compositor goes away. Until the surface API is wired this reports why it
/// cannot draw, so the shell falls back to its own panels.
pub fn run() -> Result<(), String> {
    Err("bifrost-panels: the layer surfaces are not wired yet (chrome-core surface API)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32, app: &str) -> Notification {
        Notification { id, app_id: app.to_string(), ..Default::default() }
    }

    fn module(id: u32, value: f32) -> ControlModule {
        ControlModule { id, value, ..Default::default() }
    }

    #[test]
    fn parses_panel_names_and_close() {
        assert_eq!(parse_panel("notifications"), Some(Panel::Notifications));
        assert_eq!(parse_panel("control"), Some(Panel::Control));
        assert_eq!(parse_panel("search"), Some(Panel::Search));
        assert_eq!(parse_panel(""), None);
        assert_eq!(parse_panel("unknown"), None);
    }

    #[test]
    fn notification_center_hangs_from_the_top_right() {
        // 1920x1080 output, menu bar 28 px: x = 1920 - 360 - 8, y = 28 + 8, h = 1080 - 36 - 8.
        let r = panel_rect(Panel::Notifications, (1920.0, 1080.0), 28.0, 0.0);
        assert_eq!(r, Rect { x: 1552.0, y: 36.0, w: 360.0, h: 1036.0 });
    }

    #[test]
    fn panels_clamp_to_a_narrow_output() {
        // 300 px wide output: the width is 300 - 16 = 284 and the panel keeps the 8 px margins.
        let r = panel_rect(Panel::Notifications, (300.0, 600.0), 0.0, 0.0);
        assert_eq!(r.w, 284.0);
        assert_eq!(r.x, 8.0);
        let s = panel_rect(Panel::Search, (300.0, 600.0), 0.0, 0.0);
        assert_eq!((s.x, s.w), (8.0, 284.0));
    }

    #[test]
    fn search_panel_is_centred_below_the_bar() {
        // (1920 - 560) / 2 = 680; y = 28 + 2 * 8 = 44.
        let r = panel_rect(Panel::Search, (1920.0, 1080.0), 28.0, 0.0);
        assert_eq!(r, Rect { x: 680.0, y: 44.0, w: 560.0, h: 52.0 });
    }

    #[test]
    fn control_center_height_follows_its_content() {
        let r = panel_rect(Panel::Control, (1920.0, 1080.0), 28.0, 132.0);
        assert_eq!((r.x, r.w, r.h), (1572.0, 340.0, 132.0));
        // Content taller than the output is cut to the space that is left.
        let short = panel_rect(Panel::Control, (1920.0, 100.0), 28.0, 500.0);
        assert_eq!(short.h, 100.0 - 36.0 - 8.0);
    }

    #[test]
    fn groups_notifications_by_app_newest_first() {
        let items = vec![note(1, "mail"), note(2, "term"), note(3, "mail"), note(4, "term"), note(5, "mail")];
        let groups = group_notifications(&items);
        assert_eq!(groups.len(), 2);
        // "term" holds id 4, "mail" holds id 5, so mail is first.
        assert_eq!(groups[0].app_id, "mail");
        let mail: Vec<u32> = groups[0].items.iter().map(|n| n.id).collect();
        assert_eq!(mail, vec![5, 3, 1]);
        let term: Vec<u32> = groups[1].items.iter().map(|n| n.id).collect();
        assert_eq!(term, vec![4, 2]);
    }

    #[test]
    fn grouping_breaks_ties_by_app_id_and_handles_empty() {
        assert!(group_notifications(&[]).is_empty());
        let groups = group_notifications(&[note(7, "b"), note(7, "a")]);
        assert_eq!(groups.iter().map(|g| g.app_id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn control_tiles_pair_toggles_and_span_sliders() {
        // Width 340: inner 316, half (316 - 8) / 2 = 154. Two toggles, then a slider.
        let modules = [module(1, -1.0), module(2, -1.0), module(3, 0.5)];
        let (rects, h) = control_layout(&modules, 340.0);
        assert_eq!(rects[0], Rect { x: 12.0, y: 12.0, w: 154.0, h: 56.0 });
        assert_eq!(rects[1], Rect { x: 174.0, y: 12.0, w: 154.0, h: 56.0 });
        assert_eq!(rects[2], Rect { x: 12.0, y: 76.0, w: 316.0, h: 44.0 });
        assert_eq!(h, 12.0 + 56.0 + 8.0 + 44.0 + 12.0);
    }

    #[test]
    fn a_lone_toggle_takes_a_row_and_empty_is_padding_only() {
        let (rects, h) = control_layout(&[module(1, -1.0)], 340.0);
        assert_eq!(rects.len(), 1);
        assert_eq!(h, 12.0 + 56.0 + 12.0);
        let (none, empty_h) = control_layout(&[], 340.0);
        assert!(none.is_empty());
        assert_eq!(empty_h, 24.0);
    }

    #[test]
    fn a_slider_after_a_single_toggle_starts_a_new_row() {
        let (rects, _) = control_layout(&[module(1, -1.0), module(2, 0.0)], 340.0);
        assert_eq!(rects[1].y, 12.0 + 56.0 + 8.0);
        assert_eq!(rects[1].x, 12.0);
        assert_eq!(rects[1].w, 316.0);
    }

    #[test]
    fn animation_progress_and_offsets() {
        assert_eq!(progress(120, 240), 0.5);
        assert_eq!(progress(300, 240), 1.0);
        assert_eq!(progress(5, 0), 1.0);
        assert_eq!(slide_offset(0.0, 360.0), 360.0);
        assert_eq!(slide_offset(1.0, 360.0), 0.0);
        // Ease-out cubic at 0.5 is 0.875, so 12.5 % of the distance is left: 45 of 360.
        assert!((slide_offset(0.5, 360.0) - 45.0).abs() < 1e-4);
        assert_eq!(fade_alpha(0.25), 0.25);
        assert_eq!(fade_alpha(2.0), 1.0);
    }

    #[test]
    fn toggling_flips_the_switch_and_reports_it() {
        let mut model = PanelsModel { controls: vec![module(1, -1.0), module(2, 0.4)], ..Default::default() };
        let msg = toggle(&mut model, 1);
        assert_eq!(msg, Some(FromChrome::ControlChanged { id: 1, on: true, value: -1.0 }));
        assert!(model.controls[0].on);
        assert_eq!(toggle(&mut model, 1), Some(FromChrome::ControlChanged { id: 1, on: false, value: -1.0 }));
        // A slider does not toggle, and an unknown id does nothing.
        assert_eq!(toggle(&mut model, 2), None);
        assert_eq!(toggle(&mut model, 9), None);
    }

    #[test]
    fn slider_values_clamp_to_the_unit_range() {
        let mut model = PanelsModel { controls: vec![module(1, -1.0), module(2, 0.4)], ..Default::default() };
        assert_eq!(
            set_value(&mut model, 2, 1.7),
            Some(FromChrome::ControlChanged { id: 2, on: false, value: 1.0 })
        );
        assert_eq!(model.controls[1].value, 1.0);
        assert_eq!(
            set_value(&mut model, 2, -0.3),
            Some(FromChrome::ControlChanged { id: 2, on: false, value: 0.0 })
        );
        // A toggle is not a slider.
        assert_eq!(set_value(&mut model, 1, 0.5), None);
    }
}
