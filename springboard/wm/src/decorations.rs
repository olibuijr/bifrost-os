//! Server-side window decorations: whether the server draws a titlebar for a toplevel, and where its parts sit.
//! Third-party apps get a titlebar with the traffic lights and the title; BifrOSt's own apps draw their chrome
//! themselves and get none. Pure geometry, free of Smithay types; the compose nodes are built from these rects.

use crate::wm::Rect;

/// Titlebar height in logical pixels (the menu bar's neighbour on the reference platform is 28 px tall).
pub const TITLEBAR_H: i32 = 28;
/// Traffic light diameter and the gap between two lights.
pub const LIGHT_D: i32 = 12;
pub const LIGHT_GAP: i32 = 8;
/// Distance from the window's left edge to the first light.
pub const LIGHT_LEFT: i32 = 12;

/// App ids of BifrOSt's own apps: their chrome is drawn by the app.
pub const OWN_APP_PREFIX: &str = "is.olibuijr.bifrost.";

/// Who draws the titlebar of a toplevel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { ServerSide, None }

/// The decoration mode for a toplevel with this app id. Apps without an id count as third-party.
pub fn mode_for(app_id: Option<&str>) -> Mode {
    match app_id {
        Some(id) if id.starts_with(OWN_APP_PREFIX) => Mode::None,
        _ => Mode::ServerSide,
    }
}

/// The titlebar strip directly above a window frame.
pub fn titlebar_of(frame: Rect) -> Rect { Rect::new(frame.x, frame.y - TITLEBAR_H, frame.w, TITLEBAR_H) }

/// The three traffic lights in order: close, minimize, zoom (maximize). Each is a square of `LIGHT_D` pixels,
/// vertically centred in the bar.
pub fn lights(bar: Rect) -> [Rect; 3] {
    let y = bar.y + (bar.h - LIGHT_D) / 2;
    let step = LIGHT_D + LIGHT_GAP;
    [0, 1, 2].map(|i| Rect::new(bar.x + LIGHT_LEFT + i * step, y, LIGHT_D, LIGHT_D))
}

/// The title's box: the bar minus the lights and an equal margin on the right, so the title stays centred.
pub fn title_rect(bar: Rect) -> Rect {
    let lights_end = bar.x + LIGHT_LEFT + 3 * LIGHT_D + 2 * LIGHT_GAP;
    let margin = lights_end - bar.x;
    let w = (bar.w - 2 * margin).max(0);
    Rect::new(bar.x + margin, bar.y, w, bar.h)
}

/// What a pointer press on the titlebar hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit { Close, Minimize, Zoom, Drag, Outside }

/// The hit at (`x`, `y`) on a titlebar. The lights are checked first, then the bar drags the window.
pub fn hit(bar: Rect, x: i32, y: i32) -> Hit {
    if !bar.contains(x, y) {
        return Hit::Outside;
    }
    let [close, minimize, zoom] = lights(bar);
    if close.contains(x, y) {
        Hit::Close
    } else if minimize.contains(x, y) {
        Hit::Minimize
    } else if zoom.contains(x, y) {
        Hit::Zoom
    } else {
        Hit::Drag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Rect = Rect::new(100, 200, 800, 600);

    #[test]
    fn third_party_apps_get_server_side_titlebars_and_our_own_apps_none() {
        assert_eq!(mode_for(Some("org.gimp.GIMP")), Mode::ServerSide);
        assert_eq!(mode_for(None), Mode::ServerSide);
        assert_eq!(mode_for(Some("is.olibuijr.bifrost.settings")), Mode::None);
        assert_eq!(mode_for(Some("is.olibuijr.bifrost.")), Mode::None);
        // a lookalike without the dot after the vendor is a third-party app
        assert_eq!(mode_for(Some("is.olibuijr.bifrostx.app")), Mode::ServerSide);
    }

    #[test]
    fn the_titlebar_sits_directly_above_the_frame() {
        assert_eq!(titlebar_of(FRAME), Rect::new(100, 200 - TITLEBAR_H, 800, TITLEBAR_H));
    }

    #[test]
    fn the_traffic_lights_are_spaced_and_centred_vertically() {
        let bar = titlebar_of(FRAME);
        let [close, minimize, zoom] = lights(bar);
        assert_eq!(close, Rect::new(112, 200 - 28 + 8, 12, 12));
        assert_eq!(minimize.x - close.x, LIGHT_D + LIGHT_GAP);
        assert_eq!(zoom.x - minimize.x, LIGHT_D + LIGHT_GAP);
        assert_eq!(close.y + close.h / 2, bar.y + bar.h / 2 - 0, "centred within one pixel");
    }

    #[test]
    fn the_title_is_centred_in_the_bar_outside_the_lights() {
        let bar = Rect::new(0, 0, 800, TITLEBAR_H);
        let t = title_rect(bar);
        let margin = LIGHT_LEFT + 3 * LIGHT_D + 2 * LIGHT_GAP;
        assert_eq!(t, Rect::new(margin, 0, 800 - 2 * margin, TITLEBAR_H));
        assert_eq!(t.x + t.w / 2, bar.x + bar.w / 2, "centred");
    }

    #[test]
    fn a_narrow_bar_gives_the_title_no_width_instead_of_a_negative_one() {
        assert_eq!(title_rect(Rect::new(0, 0, 40, TITLEBAR_H)).w, 0);
    }

    #[test]
    fn presses_on_the_lights_are_close_minimize_and_zoom() {
        let bar = titlebar_of(FRAME);
        let [close, minimize, zoom] = lights(bar);
        assert_eq!(hit(bar, close.x + 1, close.y + 1), Hit::Close);
        assert_eq!(hit(bar, minimize.x + 6, minimize.y + 6), Hit::Minimize);
        assert_eq!(hit(bar, zoom.x, zoom.y), Hit::Zoom);
    }

    #[test]
    fn the_rest_of_the_bar_drags_and_the_outside_is_outside() {
        let bar = titlebar_of(FRAME);
        assert_eq!(hit(bar, 500, bar.y + 5), Hit::Drag);
        assert_eq!(hit(bar, 500, bar.y + bar.h), Hit::Outside, "the bottom edge is exclusive");
        assert_eq!(hit(bar, 50, bar.y + 5), Hit::Outside);
    }

    #[test]
    fn the_light_gap_is_not_a_light() {
        let bar = titlebar_of(FRAME);
        let [close, minimize, _] = lights(bar);
        let gap_x = close.x + LIGHT_D + 2;
        assert!(gap_x < minimize.x);
        assert_eq!(hit(bar, gap_x, close.y + 1), Hit::Drag);
    }
}
