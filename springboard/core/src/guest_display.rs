//! Showing a guest's framebuffer inside a window: where the picture sits, how window points map to guest pixels, and
//! when keyboard input goes to the guest.
//!
//! Pure model. [`fit`] places a `frame_w` x `frame_h` guest picture in a view of `view_w` x `view_h` pixels, keeping its
//! aspect ratio and centring it; [`to_guest`] maps a window point back into the picture. [`Capture`] decides whether
//! keys reach the guest: a click captures them, a release key or losing focus gives them back. The official
//! Virtualization documentation describes a view that displays and interacts with a guest's graphics in a window; this
//! module is the geometry and input rule for such a view, not a drawing surface. The release key is a caller-chosen code.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const EMPTY: Rect = Rect { x: 0, y: 0, w: 0, h: 0 };

    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && py >= self.y && px - self.x < self.w && py - self.y < self.h
    }
}

/// Places a `frame_w` x `frame_h` picture in a `view_w` x `view_h` view. The picture keeps its aspect ratio and is
/// centred. With `integer` set, the picture is shown at a whole-number scale when one fits (at least 1 times); if none
/// fits, the picture is scaled to fit instead. A zero size on either side gives [`Rect::EMPTY`].
pub fn fit(frame_w: u32, frame_h: u32, view_w: u32, view_h: u32, integer: bool) -> Rect {
    if frame_w == 0 || frame_h == 0 || view_w == 0 || view_h == 0 {
        return Rect::EMPTY;
    }
    let (fw, fh, vw, vh) = (frame_w as u64, frame_h as u64, view_w as u64, view_h as u64);
    let scale = (vw / fw).min(vh / fh);
    let (w, h) = if integer && scale >= 1 {
        (fw * scale, fh * scale)
    } else if vw * fh <= vh * fw {
        (vw, vw * fh / fw)
    } else {
        (vh * fw / fh, vh)
    };
    let (w, h) = (w as u32, h as u32);
    Rect { x: (view_w - w) / 2, y: (view_h - h) / 2, w, h }
}

/// Maps a window point to a guest pixel. `None` when the point is outside the picture.
pub fn to_guest(rect: Rect, frame_w: u32, frame_h: u32, px: u32, py: u32) -> Option<(u32, u32)> {
    if rect.w == 0 || rect.h == 0 || !rect.contains(px, py) {
        return None;
    }
    let gx = (px - rect.x) as u64 * frame_w as u64 / rect.w as u64;
    let gy = (py - rect.y) as u64 * frame_h as u64 / rect.h as u64;
    Some((gx as u32, gy as u32))
}

/// Where a key goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Guest,
    Host,
}

/// Keyboard capture for a guest view. Not captured at first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    release_key: u32,
    captured: bool,
}

impl Capture {
    pub fn new(release_key: u32) -> Self {
        Capture { release_key, captured: false }
    }

    pub fn is_captured(&self) -> bool {
        self.captured
    }

    /// A click in the view captures the keyboard. Returns true only when this click captured it.
    pub fn click(&mut self) -> bool {
        if self.captured {
            return false;
        }
        self.captured = true;
        true
    }

    /// Routes a key press or release. Pressing the release key while captured gives the keyboard back to the host.
    pub fn key(&mut self, code: u32, pressed: bool) -> Route {
        if !self.captured {
            return Route::Host;
        }
        if code == self.release_key && pressed {
            self.captured = false;
            return Route::Host;
        }
        Route::Guest
    }

    /// The window lost focus: input goes back to the host.
    pub fn focus_lost(&mut self) {
        self.captured = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_picture_letterboxes_top_and_bottom() {
        // 1920x1080 in 800x800: full width, height 450, centred vertically.
        let r = fit(1920, 1080, 800, 800, false);
        assert_eq!(r, Rect { x: 0, y: 175, w: 800, h: 450 });
    }

    #[test]
    fn integer_scale_is_used_when_it_fits() {
        // 640x480 in 1600x900: scale 1 (900/480 is 1), centred.
        let r = fit(640, 480, 1600, 900, true);
        assert_eq!(r, Rect { x: 480, y: 210, w: 640, h: 480 });
        // 1000x1000 in 2500x1500: scale 1 limited by the height.
        assert_eq!(fit(1000, 1000, 2500, 1500, true), Rect { x: 750, y: 250, w: 1000, h: 1000 });
    }

    #[test]
    fn integer_request_falls_back_to_fit_when_no_whole_scale_fits() {
        // 2000x1000 in 1500x800: scale 0, so the picture is scaled to 1500x750.
        let r = fit(2000, 1000, 1500, 800, true);
        assert_eq!(r, Rect { x: 0, y: 25, w: 1500, h: 750 });
    }

    #[test]
    fn window_point_maps_to_guest_pixel_and_outside_is_none() {
        let r = Rect { x: 480, y: 210, w: 640, h: 480 };
        assert_eq!(to_guest(r, 640, 480, 480, 210), Some((0, 0)));
        assert_eq!(to_guest(r, 640, 480, 1119, 689), Some((639, 479)));
        assert_eq!(to_guest(r, 640, 480, 479, 210), None);
        assert_eq!(to_guest(r, 640, 480, 1120, 300), None);
    }

    #[test]
    fn zero_sizes_give_an_empty_rect() {
        assert_eq!(fit(0, 480, 800, 600, false), Rect::EMPTY);
        assert_eq!(fit(640, 480, 0, 600, false), Rect::EMPTY);
        assert_eq!(to_guest(Rect::EMPTY, 640, 480, 0, 0), None);
    }

    #[test]
    fn click_captures_and_release_key_gives_keys_back() {
        let mut c = Capture::new(0x1d);
        assert_eq!(c.key(30, true), Route::Host);
        assert!(c.click());
        assert!(!c.click());
        assert_eq!(c.key(30, true), Route::Guest);
        assert_eq!(c.key(0x1d, false), Route::Guest);
        assert_eq!(c.key(0x1d, true), Route::Host);
        assert!(!c.is_captured());
        assert_eq!(c.key(30, true), Route::Host);
    }

    #[test]
    fn losing_focus_releases_capture() {
        let mut c = Capture::new(0x1d);
        c.click();
        c.focus_lost();
        assert!(!c.is_captured());
        assert!(c.click());
    }
}
