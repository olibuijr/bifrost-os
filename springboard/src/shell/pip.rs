//! Picture in Picture in the shell: where the floating video window sits on the usable screen (the screen without the
//! menu bar and dock), and whether the window starts by itself when the app leaves the foreground. The life cycle and
//! its delegate events come from `bifrost_ui::pip_window`; the Settings toggle "Start PiP Automatically" (`pip_auto`)
//! gates the automatic start here, since the controller only reads the player's own opt-in.

use super::music::icons;
use super::switcher::draw_scaled;
use super::ui::Ui;
use crate::gfx::{rgba, Image, Surface};
use bifrost_ui::pip_window::{self, Controller, Corner, Event, State};
use bifrost_ui::Rect;

/// The floating window's corner radius in pt: the grouped-card radius of the reference-style design.
pub const WINDOW_RADIUS_PT: f32 = 13.0;

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

/// The floating window's rectangle for a video of `aspect` (width over height) in `corner` of `usable`.
pub fn window_rect(usable: Rect, aspect: f32, corner: Corner) -> Rect {
    let size = pip_window::window_size(usable.w, usable.h, aspect);
    pip_window::place(usable, size, corner)
}

/// Draws the floating window at `r`: a black rounded frame with the video `frame` scaled to fill it (a placeholder
/// note while there is no frame yet) and a thin light edge. Nothing is drawn outside `r`.
pub fn draw_window(s: &mut Surface, ui: &Ui, r: Rect, frame: Option<&Image>) {
    let radius = ui.pt(WINDOW_RADIUS_PT);
    s.fill_rrect(r.x, r.y, r.w, r.h, radius, rgba(0, 0, 0, 255));
    let slot = crate::layout::Rect { x: r.x as i32, y: r.y as i32, w: r.w as i32, h: r.h as i32 };
    match frame {
        Some(img) => draw_scaled(s, img, slot, radius, 255),
        None => draw_scaled(s, &icons::placeholder(64, 0.0), slot, radius, 255),
    }
    s.stroke_rrect(r.x, r.y, r.w, r.h, radius, 1.0, rgba(255, 255, 255, 60));
}

/// The largest frame a player may send, in pixels (full HD).
pub const MAX_FRAME_PIXELS: usize = 1920 * 1080;

/// A frame from `w` by `h` pixels of 8-bit RGBA bytes (red first, alpha last, premultiplied; a video frame is
/// opaque). Refuses an empty or oversized frame and a byte count that does not match.
pub fn frame_from_rgba(w: usize, h: usize, bytes: &[u8]) -> Result<Image, String> {
    let pixels = w.checked_mul(h).filter(|n| (1..=MAX_FRAME_PIXELS).contains(n)).ok_or_else(|| format!("pip: a {w}x{h} frame is empty or larger than {MAX_FRAME_PIXELS} pixels"))?;
    if bytes.len() != pixels * 4 {
        return Err(format!("pip: a {w}x{h} frame needs {} bytes, got {}", pixels * 4, bytes.len()));
    }
    let px = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok(Image { w, h, px })
}

/// The Picture in Picture window the shell holds: the controller's life cycle, the corner it rests in and the
/// video's aspect ratio (width over height; a bad value uses 16:9). `Shell::draw` paints it over the screen while it
/// is starting or active.
#[derive(Debug, Default)]
pub struct Floating {
    pub controller: Controller,
    pub corner: Corner,
    pub aspect: f32,
    /// The latest video frame the player sent (`pip` service, `frame`); `None` draws the placeholder.
    pub frame: Option<Image>,
}

impl Floating {
    /// Whether the window is on screen (starting or active).
    pub fn showing(&self) -> bool {
        matches!(self.controller.state(), State::Starting | State::Active)
    }

    /// Starts the window and finishes the hand-over at once: the shell paints it on the next frame. Returns the
    /// delegate events in order.
    pub fn start(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        events.extend(self.controller.start());
        events.extend(self.controller.finish_start());
        events
    }

