//! Window open, close and minimize animations, run by the server rather than by the clients. Each animation maps a
//! frame of time to a `Pose` (scale, offset, opacity) that the compositor applies to the window's surface tree.
//! Pure timing and geometry: no GPU or Smithay types, so it is host-tested.
//!
//! Reduce motion (from `/etc/bifrost` or the shell prefs) removes the scale and the travel: open and close become
//! a short fade, and minimize is instant.

use crate::wm::Rect;

/// Open: from 90 % to full size with a fade, 250 ms, ease-out.
pub const OPEN_MS: u32 = 250;
/// Close: the reverse of open, from full size to 90 % with a fade.
pub const CLOSE_MS: u32 = 250;
/// Minimize: from the window frame to the Dock icon's rect.
pub const MINIMIZE_MS: u32 = 350;
/// The reduced-motion fade for open and close.
pub const REDUCED_FADE_MS: u32 = 120;
/// The scale an opening or closing window starts or ends at.
pub const OPEN_FROM_SCALE: f32 = 0.9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Open, Close, Minimize }

/// How the window is drawn at one instant: scaled around its centre, moved by (`dx`, `dy`), faded to `alpha`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose { pub sx: f32, pub sy: f32, pub dx: f32, pub dy: f32, pub alpha: f32 }

impl Pose {
    pub const REST: Pose = Pose { sx: 1.0, sy: 1.0, dx: 0.0, dy: 0.0, alpha: 1.0 };

    /// The rect the window is drawn into, given its frame at rest.
    pub fn apply(&self, frame: Rect) -> Rect {
        let cx = frame.x as f32 + frame.w as f32 / 2.0 + self.dx;
        let cy = frame.y as f32 + frame.h as f32 / 2.0 + self.dy;
        let w = frame.w as f32 * self.sx;
        let h = frame.h as f32 * self.sy;
        Rect::new((cx - w / 2.0).round() as i32, (cy - h / 2.0).round() as i32, w.round() as i32, h.round() as i32)
    }
}

/// Ease-out cubic: fast start, gentle landing.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

/// One running animation of a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anim {
    pub kind: Kind,
    pub start_ms: u64,
    pub duration_ms: u32,
    /// The window's frame at rest.
    pub frame: Rect,
    /// Where a minimize goes: the Dock icon's rect. Unused by open and close.
    pub target: Rect,
    pub reduce_motion: bool,
}

impl Anim {
    pub fn new(kind: Kind, start_ms: u64, frame: Rect, target: Rect, reduce_motion: bool) -> Anim {
        let duration_ms = match (kind, reduce_motion) {
            (Kind::Minimize, true) => 0,
            (Kind::Minimize, false) => MINIMIZE_MS,
            (_, true) => REDUCED_FADE_MS,
            (Kind::Open, false) => OPEN_MS,
            (Kind::Close, false) => CLOSE_MS,
        };
        Anim { kind, start_ms, duration_ms, frame, target, reduce_motion }
    }

    /// The instant the animation ends.
    pub fn end_ms(&self) -> u64 { self.start_ms + self.duration_ms as u64 }

    pub fn finished(&self, now_ms: u64) -> bool { now_ms >= self.end_ms() }

    /// Progress from 0 to 1 at `now_ms`, eased for the motion kinds.
    fn progress(&self, now_ms: u64) -> f32 {
        if self.duration_ms == 0 { return 1.0; }
        let t = now_ms.saturating_sub(self.start_ms) as f32 / self.duration_ms as f32;
        t.clamp(0.0, 1.0)
    }

