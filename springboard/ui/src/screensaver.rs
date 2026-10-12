//! Screen saver module model (`ScreenSaverView`) and its utilities (`SSRandomIntBetween`, `SSRandomFloatBetween`,
//! `SSRandomPointForSizeWithinRect`, `SSCenteredRectInRect`). The view is a pure state machine: the caller's clock
//! drives the frame timer, so the lifecycle is testable without a window or a screen saver engine. Nothing here loads a
//! module bundle, draws, or reads idle state. Random utilities take unit values in [0, 1) that the caller samples, so
//! the generator and its seed stay with the caller.

use crate::Rect;

/// Default time between frames in seconds (30 frames a second).
pub const DEFAULT_FRAME_INTERVAL: f64 = 1.0 / 30.0;
/// The shortest accepted frame interval in seconds (120 frames a second).
pub const MIN_FRAME_INTERVAL: f64 = 1.0 / 120.0;
/// Most frames one `tick` reports after a stall. Frames past this are dropped, so a long pause cannot queue a burst.
pub const MAX_CATCH_UP_FRAMES: u32 = 4;

/// The animation state of a screen saver view (`startAnimation`, `stopAnimation`, `isAnimating`,
/// `animationTimeInterval`, `animateOneFrame`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenSaverView {
    frame: Rect,
    is_preview: bool,
    interval: f64,
    animating: bool,
    next_frame_at: f64,
    frames: u64,
}

impl ScreenSaverView {
    /// `init(frame:isPreview:)`: a view that is not animating yet. `is_preview` is set when the view shows a small
    /// preview thumbnail instead of the full screen.
    pub fn new(frame: Rect, is_preview: bool) -> Self {
        ScreenSaverView {
            frame,
            is_preview,
            interval: DEFAULT_FRAME_INTERVAL,
            animating: false,
            next_frame_at: 0.0,
            frames: 0,
        }
    }

    pub fn frame(&self) -> Rect {
        self.frame
    }

    /// `isPreview`.
    pub fn is_preview(&self) -> bool {
        self.is_preview
    }

    /// `isAnimating`.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// `animationTimeInterval` in seconds.
    pub fn animation_time_interval(&self) -> f64 {
        self.interval
    }

    /// Frames advanced by `animate_one_frame` since the view was made.
    pub fn frames_animated(&self) -> u64 {
        self.frames
    }

    /// Sets `animationTimeInterval`. A value that is not finite or not positive keeps the default, and a value shorter
    /// than `MIN_FRAME_INTERVAL` is raised to it. The new interval applies from the next scheduled frame.
    pub fn set_animation_time_interval(&mut self, secs: f64) {
        self.interval = if !secs.is_finite() || secs <= 0.0 {
            DEFAULT_FRAME_INTERVAL
        } else {
            secs.max(MIN_FRAME_INTERVAL)
        };
    }

    /// `startAnimation`: starts the frame timer, with the first frame one interval after `now`. Calling it while
    /// already animating changes nothing.
    pub fn start_animation(&mut self, now: f64) {
        if self.animating {
            return;
        }
        self.animating = true;
        self.next_frame_at = now + self.interval;
    }

    /// `stopAnimation`: stops the frame timer. No frame is due until the next `start_animation`.
    pub fn stop_animation(&mut self) {
        self.animating = false;
    }

    /// Returns how many frames the caller should advance at time `now` and moves the schedule past them. Zero while
    /// stopped or before the next frame. After a stall, at most `MAX_CATCH_UP_FRAMES` are reported and the schedule
    /// restarts one interval after `now`.
    pub fn tick(&mut self, now: f64) -> u32 {
        if !self.animating || now < self.next_frame_at {
            return 0;
        }
        let due = ((now - self.next_frame_at) / self.interval).floor() as u64 + 1;
        if due > MAX_CATCH_UP_FRAMES as u64 {
            self.next_frame_at = now + self.interval;
            MAX_CATCH_UP_FRAMES
        } else {
            self.next_frame_at += due as f64 * self.interval;
            due as u32
        }
    }

    /// `animateOneFrame`: advances the animation by one frame. The system may still call it after `stop_animation`,
    /// so it does not check the timer.
    pub fn animate_one_frame(&mut self) {
        self.frames += 1;
    }
}

/// Maps a unit value to [0, 1). A NaN becomes 0 and values above 1 are clamped below 1.
fn unit(u: f64) -> f64 {
    if u.is_nan() {
        0.0
    } else {
        u.clamp(0.0, 1.0 - f64::EPSILON)
    }
}

/// `SSRandomIntBetween`: an integer in the closed range between the two bounds, order not mattering. `u` is a unit
/// value in [0, 1); each integer gets an equal share of the unit range.
pub fn ss_random_int_between(u: f64, a: i32, b: i32) -> i32 {
    let lo = a.min(b) as i64;
    let hi = a.max(b) as i64;
    let span = (hi - lo + 1) as f64;
    let k = (unit(u) * span).floor() as i64;
    (lo + k) as i32
}

/// `SSRandomFloatBetween`: a float in the closed range between the two bounds, order not mattering.
pub fn ss_random_float_between(u: f64, a: f32, b: f32) -> f32 {
    let lo = a.min(b) as f64;
    let hi = a.max(b) as f64;
    (lo + unit(u) * (hi - lo)) as f32
}

