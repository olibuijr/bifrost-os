//! App-facing screen saver surface. The `ScreenSaverView` lifecycle, the `SSRandom...` and `SSCenteredRectInRect`
//! helpers, the configuration sheet options and the preview fit are modelled in `bifrost_ui::screensaver`. This
//! module gives them an SDK path and adds [`SaverRunner`], which advances a view from the caller's clock. The
//! preference store for a module is [`crate::screen_saver_defaults::ScreenSaverDefaults`].
//!
//! [`SaverManifest`] reads a module's text descriptor (name, interval, fade, backing store and options). Nothing here
//! loads a saver's code from a `.saver` bundle or draws a frame. The caller owns the clock and the drawing.

pub use bifrost_ui::screensaver::{
    preview_rect, ss_centered_rect_in_rect, ss_random_float_between, ss_random_int_between,
    ss_random_point_for_size_within_rect, BackingStoreType, ConfigureSheet, SaverValue, ScreenSaverView,
    DEFAULT_FRAME_INTERVAL, MAX_CATCH_UP_FRAMES, MIN_FRAME_INTERVAL,
};
pub use bifrost_ui::saver_random::SaverRandom;
pub use bifrost_ui::screensaver_manifest::{ManifestError, SaverManifest};
pub use bifrost_ui::Rect;

/// Drives one [`ScreenSaverView`] from a caller's clock in seconds. `advance` reports the frames due at that time and
/// counts each one with `animate_one_frame`, so `frames_animated` matches the frames the caller has drawn.
#[derive(Debug, Clone)]
pub struct SaverRunner {
    view: ScreenSaverView,
}

impl SaverRunner {
    /// A runner for a view with the given frame. The view is not animating until [`SaverRunner::start`].
    pub fn new(frame: Rect, is_preview: bool) -> Self {
        SaverRunner { view: ScreenSaverView::new(frame, is_preview) }
    }

    /// `startAnimation`: the first frame is due one interval after `now`.
    pub fn start(&mut self, now: f64) {
        self.view.start_animation(now);
    }

    /// `stopAnimation`: no frame is due until the next `start`.
    pub fn stop(&mut self) {
        self.view.stop_animation();
    }

    /// Advances the animation for the frames due at `now` and returns how many were advanced.
    pub fn advance(&mut self, now: f64) -> u32 {
        let due = self.view.tick(now);
        for _ in 0..due {
            self.view.animate_one_frame();
        }
        due
    }

    pub fn view(&self) -> &ScreenSaverView {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut ScreenSaverView {
        &mut self.view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }
    }

    #[test]
    fn nothing_is_due_before_the_first_interval() {
        let mut r = SaverRunner::new(frame(), false);
        r.start(0.0);
        assert_eq!(r.advance(DEFAULT_FRAME_INTERVAL / 2.0), 0);
        assert_eq!(r.view().frames_animated(), 0);
    }

    #[test]
    fn one_frame_is_due_at_the_first_interval() {
        let mut r = SaverRunner::new(frame(), false);
        r.start(0.0);
        assert_eq!(r.advance(DEFAULT_FRAME_INTERVAL), 1);
        assert_eq!(r.view().frames_animated(), 1);
    }

    #[test]
    fn stopped_runner_advances_nothing() {
        let mut r = SaverRunner::new(frame(), false);
        r.start(0.0);
        r.stop();
        assert_eq!(r.advance(10.0), 0);
        assert!(!r.view().is_animating());
    }

    #[test]
    fn a_long_stall_is_capped_at_the_catch_up_limit() {
        let mut r = SaverRunner::new(frame(), false);
        r.start(0.0);
        assert_eq!(r.advance(60.0), MAX_CATCH_UP_FRAMES);
        assert_eq!(r.view().frames_animated(), MAX_CATCH_UP_FRAMES as u64);
    }

    #[test]
    fn preview_flag_is_kept() {
        let r = SaverRunner::new(frame(), true);
        assert!(r.view().is_preview());
    }

    #[test]
    fn centred_helper_is_reexported_with_the_same_result() {
        let inner = Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let outer = Rect { x: 0.0, y: 0.0, w: 300.0, h: 150.0 };
        let c = ss_centered_rect_in_rect(inner, outer);
        assert_eq!((c.x, c.y), (100.0, 50.0));
    }
}