    /// The pose at `now_ms`. Once finished it is the end pose: open lands at rest, close and minimize end invisible.
    pub fn pose(&self, now_ms: u64) -> Pose {
        let t = ease_out_cubic(self.progress(now_ms));
        let end_alpha = match self.kind { Kind::Open => 1.0, Kind::Close | Kind::Minimize => 0.0 };
        if self.reduce_motion {
            return Pose { alpha: lerp(1.0 - end_alpha, end_alpha, t), ..Pose::REST };
        }
        match self.kind {
            Kind::Open => Pose { sx: lerp(OPEN_FROM_SCALE, 1.0, t), sy: lerp(OPEN_FROM_SCALE, 1.0, t), dx: 0.0, dy: 0.0, alpha: t },
            Kind::Close => Pose { sx: lerp(1.0, OPEN_FROM_SCALE, t), sy: lerp(1.0, OPEN_FROM_SCALE, t), dx: 0.0, dy: 0.0, alpha: 1.0 - t },
            Kind::Minimize => {
                let (fc_x, fc_y) = centre(self.frame);
                let (tc_x, tc_y) = centre(self.target);
                let sx = lerp(1.0, self.target.w as f32 / self.frame.w.max(1) as f32, t);
                let sy = lerp(1.0, self.target.h as f32 / self.frame.h.max(1) as f32, t);
                Pose { sx, sy, dx: lerp(0.0, tc_x - fc_x, t), dy: lerp(0.0, tc_y - fc_y, t), alpha: 1.0 - t }
            }
        }
    }
}

fn centre(r: Rect) -> (f32, f32) { (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0) }

/// The running animations of all windows. A window can have one at a time; a new one replaces the old.
#[derive(Default)]
pub struct Animator { anims: Vec<(u64, Anim)> }

impl Animator {
    pub fn new() -> Animator { Animator::default() }

    pub fn start(&mut self, window: u64, anim: Anim) {
        self.anims.retain(|(w, _)| *w != window);
        self.anims.push((window, anim));
    }

    /// The pose to draw `window` with, or `None` when it has no animation (draw it at rest).
    pub fn pose(&self, window: u64, now_ms: u64) -> Option<Pose> {
        self.anims.iter().find(|(w, _)| *w == window).map(|(_, a)| a.pose(now_ms))
    }

    /// The kind of the window's running animation, if any: a close keeps its last buffer until it ends.
    pub fn running(&self, window: u64, now_ms: u64) -> Option<Kind> {
        self.anims.iter().find(|(w, a)| *w == window && !a.finished(now_ms)).map(|(_, a)| a.kind)
    }

    /// Drops the animations that have ended and returns their window ids, so the caller can finish closes and
    /// minimizes.
    pub fn sweep(&mut self, now_ms: u64) -> Vec<(u64, Kind)> {
        let mut done = Vec::new();
        self.anims.retain(|(w, a)| {
            if a.finished(now_ms) {
                done.push((*w, a.kind));
                false
            } else {
                true
            }
        });
        done
    }

    /// Whether any animation still runs, so the compositor keeps scheduling frames.
    pub fn active(&self) -> bool { !self.anims.is_empty() }

    /// The next instant an animation needs a frame, if any is running.
    pub fn next_deadline(&self) -> Option<u64> { self.anims.iter().map(|(_, a)| a.end_ms()).min() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Rect = Rect::new(100, 100, 800, 600);
    const DOCK: Rect = Rect::new(1000, 900, 64, 64);

    fn close(a: f32, b: f32) -> bool { (a - b).abs() < 1e-4 }

    #[test]
    fn ease_out_cubic_starts_fast_and_lands_exactly() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(close(ease_out_cubic(0.5), 0.875), "1 - 0.5^3");
        assert!(ease_out_cubic(0.1) > 0.1, "ease-out is ahead of linear at the start");
        assert_eq!(ease_out_cubic(2.0), 1.0, "clamped");
    }

    #[test]
    fn an_open_grows_from_ninety_percent_with_a_fade_over_250_ms() {
        let a = Anim::new(Kind::Open, 1000, FRAME, FRAME, false);
        assert_eq!(a.duration_ms, OPEN_MS);
        let start = a.pose(1000);
        assert!(close(start.sx, 0.9) && close(start.alpha, 0.0));
        let mid = a.pose(1125);
        assert!(close(mid.sx, lerp(0.9, 1.0, 0.875)), "half time is 0.875 eased");
        let end = a.pose(1250);
        assert!(close(end.sx, 1.0) && close(end.alpha, 1.0), "it lands at rest");
        assert!(a.finished(1250) && !a.finished(1249));
    }

