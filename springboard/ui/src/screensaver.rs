//! Screen saver module model (`ScreenSaverView`) and its utilities (`SSRandomIntBetween`, `SSRandomFloatBetween`,
//! `SSRandomPointForSizeWithinRect`, `SSCenteredRectInRect`). The view is a pure state machine: the caller's clock
//! drives the frame timer, so the lifecycle is testable without a window or a screen saver engine. Nothing here loads a
//! module bundle, draws, or reads idle state. Random utilities take unit values in [0, 1) that the caller samples, so
//! the generator and its seed stay with the caller. The view also keeps the preferred backing store, the gamma fade
//! request and the configuration sheet flag (`ConfigureSheet` holds the saver's options and checks their values). The
//! gamma ramp itself is in `render/src/gamma_fade.rs`; `preview_rect` fits a screen into a preview thumbnail.

use crate::Rect;

/// The backing store a saver window asks for (`backingStoreType()`). The shell draws every window into a buffer, so
/// `Buffered` is the default; the other two are kept as the view's stated preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackingStoreType {
    Retained,
    Nonretained,
    #[default]
    Buffered,
}

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
    backing_store: BackingStoreType,
    gamma_fade: bool,
    configure_sheet: bool,
}

impl ScreenSaverView {
    /// `init(frame:isPreview:)`: a view that is not animating yet. `is_preview` is set when the view shows a small
    /// preview thumbnail instead of the full screen. The backing store is `Buffered`, no gamma fade is requested and
    /// the view has no configuration sheet until the module sets one.
    pub fn new(frame: Rect, is_preview: bool) -> Self {
        ScreenSaverView {
            frame,
            is_preview,
            interval: DEFAULT_FRAME_INTERVAL,
            animating: false,
            next_frame_at: 0.0,
            frames: 0,
            backing_store: BackingStoreType::Buffered,
            gamma_fade: false,
            configure_sheet: false,
        }
    }

    pub fn frame(&self) -> Rect {
        self.frame
    }

    /// `isPreview`.
    pub fn is_preview(&self) -> bool {
        self.is_preview
    }

    /// `backingStoreType()`: the backing store the saver window asks for.
    pub fn backing_store_type(&self) -> BackingStoreType {
        self.backing_store
    }

    /// Sets the backing store the module asks for (the override of `backingStoreType()`).
    pub fn set_backing_store_type(&mut self, kind: BackingStoreType) {
        self.backing_store = kind;
    }

    /// `performGammaFade()`: whether the screen fades gradually when the animation starts and stops. The reference
    /// page gives no default, so the model starts with `false`.
    pub fn perform_gamma_fade(&self) -> bool {
        self.gamma_fade
    }

    /// Sets the gamma fade request (the override of `performGammaFade()`).
    pub fn set_perform_gamma_fade(&mut self, on: bool) {
        self.gamma_fade = on;
    }

    /// `hasConfigureSheet`: true when the module ships a configuration sheet.
    pub fn has_configure_sheet(&self) -> bool {
        self.configure_sheet
    }

