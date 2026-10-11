//! Host-testable Camera logic: the self-timer state machine, aspect crop math,
//! capture file naming and the small animation curves. No drawing, no device.

use crate::shell::photos::library;
use std::path::Path;

/// Capture shape. The sensor is 4:3; Square centre-crops the long side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Aspect {
    FourThree,
    Square,
}

impl Aspect {
    pub fn toggled(self) -> Aspect {
        match self {
            Aspect::FourThree => Aspect::Square,
            Aspect::Square => Aspect::FourThree,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Aspect::FourThree => "4:3",
            Aspect::Square => "1:1",
        }
    }

    /// The centred crop `(x, y, w, h)` of a `fw` x `fh` frame. 4:3 keeps the frame's
    /// orientation (3:4 for a portrait frame). Never exceeds the frame.
    pub fn crop(self, fw: usize, fh: usize) -> (usize, usize, usize, usize) {
        if fw == 0 || fh == 0 {
            return (0, 0, fw, fh);
        }
        let (w, h) = match self {
            Aspect::Square => {
                let side = fw.min(fh);
                (side, side)
            }
            Aspect::FourThree => {
                let (long, short) = if fw >= fh { (4usize, 3usize) } else { (3, 4) };
                // largest (long:short) rectangle that fits: limit by width or by height
                if fw * short <= fh * long {
                    (fw, (fw * short / long).max(1))
                } else {
                    ((fh * long / short).max(1), fh)
                }
            }
        };
        let (w, h) = (w.min(fw), h.min(fh));
        ((fw - w) / 2, (fh - h) / 2, w, h)
    }
}

/// Self-timer choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimerMode {
    Off,
    Three,
    Ten,
}

impl TimerMode {
    pub fn cycled(self) -> TimerMode {
        match self {
            TimerMode::Off => TimerMode::Three,
            TimerMode::Three => TimerMode::Ten,
            TimerMode::Ten => TimerMode::Off,
        }
    }

    pub fn seconds(self) -> u32 {
        match self {
            TimerMode::Off => 0,
            TimerMode::Three => 3,
            TimerMode::Ten => 10,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TimerMode::Off => "Timer off",
            TimerMode::Three => "Timer 3s",
            TimerMode::Ten => "Timer 10s",
        }
    }
}

/// What pressing the shutter should do right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShutterAction {
    /// take the photo immediately
    Capture,
    /// a countdown began
    Countdown,
    /// a running countdown was cancelled
    Cancelled,
}

/// The countdown state machine. Times are milliseconds on the shell clock.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SelfTimer {
    pub mode: TimerMode,
    ends_at: Option<f64>,
}

impl SelfTimer {
    pub fn new() -> Self {
        Self { mode: TimerMode::Off, ends_at: None }
    }

    pub fn running(&self) -> bool {
        self.ends_at.is_some()
    }

    /// Cycles Off -> 3 s -> 10 s; changing the mode cancels a running countdown.
    pub fn cycle(&mut self) {
        self.mode = self.mode.cycled();
        self.ends_at = None;
    }

    pub fn press(&mut self, now: f64) -> ShutterAction {
        if self.ends_at.take().is_some() {
            return ShutterAction::Cancelled;
        }
        if self.mode == TimerMode::Off {
            return ShutterAction::Capture;
        }
        self.ends_at = Some(now + self.mode.seconds() as f64 * 1000.0);
        ShutterAction::Countdown
    }

    pub fn cancel(&mut self) {
        self.ends_at = None;
    }

    /// Whole seconds left, shown on screen (3, 2, 1); None when not counting.
    pub fn remaining(&self, now: f64) -> Option<u32> {
        let end = self.ends_at?;
        Some(((end - now).max(0.0) / 1000.0).ceil() as u32)
    }

    /// True exactly once, when the countdown reaches zero; the timer is then idle again.
    pub fn poll(&mut self, now: f64) -> bool {
        match self.ends_at {
            Some(end) if now >= end => {
                self.ends_at = None;
                true
            }
            _ => false,
        }
    }
}

impl Default for SelfTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// A file name for a photo taken at `date` that is free in `dir` and not in `reserved`
/// (names handed out whose files are still being written): `-2`, `-3` on a same-second clash.
pub fn photo_name_avoiding(dir: &Path, date: &library::Date, reserved: &[String]) -> String {
    let stamp = library::stamp(date);
    let mut name = format!("IMG_{stamp}.jpg");
    let mut n = 2;
    while dir.join(&name).exists() || reserved.contains(&name) {
        name = format!("IMG_{stamp}-{n}.jpg");
        n += 1;
    }
    name
}

