//! Picture in Picture as pure state: a floating video window that stays on top while the player is inline. The
//! controller decides when the window may start, stop or be suspended, and returns the delegate events an app would
//! receive. Placing the window on the screen is pure geometry: callers pass the usable screen area (without the menu
//! bar and dock) and get back the window rectangle. Nothing is drawn here.
use crate::Rect;

/// Gap between the floating window and the edges of the usable screen area, in points.
pub const EDGE_MARGIN: f32 = 16.0;
/// Narrowest floating window, in points.
pub const MIN_WIDTH: f32 = 240.0;
/// Widest floating window, in points.
pub const MAX_WIDTH: f32 = 480.0;
/// Aspect ratio (width / height) used when the video gives none usable.
pub const DEFAULT_ASPECT: f32 = 16.0 / 9.0;

/// Where the controller is in its life cycle. `Starting` and `Stopping` are the short hand-over steps between
/// `Inline` and `Active`, so the caller can finish them once the window has been put on screen or taken down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Inline,
    Starting,
    Active,
    Stopping,
}

/// Delegate events. `Will*` is sent before the window changes, `Did*` after it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    WillStart,
    DidStart,
    WillStop,
    DidStop,
}

/// Corner of the usable screen area that the floating window sits in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

/// The Picture in Picture state of one player.
#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    state: State,
    possible: bool,
    suspended: bool,
    /// When set, the viewer may not skip through the content while the window is up.
    pub requires_linear_playback: bool,
    /// When set, the window starts by itself when the app leaves the foreground during playback.
    pub start_automatically: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Controller::new()
    }
}

