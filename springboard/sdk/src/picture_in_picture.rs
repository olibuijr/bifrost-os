//! Picture in Picture for apps: a floating video window that stays on top while the player is inline. The life cycle
//! and delegate events come from `bifrost_ui::pip_window::Controller`; this wrapper adds the window rectangle for the
//! usable screen (without the menu bar and dock) and the corner the window snaps to. Nothing is drawn here: the shell
//! places the window with [`Pip::window`] and the app draws the video into it.

pub use bifrost_ui::pip_window::{Corner, Event, State};

use bifrost_ui::pip_window::{nearest_corner, place, window_size, Controller};
use bifrost_ui::Rect;

/// The Picture in Picture state of one player and where its window sits.
#[derive(Clone, Debug, PartialEq)]
pub struct Pip {
    controller: Controller,
    /// Width over height of the video.
    aspect: f32,
    corner: Corner,
}

impl Pip {
    /// A player with the given video aspect (width over height). The window starts in the bottom right corner.
    pub fn new(aspect: f32) -> Pip {
        Pip { controller: Controller::new(), aspect, corner: Corner::default() }
    }

    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    /// The controller, for the life-cycle calls (`set_possible`, `start`, `finish_start`, `stop`, ...).
    pub fn controller_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }

    pub fn corner(&self) -> Corner {
        self.corner
    }

    /// Sets the corner the window snaps to.
    pub fn set_corner(&mut self, corner: Corner) {
        self.corner = corner;
    }

    /// Snaps to the corner nearest the point where a dragged window was let go.
    pub fn drop_at(&mut self, screen: Rect, x: f32, y: f32) {
        self.corner = nearest_corner(screen, x, y);
    }

    /// The window rectangle on `screen` (the usable area) in the current corner.
    pub fn window(&self, screen: Rect) -> Rect {
        let size = window_size(screen.w, screen.h, self.aspect);
        place(screen, size, self.corner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect { x: 0.0, y: 24.0, w: 1200.0, h: 800.0 }
    }

    #[test]
    fn the_window_sits_in_the_default_corner_with_the_video_aspect() {
        let pip = Pip::new(16.0 / 9.0);
        let w = pip.window(screen());
        assert_eq!(w.w, 300.0, "a quarter of the screen width");
        assert!((w.h - 300.0 * 9.0 / 16.0).abs() < 0.01, "the video's height");
        assert!(w.x + w.w <= screen().x + screen().w, "inside the right edge");
        assert!(w.y + w.h <= screen().y + screen().h, "inside the bottom edge");
    }

    #[test]
    fn a_dropped_window_snaps_to_the_nearest_corner() {
        let mut pip = Pip::new(16.0 / 9.0);
        pip.drop_at(screen(), 10.0, 40.0);
        assert_eq!(pip.corner(), Corner::TopLeft);
        let top_left = pip.window(screen());
        assert_eq!(top_left.x, screen().x + bifrost_ui::pip_window::EDGE_MARGIN);
        assert_eq!(top_left.y, screen().y + bifrost_ui::pip_window::EDGE_MARGIN);
    }

    #[test]
    fn a_bad_aspect_falls_back_to_sixteen_by_nine() {
        let bad = Pip::new(f32::NAN).window(screen());
        let good = Pip::new(16.0 / 9.0).window(screen());
        assert_eq!(bad, good);
    }

    #[test]
    fn the_controller_starts_only_when_possible() {
        let mut pip = Pip::new(1.0);
        assert_eq!(pip.controller_mut().start(), None, "not possible yet");
        pip.controller_mut().set_possible(true);
        assert_eq!(pip.controller_mut().start(), Some(Event::WillStart));
        assert_eq!(pip.controller().state(), State::Starting);
    }
}