    /// Takes the window down, finishing the hand-over at once. Returns the delegate events in order.
    pub fn stop(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        events.extend(self.controller.stop());
        events.extend(self.controller.finish_stop());
        self.frame = None;
        events
    }

    /// Keeps `frame` as the picture the window shows, and sizes the window to its aspect ratio.
    pub fn set_frame(&mut self, frame: Image) {
        self.aspect = frame.w as f32 / frame.h.max(1) as f32;
        self.frame = Some(frame);
    }

    /// The window's rectangle inside `usable` (the screen without the menu bar and the dock).
    pub fn rect(&self, usable: Rect) -> Rect {
        window_rect(usable, self.aspect, self.corner)
    }
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
    fn a_frame_decodes_from_rgba_bytes_and_bad_sizes_are_refused() {
        let img = frame_from_rgba(2, 1, &[1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        assert_eq!((img.w, img.h), (2, 1));
        assert_eq!(img.px[0], rgba(1, 2, 3, 255), "red is the low byte");
        assert!(frame_from_rgba(2, 1, &[0; 7]).is_err(), "the byte count must match");
        assert!(frame_from_rgba(0, 4, &[]).is_err(), "an empty frame");
        assert!(frame_from_rgba(usize::MAX, 2, &[]).is_err(), "no overflow");
        assert!(frame_from_rgba(4000, 4000, &[]).is_err(), "larger than the limit");
    }

    #[test]
    fn a_frame_sets_the_window_aspect_and_stopping_drops_it() {
        let mut f = Floating::default();
        f.set_frame(frame_from_rgba(4, 2, &[0; 32]).unwrap());
        assert_eq!(f.aspect, 2.0);
        assert!(f.frame.is_some());
        f.stop();
        assert!(f.frame.is_none(), "the next window starts from the placeholder");
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
    fn the_window_draws_inside_its_rectangle_only() {
        let ui = crate::shell::tests::ui();
        let (w, h) = (400usize, 300usize);
        let mut px = vec![0u32; w * h];
        {
            let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
            draw_window(&mut s, &ui, Rect { x: 100.0, y: 100.0, w: 200.0, h: 150.0 }, None);
        }
        assert_eq!(px[0], 0, "the corner of the screen is untouched");
        assert_ne!(px[175 * w + 200], 0, "the middle of the window is drawn");
        assert_eq!(px[299 * w + 399], 0, "the far corner is untouched");
    }

    #[test]
    fn a_started_window_is_showing_until_it_stops() {
        let mut f = Floating { aspect: 16.0 / 9.0, ..Default::default() };
        assert!(!f.showing());
        assert!(f.start().is_empty(), "an impossible player cannot start");
        f.controller.set_possible(true);
        assert_eq!(f.start(), vec![Event::WillStart, Event::DidStart]);
        assert!(f.showing());
        assert_eq!(f.rect(usable()).w, 300.0, "placed by the shared geometry");
        assert_eq!(f.stop(), vec![Event::WillStop, Event::DidStop]);
        assert!(!f.showing());
    }

    #[test]
    fn a_paused_player_does_not_start_in_the_background() {
        let mut c = player();
        assert_eq!(enter_background(true, &mut c, false), None);
    }

    #[test]
    fn the_window_is_clipped_to_the_surface() {
        assert_eq!(clip_to(Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 }, 800, 600), Some((10, 20, 100, 50)));
        assert_eq!(clip_to(Rect { x: 750.0, y: 560.0, w: 100.0, h: 100.0 }, 800, 600), Some((750, 560, 50, 40)));
        assert_eq!(clip_to(Rect { x: -20.0, y: -10.0, w: 100.0, h: 50.0 }, 800, 600), Some((0, 0, 80, 40)));
        assert_eq!(clip_to(Rect { x: 900.0, y: 0.0, w: 100.0, h: 50.0 }, 800, 600), None, "off the right edge");
        assert_eq!(clip_to(Rect { x: 0.0, y: 0.0, w: 0.0, h: 50.0 }, 800, 600), None, "an empty window draws nothing");
    }
}
