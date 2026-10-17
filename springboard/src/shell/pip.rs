//! Picture in Picture in the shell: where the floating video window sits on the usable screen (the screen without the
//! menu bar and dock), and whether the window starts by itself when the app leaves the foreground. The life cycle and
//! its delegate events come from `bifrost_ui::pip_window`; the Settings toggle "Start PiP Automatically" (`pip_auto`)
//! gates the automatic start here, since the controller only reads the player's own opt-in.

use crate::gfx::{Image, Surface};
use bifrost_ui::pip_window::{self, Controller, Corner, Event};
use bifrost_ui::Rect;

/// The window's backing colour behind the video: opaque black.
const BACKING: u32 = 0xFF00_0000;

/// The part of `rect` that lies on a `w` by `h` surface, as `(x, y, width, height)` in pixels, or `None` when nothing
/// of it is on the surface. Fractions are rounded outward so no edge pixel is lost.
pub fn clip_to(rect: Rect, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let x0 = rect.x.floor().max(0.0);
    let y0 = rect.y.floor().max(0.0);
    let x1 = (rect.x + rect.w).ceil().min(w as f32);
    let y1 = (rect.y + rect.h).ceil().min(h as f32);
    if !(x0 < x1 && y0 < y1) {
        return None;
    }
    Some((x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize))
}

/// Draws the floating window on top of whatever the shell has drawn: a black backing over `rect`, then `frame` when
/// the player has one. The frame must already be scaled to the window's size; this does not scale it. Called last in
/// the frame so the window sits above the windows, the menu bar and the dock.
pub fn draw_window(s: &mut Surface, rect: Rect, frame: Option<&Image>) {
    let Some((x, y, cw, ch)) = clip_to(rect, s.w, s.h) else { return };
    s.fill_rect(x, y, cw, ch, BACKING);
    if let Some(img) = frame {
        s.blit(img, rect.x as i32, rect.y as i32, 0xFFFF_FFFF, 0xFF);
    }
}

/// The floating window's rectangle for a video of `aspect` (width over height) in `corner` of `usable`.
pub fn window_rect(usable: Rect, aspect: f32, corner: Corner) -> Rect {
    let size = pip_window::window_size(usable.w, usable.h, aspect);
    pip_window::place(usable, size, corner)
}

/// The app left the foreground. Starts the window when the Settings toggle is on and the controller allows it (see
/// [`Controller::enter_background`]). Returns the events the app receives.
pub fn enter_background(pref_auto: bool, controller: &mut Controller, playing: bool) -> Option<Event> {
    if !pref_auto {
        return None;
    }
    controller.enter_background(playing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::pip_window::State;

    fn usable() -> Rect {
        Rect { x: 0.0, y: 24.0, w: 1200.0, h: 800.0 }
    }

    fn player() -> Controller {
        let mut c = Controller::new();
        c.set_possible(true);
        c.start_automatically = true;
        c
    }

    #[test]
    fn the_window_sits_in_its_corner_inside_the_usable_area() {
        let r = window_rect(usable(), 16.0 / 9.0, Corner::BottomRight);
        assert_eq!(r.w, 300.0);
        assert!(r.x + r.w <= usable().x + usable().w);
        assert!(r.y + r.h <= usable().y + usable().h, "clear of the dock at the bottom");
        let tl = window_rect(usable(), 16.0 / 9.0, Corner::TopLeft);
        assert_eq!(tl.x, usable().x + pip_window::EDGE_MARGIN);
    }

    #[test]
    fn the_window_is_clipped_to_the_surface() {
        assert_eq!(clip_to(Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 }, 800, 600), Some((10, 20, 100, 50)));
        assert_eq!(clip_to(Rect { x: 750.0, y: 560.0, w: 100.0, h: 100.0 }, 800, 600), Some((750, 560, 50, 40)));
        assert_eq!(clip_to(Rect { x: -20.0, y: -10.0, w: 100.0, h: 50.0 }, 800, 600), Some((0, 0, 80, 40)));
        assert_eq!(clip_to(Rect { x: 900.0, y: 0.0, w: 100.0, h: 50.0 }, 800, 600), None, "off the right edge");
        assert_eq!(clip_to(Rect { x: 0.0, y: 0.0, w: 0.0, h: 50.0 }, 800, 600), None, "an empty window draws nothing");
    }

    #[test]
    fn the_settings_toggle_gates_the_automatic_start() {
        let mut off = player();
        assert_eq!(enter_background(false, &mut off, true), None, "toggle off: no window");
        assert_eq!(off.state(), State::Inline);

        let mut on = player();
        assert_eq!(enter_background(true, &mut on, true), Some(Event::WillStart));
        assert_eq!(on.state(), State::Starting);
    }

    #[test]
    fn a_paused_player_does_not_start_in_the_background() {
        let mut c = player();
        assert_eq!(enter_background(true, &mut c, false), None);
    }
}