    /// Sets the configuration sheet flag (the override of `hasConfigureSheet`).
    pub fn set_has_configure_sheet(&mut self, on: bool) {
        self.configure_sheet = on;
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

/// Fits a screen's frame into a preview thumbnail: the largest rectangle with the screen's aspect that fits in `thumb`,
/// centred in it. A screen or thumbnail with no area gives an empty rectangle centred in the thumbnail.
pub fn preview_rect(screen: Rect, thumb: Rect) -> Rect {
    if screen.w <= 0.0 || screen.h <= 0.0 || thumb.w <= 0.0 || thumb.h <= 0.0 {
        return ss_centered_rect_in_rect(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, thumb);
    }
    let scale = (thumb.w / screen.w).min(thumb.h / screen.h);
    let fitted = Rect { x: 0.0, y: 0.0, w: screen.w * scale, h: screen.h * scale };
    ss_centered_rect_in_rect(fitted, thumb)
}

/// The value an option in a configuration sheet holds.
#[derive(Clone, Debug, PartialEq)]
pub enum SaverValue {
    Bool(bool),
    Int(i64),
    Choice(String),
}

#[derive(Clone, Debug, PartialEq)]
enum OptionKind {
    Bool,
    Int { min: i64, max: i64 },
    Choice(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
struct SheetOption {
    key: String,
    kind: OptionKind,
    value: SaverValue,
}

/// The options a saver's configuration sheet edits (the controls behind `configureSheet`). Each option has a key, a kind
/// (on or off, a whole number in a closed range, or one of a list of choices) and a current value. `set` checks the
/// value, so the sheet never holds one the saver does not accept. Saving is the caller's job: write `values` to the
/// module's preferences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigureSheet {
    options: Vec<SheetOption>,
}

impl ConfigureSheet {
    pub fn new() -> Self {
        ConfigureSheet { options: Vec::new() }
    }

    /// Adds an on or off option. Returns false when the key is empty or already used.
    pub fn add_bool(&mut self, key: &str, default: bool) -> bool {
        self.add(key, OptionKind::Bool, SaverValue::Bool(default))
    }

    /// Adds a whole-number option in `min..=max`. Returns false when the key is empty or used, the range is empty, or
    /// the default is outside the range.
    pub fn add_int(&mut self, key: &str, min: i64, max: i64, default: i64) -> bool {
        if min > max || default < min || default > max {
            return false;
        }
        self.add(key, OptionKind::Int { min, max }, SaverValue::Int(default))
    }

    /// Adds a choice option. Returns false when the key is empty or used, there are no choices, or the default is not
    /// one of them.
    pub fn add_choice(&mut self, key: &str, choices: &[&str], default: &str) -> bool {
        if choices.is_empty() || !choices.contains(&default) {
            return false;
        }
        let list: Vec<String> = choices.iter().map(|c| c.to_string()).collect();
        self.add(key, OptionKind::Choice(list), SaverValue::Choice(default.to_string()))
    }

    fn add(&mut self, key: &str, kind: OptionKind, value: SaverValue) -> bool {
        if key.is_empty() || self.options.iter().any(|o| o.key == key) {
            return false;
        }
        self.options.push(SheetOption { key: key.to_string(), kind, value });
        true
    }

    /// The current value of an option, or None for an unknown key.
    pub fn get(&self, key: &str) -> Option<&SaverValue> {
        self.options.iter().find(|o| o.key == key).map(|o| &o.value)
    }

    /// Sets an option's value. Returns false and changes nothing for an unknown key, a value of the wrong kind, an
    /// integer outside its range, or a choice that is not in the list.
    pub fn set(&mut self, key: &str, value: SaverValue) -> bool {
        let Some(opt) = self.options.iter_mut().find(|o| o.key == key) else {
            return false;
        };
        let ok = match (&opt.kind, &value) {
            (OptionKind::Bool, SaverValue::Bool(_)) => true,
            (OptionKind::Int { min, max }, SaverValue::Int(v)) => v >= min && v <= max,
            (OptionKind::Choice(list), SaverValue::Choice(c)) => list.iter().any(|x| x == c),
            _ => false,
        };
        if ok {
            opt.value = value;
        }
        ok
    }

    /// The option keys in the order they were added.
    pub fn keys(&self) -> Vec<&str> {
        self.options.iter().map(|o| o.key.as_str()).collect()
    }

    /// The (key, value) pairs in the order they were added, for saving to the module's preferences.
    pub fn values(&self) -> Vec<(String, SaverValue)> {
        self.options.iter().map(|o| (o.key.clone(), o.value.clone())).collect()
    }

    /// Number of options.
    pub fn len(&self) -> usize {
        self.options.len()
    }

    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
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

    #[test]
    fn view_reports_backing_store_gamma_fade_and_sheet_flags() {
        let mut v = ScreenSaverView::new(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, false);
        // Defaults: a buffered store, no gamma fade and no configuration sheet.
        assert_eq!(v.backing_store_type(), BackingStoreType::Buffered);
        assert!(!v.perform_gamma_fade());
        assert!(!v.has_configure_sheet());
        v.set_backing_store_type(BackingStoreType::Retained);
        v.set_perform_gamma_fade(true);
        v.set_has_configure_sheet(true);
        assert_eq!(v.backing_store_type(), BackingStoreType::Retained);
        assert!(v.perform_gamma_fade());
        assert!(v.has_configure_sheet());
    }

    #[test]
    fn preview_fits_screen_aspect_and_centres() {
        // A 1600x1000 screen (1.6:1) in a 200x200 box: scale is min(0.125, 0.2) = 0.125, so 200x125, centred at y 37.5.
        let thumb = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        assert_eq!(
            preview_rect(Rect { x: 0.0, y: 0.0, w: 1600.0, h: 1000.0 }, thumb),
            Rect { x: 0.0, y: 37.5, w: 200.0, h: 125.0 }
        );
        // A square screen in a 300x100 box: scale 0.1, so 100x100 at x 10 + 100, y 20.
        let wide = Rect { x: 10.0, y: 20.0, w: 300.0, h: 100.0 };
        assert_eq!(
            preview_rect(Rect { x: 0.0, y: 0.0, w: 1000.0, h: 1000.0 }, wide),
            Rect { x: 110.0, y: 20.0, w: 100.0, h: 100.0 }
        );
        // A screen with no area gives an empty rectangle at the centre of the box.
        assert_eq!(
            preview_rect(Rect { x: 0.0, y: 0.0, w: 0.0, h: 1000.0 }, thumb),
            Rect { x: 100.0, y: 100.0, w: 0.0, h: 0.0 }
        );
    }

    #[test]
    fn configure_sheet_checks_values_on_set() {
        let mut sheet = ConfigureSheet::new();
        assert!(sheet.add_bool("fade", true));
        // A duplicate key is refused and the first option is kept.
        assert!(!sheet.add_bool("fade", false));
        assert_eq!(sheet.get("fade"), Some(&SaverValue::Bool(true)));
        // A default outside the range is refused; an in-range default is accepted.
        assert!(!sheet.add_int("count", 1, 10, 12));
        assert!(sheet.add_int("count", 1, 10, 4));
        assert!(sheet.set("count", SaverValue::Int(10)));
        assert!(!sheet.set("count", SaverValue::Int(11)));
        assert_eq!(sheet.get("count"), Some(&SaverValue::Int(10)));
        assert!(sheet.add_choice("style", &["clock", "photos"], "photos"));
        assert!(!sheet.add_choice("bad", &["a"], "b"));
        assert!(sheet.set("style", SaverValue::Choice("clock".to_string())));
        assert!(!sheet.set("style", SaverValue::Choice("ocean".to_string())));
        // A value of the wrong kind, or for an unknown key, changes nothing.
        assert!(!sheet.set("style", SaverValue::Bool(true)));
        assert!(!sheet.set("missing", SaverValue::Bool(true)));
        assert_eq!(sheet.get("style"), Some(&SaverValue::Choice("clock".to_string())));
        assert_eq!(sheet.keys(), vec!["fade", "count", "style"]);
        assert_eq!(sheet.len(), 3);
    }

    #[test]
    fn configure_sheet_values_keep_insertion_order() {
        let mut sheet = ConfigureSheet::new();
        assert!(sheet.is_empty());
        assert!(sheet.add_int("speed", 0, 5, 2));
        assert!(sheet.add_bool("stars", false));
        assert!(sheet.set("stars", SaverValue::Bool(true)));
        assert_eq!(
            sheet.values(),
            vec![("speed".to_string(), SaverValue::Int(2)), ("stars".to_string(), SaverValue::Bool(true))]
        );
        // An empty key is refused.
        assert!(!sheet.add_bool("", true));
    }
}