/// `photo_name_avoiding` with nothing reserved.
pub fn photo_name(dir: &Path, date: &library::Date) -> String {
    photo_name_avoiding(dir, date, &[])
}

/// The EXIF `DateTime` text ("YYYY:MM:DD HH:MM:SS").
pub fn exif_date(d: &library::Date) -> String {
    format!("{:04}:{:02}:{:02} {:02}:{:02}:{:02}", d.year, d.month, d.day, d.hour, d.min, d.sec)
}

/// The two vertical and two horizontal rule-of-thirds lines of a rect, as `(x0, y0, x1, y1)`.
pub fn grid_lines(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32, f32, f32); 4] {
    [
        (x + w / 3.0, y, x + w / 3.0, y + h),
        (x + 2.0 * w / 3.0, y, x + 2.0 * w / 3.0, y + h),
        (x, y + h / 3.0, x + w, y + h / 3.0),
        (x, y + 2.0 * h / 3.0, x + w, y + 2.0 * h / 3.0),
    ]
}

/// Clamps a tap into the viewfinder rect `(x, y, w, h)`.
pub fn clamp_into(r: (f32, f32, f32, f32), px: f32, py: f32) -> (f32, f32) {
    (px.clamp(r.0, r.0 + r.2), py.clamp(r.1, r.1 + r.3))
}

pub const FOCUS_MS: f64 = 700.0;
pub const SHUTTER_MS: f64 = 180.0;

/// The tap-to-focus square: `(scale, alpha)` at `elapsed` ms. It starts large, snaps to
/// size 1.0 in the first 200 ms, holds, and fades over the last 250 ms. None when finished.
pub fn focus_anim(elapsed: f64) -> Option<(f32, u8)> {
    if !(0.0..FOCUS_MS).contains(&elapsed) {
        return None;
    }
    let scale = if elapsed < 200.0 {
        let k = (elapsed / 200.0) as f32;
        1.5 - 0.5 * (1.0 - (1.0 - k) * (1.0 - k))
    } else {
        1.0
    };
    let fade_start = FOCUS_MS - 250.0;
    let alpha = if elapsed < fade_start { 255.0 } else { 255.0 * (1.0 - (elapsed - fade_start) / 250.0) };
    Some((scale, alpha.clamp(0.0, 255.0) as u8))
}

/// Shutter feedback over the viewfinder: a white flash that peaks at once and fades out.
pub fn shutter_flash(elapsed: f64) -> u8 {
    if !(0.0..SHUTTER_MS).contains(&elapsed) {
        return 0;
    }
    (255.0 * (1.0 - elapsed / SHUTTER_MS)) as u8
}