impl Controller {
    /// An inline controller that cannot start yet (the player has not reported that it can).
    pub fn new() -> Controller {
        Controller {
            state: State::Inline,
            possible: false,
            suspended: false,
            requires_linear_playback: false,
            start_automatically: false,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// True while the floating window is on screen.
    pub fn is_active(&self) -> bool {
        self.state == State::Active
    }

    /// True when the player reports that the window can be started now.
    pub fn is_possible(&self) -> bool {
        self.possible
    }

    /// True while the system has hidden the window; it comes back when the suspension ends.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// True when `stop` would take effect.
    pub fn can_stop(&self) -> bool {
        self.state == State::Active
    }

    /// The player reports whether the window can run. Turning this off while the window is up takes the window
    /// down at once, without the will-stop step, and returns `DidStop`.
    pub fn set_possible(&mut self, possible: bool) -> Option<Event> {
        self.possible = possible;
        if possible {
            return None;
        }
        match self.state {
            State::Active | State::Starting => {
                self.state = State::Inline;
                Some(Event::DidStop)
            }
            _ => None,
        }
    }

    /// The system hides (`true`) or shows (`false`) the window without ending it.
    pub fn suspend(&mut self, suspended: bool) {
        self.suspended = suspended;
    }

    /// Asks for the window. Only an inline controller that is possible and not suspended can start.
    pub fn start(&mut self) -> Option<Event> {
        if self.state != State::Inline || !self.possible || self.suspended {
            return None;
        }
        self.state = State::Starting;
        Some(Event::WillStart)
    }

    /// The window is on screen. Only a starting controller can finish starting.
    pub fn finish_start(&mut self) -> Option<Event> {
        if self.state != State::Starting {
            return None;
        }
        self.state = State::Active;
        Some(Event::DidStart)
    }

    /// Asks for the window to go away. Only an active window can stop.
    pub fn stop(&mut self) -> Option<Event> {
        if !self.can_stop() {
            return None;
        }
        self.state = State::Stopping;
        Some(Event::WillStop)
    }

    /// The window has been taken down. Only a stopping controller can finish stopping.
    pub fn finish_stop(&mut self) -> Option<Event> {
        if self.state != State::Stopping {
            return None;
        }
        self.state = State::Inline;
        Some(Event::DidStop)
    }

    /// The app left the foreground. Starts the window when automatic start is on, the player is playing and the
    /// controller is inline. Returns the same events as `start`.
    pub fn enter_background(&mut self, playing: bool) -> Option<Event> {
        if !self.start_automatically || !playing {
            return None;
        }
        self.start()
    }
}

/// Size of the floating window for a screen of `screen_w` by `screen_h` points and a video of `aspect` (width over
/// height). The width is a quarter of the screen clamped to the minimum and maximum width. The window never grows
/// past half the screen height, so it shrinks width-first when that would be too tall. A bad aspect uses 16:9.
pub fn window_size(screen_w: f32, screen_h: f32, aspect: f32) -> (f32, f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { DEFAULT_ASPECT };
    let screen_w = if screen_w.is_finite() { screen_w.max(0.0) } else { 0.0 };
    let screen_h = if screen_h.is_finite() { screen_h.max(0.0) } else { 0.0 };
    let mut w = (screen_w * 0.25).clamp(MIN_WIDTH, MAX_WIDTH);
    if w / aspect > screen_h * 0.5 {
        w = screen_h * 0.5 * aspect;
    }
    (w, w / aspect)
}

/// The window rectangle of `size` (width, height) set in `corner` of the usable screen area, `EDGE_MARGIN` in.
pub fn place(screen: Rect, size: (f32, f32), corner: Corner) -> Rect {
    let (w, h) = size;
    let left = screen.x + EDGE_MARGIN;
    let right = screen.x + screen.w - EDGE_MARGIN - w;
    let top = screen.y + EDGE_MARGIN;
    let bottom = screen.y + screen.h - EDGE_MARGIN - h;
    let (x, y) = match corner {
        Corner::TopLeft => (left, top),
        Corner::TopRight => (right, top),
        Corner::BottomLeft => (left, bottom),
        Corner::BottomRight => (right, bottom),
    };
    Rect { x, y, w, h }
}

/// The corner nearest to the point (`x`, `y`) when a dragged window is let go. A point exactly on the middle line
/// counts as the right or bottom half.
pub fn nearest_corner(screen: Rect, x: f32, y: f32) -> Corner {
    let right = x >= screen.x + screen.w * 0.5;
    let bottom = y >= screen.y + screen.h * 0.5;
    match (right, bottom) {
        (false, false) => Corner::TopLeft,
        (true, false) => Corner::TopRight,
        (false, true) => Corner::BottomLeft,
        (true, true) => Corner::BottomRight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }
    }

    #[test]
    fn window_starts_and_stops_in_order() {
        let mut c = Controller::new();
        assert_eq!(c.start(), None, "not possible yet");
        assert_eq!(c.set_possible(true), None);
        assert_eq!(c.start(), Some(Event::WillStart));
        assert_eq!(c.state(), State::Starting);
        assert!(!c.is_active());
        assert_eq!(c.finish_start(), Some(Event::DidStart));
        assert!(c.is_active() && c.can_stop());
        assert_eq!(c.stop(), Some(Event::WillStop));
        assert!(!c.can_stop(), "a stopping window cannot be stopped twice");
        assert_eq!(c.finish_stop(), Some(Event::DidStop));
        assert_eq!(c.state(), State::Inline);
    }

    #[test]
    fn start_is_refused_when_suspended_or_active() {
        let mut c = Controller::new();
        c.set_possible(true);
        c.suspend(true);
        assert!(c.is_suspended());
        assert_eq!(c.start(), None, "suspended by the system");
        c.suspend(false);
        assert!(c.start().is_some());
        assert_eq!(c.start(), None, "already starting");
        c.finish_start();
        assert_eq!(c.start(), None, "already active");
        assert_eq!(c.finish_start(), None, "finish needs a starting window");
    }

    #[test]
    fn losing_the_possibility_takes_the_window_down() {
        let mut c = Controller::new();
        c.set_possible(true);
        c.start();
        c.finish_start();
        assert_eq!(c.set_possible(false), Some(Event::DidStop));
        assert_eq!(c.state(), State::Inline);
        assert!(!c.is_possible());
        assert_eq!(c.set_possible(false), None, "nothing left to take down");
    }

    #[test]
    fn automatic_start_only_when_playing_in_the_background() {
        let mut c = Controller::new();
        c.set_possible(true);
        assert_eq!(c.enter_background(true), None, "automatic start is off");
        c.start_automatically = true;
        assert_eq!(c.enter_background(false), None, "paused");
        assert_eq!(c.enter_background(true), Some(Event::WillStart));
        assert_eq!(c.enter_background(true), None, "already starting");
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn window_size_clamps_width_and_keeps_aspect() {
        assert!(near(window_size(1920.0, 1080.0, 16.0 / 9.0), (480.0, 270.0)));
        assert!(near(window_size(800.0, 600.0, 16.0 / 9.0), (240.0, 135.0)), "a quarter is under the minimum");
        // A quarter of 1000 is 250, but 250 tall is above half of 400, so the width drops to 200.
        assert!(near(window_size(1000.0, 400.0, 1.0), (200.0, 200.0)));
        assert!(near(window_size(1920.0, 1080.0, f32::NAN), (480.0, 270.0)), "no usable aspect uses 16:9");
    }

    #[test]
    fn window_sits_in_the_chosen_corner_with_margin() {
        let size = (480.0, 270.0);
        assert_eq!(place(screen(), size, Corner::TopLeft), Rect { x: 16.0, y: 16.0, w: 480.0, h: 270.0 });
        assert_eq!(place(screen(), size, Corner::BottomRight), Rect { x: 1424.0, y: 794.0, w: 480.0, h: 270.0 });
        assert_eq!(place(screen(), size, Corner::TopRight).x, 1424.0);
        assert_eq!(place(screen(), size, Corner::BottomLeft).y, 794.0);
    }

    #[test]
    fn dropped_window_snaps_to_the_nearest_corner() {
        assert_eq!(nearest_corner(screen(), 100.0, 100.0), Corner::TopLeft);
        assert_eq!(nearest_corner(screen(), 1800.0, 100.0), Corner::TopRight);
        assert_eq!(nearest_corner(screen(), 100.0, 1000.0), Corner::BottomLeft);
        assert_eq!(nearest_corner(screen(), 960.0, 540.0), Corner::BottomRight, "the middle counts as right and bottom");
    }
}
