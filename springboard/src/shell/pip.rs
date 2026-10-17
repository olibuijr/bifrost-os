//! Picture in Picture in the shell: where the floating video window sits on the usable screen (the screen without the
//! menu bar and dock), and whether the window starts by itself when the app leaves the foreground. The life cycle and
//! its delegate events come from `bifrost_ui::pip_window`; the Settings toggle "Start PiP Automatically" (`pip_auto`)
//! gates the automatic start here, since the controller only reads the player's own opt-in.

use bifrost_ui::pip_window::{self, Controller, Corner, Event};
use bifrost_ui::Rect;

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