/// The shutter button's press-in scale: dips to 0.9 at the start and recovers.
pub fn shutter_button_scale(elapsed: f64) -> f32 {
    if !(0.0..SHUTTER_MS).contains(&elapsed) {
        return 1.0;
    }
    let k = (elapsed / SHUTTER_MS) as f32;
    0.9 + 0.1 * k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_crop_centres_the_short_side() {
        assert_eq!(Aspect::Square.crop(4000, 3000), (500, 0, 3000, 3000));
        assert_eq!(Aspect::Square.crop(3000, 4000), (0, 500, 3000, 3000));
    }

    #[test]
    fn four_three_crop_fits_any_frame() {
        assert_eq!(Aspect::FourThree.crop(4000, 3000), (0, 0, 4000, 3000));
        // a 16:9 frame loses width
        assert_eq!(Aspect::FourThree.crop(1920, 1080), (240, 0, 1440, 1080));
        // a portrait frame stays portrait
        assert_eq!(Aspect::FourThree.crop(1080, 1920), (0, 240, 1080, 1440));
        // a squarish frame loses height
        assert_eq!(Aspect::FourThree.crop(1000, 1000), (0, 125, 1000, 750));
    }

    #[test]
    fn crops_never_leave_the_frame_and_handle_empty() {
        for (w, h) in [(1, 1), (7, 3), (640, 480), (5, 9)] {
            for a in [Aspect::FourThree, Aspect::Square] {
                let (x, y, cw, ch) = a.crop(w, h);
                assert!(cw >= 1 && ch >= 1 && x + cw <= w && y + ch <= h, "{a:?} {w}x{h}");
            }
        }
        assert_eq!(Aspect::Square.crop(0, 0), (0, 0, 0, 0));
    }

    #[test]
    fn aspect_toggles() {
        assert_eq!(Aspect::FourThree.toggled(), Aspect::Square);
        assert_eq!(Aspect::Square.toggled().label(), "4:3");
    }

    #[test]
    fn timer_cycles_through_the_modes() {
        let mut t = SelfTimer::new();
        assert_eq!(t.mode.seconds(), 0);
        t.cycle();
        assert_eq!(t.mode, TimerMode::Three);
        t.cycle();
        assert_eq!(t.mode.seconds(), 10);
        t.cycle();
        assert_eq!(t.mode, TimerMode::Off);
    }

    #[test]
    fn off_timer_captures_at_once() {
        let mut t = SelfTimer::new();
        assert_eq!(t.press(0.0), ShutterAction::Capture);
        assert!(!t.running());
    }

    #[test]
    fn countdown_counts_3_2_1_then_fires_once() {
        let mut t = SelfTimer::new();
        t.cycle();
        assert_eq!(t.press(1000.0), ShutterAction::Countdown);
        assert_eq!(t.remaining(1000.0), Some(3));
        assert_eq!(t.remaining(2100.0), Some(2));
        assert_eq!(t.remaining(3999.0), Some(1));
        assert!(!t.poll(3999.0));
        assert!(t.poll(4000.0));
        assert!(!t.poll(4001.0), "fires once");
        assert_eq!(t.remaining(4001.0), None);
        assert!(!t.running());
    }

    #[test]
    fn pressing_during_a_countdown_cancels_it() {
        let mut t = SelfTimer::new();
        t.cycle();
        t.cycle();
        assert_eq!(t.press(0.0), ShutterAction::Countdown);
        assert_eq!(t.remaining(0.0), Some(10));
        assert_eq!(t.press(500.0), ShutterAction::Cancelled);
        assert!(!t.poll(20_000.0));
    }

    #[test]
    fn changing_the_mode_cancels_a_countdown() {
        let mut t = SelfTimer::new();
        t.cycle();
        t.press(0.0);
        t.cycle();
        assert!(!t.running());
    }

    #[test]
    fn names_are_jpg_stamped_and_never_clash() {
        let dir = std::env::temp_dir().join(format!("camera-logic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = library::civil(1_791_073_395, 0);
        let first = photo_name(&dir, &d);
        assert_eq!(first, "IMG_20261004-002315.jpg");
        std::fs::write(dir.join(&first), b"x").unwrap();
        assert_eq!(photo_name(&dir, &d), "IMG_20261004-002315-2.jpg");
        let reserved = vec![first.clone(), "IMG_20261004-002315-2.jpg".to_string()];
        assert_eq!(photo_name_avoiding(&dir, &d, &reserved), "IMG_20261004-002315-3.jpg");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exif_date_format() {
        let d = library::civil(1_791_073_395, 0);
        assert_eq!(exif_date(&d), "2026:10:04 00:23:15");
    }

    #[test]
    fn grid_is_rule_of_thirds() {
        let g = grid_lines(10.0, 20.0, 300.0, 600.0);
        assert_eq!(g[0], (110.0, 20.0, 110.0, 620.0));
        assert_eq!(g[1].0, 210.0);
        assert_eq!(g[2], (10.0, 220.0, 310.0, 220.0));
        assert_eq!(g[3].1, 420.0);
    }

    #[test]
    fn taps_clamp_into_the_view() {
        assert_eq!(clamp_into((0.0, 10.0, 100.0, 100.0), -5.0, 500.0), (0.0, 110.0));
        assert_eq!(clamp_into((0.0, 10.0, 100.0, 100.0), 50.0, 50.0), (50.0, 50.0));
    }

    #[test]
    fn focus_square_snaps_in_holds_and_fades() {
        let (s0, a0) = focus_anim(0.0).unwrap();
        assert!(s0 > 1.4 && a0 == 255);
        let (s1, a1) = focus_anim(300.0).unwrap();
        assert_eq!((s1, a1), (1.0, 255));
        let (_, a2) = focus_anim(600.0).unwrap();
        assert!(a2 < 255 && a2 > 0);
        assert!(focus_anim(FOCUS_MS).is_none());
        assert!(focus_anim(-1.0).is_none());
    }

    #[test]
    fn shutter_flash_fades_and_button_recovers() {
        assert_eq!(shutter_flash(0.0), 255);
        assert!(shutter_flash(SHUTTER_MS / 2.0) < 200);
        assert_eq!(shutter_flash(SHUTTER_MS), 0);
        assert!(shutter_button_scale(0.0) < 0.95);
        assert_eq!(shutter_button_scale(SHUTTER_MS + 1.0), 1.0);
    }
}