    #[test]
    fn a_close_is_the_reverse_and_ends_invisible_at_ninety_percent() {
        let a = Anim::new(Kind::Close, 0, FRAME, FRAME, false);
        assert!(close(a.pose(0).sx, 1.0) && close(a.pose(0).alpha, 1.0));
        let end = a.pose(250);
        assert!(close(end.sx, 0.9) && close(end.alpha, 0.0));
    }

    #[test]
    fn a_minimize_flies_to_the_dock_rect_in_350_ms_and_ends_on_the_target() {
        let a = Anim::new(Kind::Minimize, 0, FRAME, DOCK, false);
        assert_eq!(a.duration_ms, MINIMIZE_MS);
        let end = a.pose(350);
        assert_eq!(end.apply(FRAME), DOCK, "the window ends exactly on the Dock icon");
        assert!(close(end.alpha, 0.0));
        assert_eq!(a.pose(0).apply(FRAME), FRAME, "it starts at rest");
    }

    #[test]
    fn reduce_motion_fades_without_scaling_or_travel() {
        let open = Anim::new(Kind::Open, 0, FRAME, FRAME, true);
        assert_eq!(open.duration_ms, REDUCED_FADE_MS);
        let p = open.pose(60);
        assert!(close(p.sx, 1.0) && close(p.dx, 0.0) && p.alpha > 0.0 && p.alpha < 1.0);
        assert_eq!(open.pose(120).alpha, 1.0);
        let min = Anim::new(Kind::Minimize, 0, FRAME, DOCK, true);
        assert_eq!(min.duration_ms, 0, "minimize is instant");
        assert!(min.finished(0));
        assert_eq!(min.pose(0).alpha, 0.0);
    }

    #[test]
    fn the_animator_keeps_one_animation_per_window_and_sweeps_the_finished_ones() {
        let mut an = Animator::new();
        an.start(1, Anim::new(Kind::Open, 0, FRAME, FRAME, false));
        an.start(2, Anim::new(Kind::Close, 0, FRAME, FRAME, false));
        an.start(1, Anim::new(Kind::Close, 100, FRAME, FRAME, false));
        assert_eq!(an.running(1, 100), Some(Kind::Close), "the newer animation replaces the old");
        assert!(an.pose(3, 0).is_none());
        assert_eq!(an.sweep(260), vec![(2, Kind::Close)], "window 2 closed by 250 ms");
        assert!(an.active());
        assert_eq!(an.sweep(400), vec![(1, Kind::Close)], "window 1 ends at 350 ms");
        assert!(!an.active());
    }

    #[test]
    fn a_closing_window_is_still_running_until_its_animation_ends() {
        let mut an = Animator::new();
        an.start(7, Anim::new(Kind::Close, 0, FRAME, FRAME, false));
        assert_eq!(an.running(7, 249), Some(Kind::Close));
        assert_eq!(an.running(7, 250), None, "a finished animation is not running");
        assert!(an.pose(7, 250).is_some(), "but its end pose stays until it is swept");
    }

    #[test]
    fn the_next_deadline_is_the_earliest_end() {
        let mut an = Animator::new();
        assert_eq!(an.next_deadline(), None);
        an.start(1, Anim::new(Kind::Minimize, 1000, FRAME, DOCK, false));
        an.start(2, Anim::new(Kind::Open, 1100, FRAME, FRAME, false));
        assert_eq!(an.next_deadline(), Some(1350));
    }

    #[test]
    fn apply_scales_around_the_centre_and_moves_by_the_offset() {
        let p = Pose { sx: 0.5, sy: 0.5, dx: 10.0, dy: -20.0, alpha: 1.0 };
        let r = p.apply(Rect::new(0, 0, 200, 100));
        assert_eq!(r, Rect::new(60, 5, 100, 50));
    }
}