/// `SSRandomPointForSizeWithinRect`: a point at which a rectangle of `size` (width, height) fits entirely inside
/// `rect`. `ux` and `uy` are unit values for the horizontal and vertical choice. When the size does not fit, the
/// point is at the rectangle's origin on that axis.
pub fn ss_random_point_for_size_within_rect(ux: f64, uy: f64, size: (f32, f32), rect: Rect) -> (f32, f32) {
    let room_x = (rect.w - size.0).max(0.0) as f64;
    let room_y = (rect.h - size.1).max(0.0) as f64;
    (
        rect.x + (unit(ux) * room_x) as f32,
        rect.y + (unit(uy) * room_y) as f32,
    )
}

/// `SSCenteredRectInRect`: a rectangle with the size of `inner` (its origin is ignored) centred in `outer`. The
/// argument order follows the official signature, with the rectangle to centre first; the reference gives no more
/// detail. When `inner` is larger than `outer`, the result is still centred and overhangs on both sides.
pub fn ss_centered_rect_in_rect(inner: Rect, outer: Rect) -> Rect {
    Rect {
        x: outer.x + (outer.w - inner.w) / 2.0,
        y: outer.y + (outer.h - inner.h) / 2.0,
        w: inner.w,
        h: inner.h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn int_range_is_closed_and_order_free() {
        // Range 1..=6 has six values: unit 0.0 gives 1, unit 0.5 gives floor(3.0)=3 offset to 4, unit ~1 gives 6.
        assert_eq!(ss_random_int_between(0.0, 1, 6), 1);
        assert_eq!(ss_random_int_between(0.5, 1, 6), 4);
        assert_eq!(ss_random_int_between(0.999_999, 1, 6), 6);
        assert_eq!(ss_random_int_between(0.5, 6, 1), 4);
        // A single-value range always gives that value.
        assert_eq!(ss_random_int_between(0.7, 9, 9), 9);
    }

    #[test]
    fn float_range_scales_between_bounds() {
        assert_eq!(ss_random_float_between(0.0, 2.0, 4.0), 2.0);
        assert_eq!(ss_random_float_between(0.25, 4.0, 2.0), 2.5);
        assert_eq!(ss_random_float_between(f64::NAN, 2.0, 4.0), 2.0);
    }

    #[test]
    fn point_fits_size_inside_rect() {
        let rect = Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        // Room is 60 across and 40 down, so half of each gives an offset of 30 and 20.
        assert_eq!(ss_random_point_for_size_within_rect(0.5, 0.5, (40.0, 10.0), rect), (40.0, 40.0));
        assert_eq!(ss_random_point_for_size_within_rect(0.0, 0.0, (40.0, 10.0), rect), (10.0, 20.0));
        // A size wider than the rect pins that axis to the origin.
        assert_eq!(ss_random_point_for_size_within_rect(0.9, 0.5, (200.0, 10.0), rect), (10.0, 40.0));
    }

    #[test]
    fn centred_rect_keeps_size_and_centres() {
        let inner = Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 };
        let outer = Rect { x: 100.0, y: 100.0, w: 200.0, h: 100.0 };
        // x = 100 + (200 - 40) / 2 = 180, y = 100 + (100 - 20) / 2 = 140.
        assert_eq!(ss_centered_rect_in_rect(inner, outer), Rect { x: 180.0, y: 140.0, w: 40.0, h: 20.0 });
    }

    #[test]
    fn view_starts_on_the_interval_and_counts_frames() {
        let mut v = ScreenSaverView::new(Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }, false);
        assert!(!v.is_animating());
        assert!(near(v.animation_time_interval(), 1.0 / 30.0));
        v.start_animation(0.0);
        assert!(v.is_animating());
        // Nothing is due before the first interval (1/30 s).
        assert_eq!(v.tick(0.02), 0);
        // One frame is due at 1/30 s, and the next is due at 2/30 s.
        assert_eq!(v.tick(0.034), 1);
        assert_eq!(v.tick(0.05), 0);
        assert_eq!(v.tick(0.07), 1);
        v.animate_one_frame();
        v.animate_one_frame();
        assert_eq!(v.frames_animated(), 2);
    }

    #[test]
    fn long_stall_reports_at_most_four_frames() {
        let mut v = ScreenSaverView::new(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, true);
        assert!(v.is_preview());
        v.start_animation(0.0);
        // About 14 frames are due after half a second; only the cap of 4 is reported.
        assert_eq!(v.tick(0.5), MAX_CATCH_UP_FRAMES);
        // The schedule restarts one interval after the stall, so the next tick right away reports nothing.
        assert_eq!(v.tick(0.5), 0);
        assert!(v.is_animating());
    }

    #[test]
    fn stop_halts_ticks_but_manual_frames_still_advance() {
        let mut v = ScreenSaverView::new(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, false);
        v.start_animation(0.0);
        v.stop_animation();
        assert!(!v.is_animating());
        assert_eq!(v.tick(1.0), 0);
        v.animate_one_frame();
        assert_eq!(v.frames_animated(), 1);
        // A second start works from the new time.
        v.start_animation(2.0);
        assert_eq!(v.tick(2.0 + 1.0 / 30.0), 1);
    }

    #[test]
    fn interval_is_validated_and_clamped() {
        let mut v = ScreenSaverView::new(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, false);
        v.set_animation_time_interval(0.0001);
        assert!(near(v.animation_time_interval(), MIN_FRAME_INTERVAL));
        v.set_animation_time_interval(f64::NAN);
        assert!(near(v.animation_time_interval(), DEFAULT_FRAME_INTERVAL));
        v.set_animation_time_interval(0.1);
        assert!(near(v.animation_time_interval(), 0.1));
    }
}
