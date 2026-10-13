//! Transitions between two pieces of content (fade, move in, push, reveal) as a pure model.
//!
//! A [`Transition`] describes the effect: its kind, the side the incoming content comes from, how long it runs, the
//! timing, and the range of progress it plays over. [`Transition::placement`] turns a time-progress into where the two
//! pieces sit: the outgoing one (`under`) and the incoming one (`over`). The caller draws them; this module touches no
//! pixels. A pop is the same transition played backwards: `start_progress` 1 and `end_progress` 0, so the content that
//! was incoming now leaves. The shell's Settings and Files stacks and the SDK share this one model.

use crate::anim::Curve;

/// Seconds a cross-fade takes when motion is reduced.
pub const REDUCED_DURATION: f32 = 0.12;

/// What the transition does to the two pieces of content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The incoming content fades in over the outgoing one; nothing moves.
    Fade,
    /// The incoming content slides in over the outgoing one, which stays still.
    MoveIn,
    /// The incoming content slides in and pushes the outgoing one out (or part way out, see `under_shift`).
    Push,
    /// The outgoing content slides away and uncovers the incoming one, which stays still underneath.
    Reveal,
}

/// The side the incoming content comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subtype {
    FromRight,
    FromLeft,
    FromTop,
    FromBottom,
}

impl Subtype {
    /// Unit direction from the centre towards the side the content comes from, in screen coordinates (y grows down).
    pub fn direction(self) -> (f32, f32) {
        match self {
            Subtype::FromRight => (1.0, 0.0),
            Subtype::FromLeft => (-1.0, 0.0),
            Subtype::FromTop => (0.0, -1.0),
            Subtype::FromBottom => (0.0, 1.0),
        }
    }
    pub fn is_horizontal(self) -> bool {
        matches!(self, Subtype::FromRight | Subtype::FromLeft)
    }
}

/// How progress maps to time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    /// `1 - (1 - p)^3`, the Settings push.
    OutCubic,
    /// `1 - (1 - p)^4`, the navigation stack push.
    OutQuartic,
    /// Any curve of the animation module.
    Curve(Curve),
}

impl Ease {
    pub fn at(self, p: f32) -> f32 {
        let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
        match self {
            Ease::Linear => p,
            Ease::OutCubic => 1.0 - (1.0 - p).powi(3),
            Ease::OutQuartic => 1.0 - (1.0 - p).powi(4),
            Ease::Curve(c) => c.at(p),
        }
    }
}

/// A transition effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub kind: Kind,
    pub subtype: Subtype,
    /// Seconds the effect runs.
    pub duration: f32,
    pub ease: Ease,
    /// Eased progress at the start of the run (0 = outgoing content shown alone).
    pub start_progress: f32,
    /// Eased progress at the end of the run (1 = incoming content shown alone).
    pub end_progress: f32,
    /// Push only: the fraction of the extent the outgoing content moves while the incoming content covers it
    /// (1 = pushed out entirely, 0.3 = a parallax drift).
    pub under_shift: f32,
    /// Push and move-in: opacity (0..1) of the shade over the outgoing content once it is fully covered.
    pub under_dim: f32,
}

/// Where the two pieces sit at one moment. Offsets are in the units of the extent given to `placement`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub under_x: f32,
    pub under_y: f32,
    /// Opacity of the outgoing content, 0..1.
    pub under_alpha: f32,
    /// Opacity of the shade over the outgoing content, 0..1.
    pub shade: f32,
    pub over_x: f32,
    pub over_y: f32,
    /// Opacity of the incoming content, 0..1.
    pub over_alpha: f32,
    /// Whether the incoming content is drawn after (above) the outgoing one. False for a reveal.
    pub over_on_top: bool,
}

impl Transition {
    pub const DEFAULT_DURATION: f32 = 0.25;

    fn base(kind: Kind, subtype: Subtype) -> Transition {
        Transition {
            kind,
            subtype,
            duration: Self::DEFAULT_DURATION,
            ease: Ease::Curve(Curve::EaseInOut),
            start_progress: 0.0,
            end_progress: 1.0,
            under_shift: if kind == Kind::Push { 1.0 } else { 0.0 },
            under_dim: 0.0,
        }
    }
    pub fn fade() -> Transition {
        Self::base(Kind::Fade, Subtype::FromRight)
    }
    pub fn move_in(from: Subtype) -> Transition {
        Self::base(Kind::MoveIn, from)
    }
    pub fn push(from: Subtype) -> Transition {
        Self::base(Kind::Push, from)
    }
    pub fn reveal(from: Subtype) -> Transition {
        Self::base(Kind::Reveal, from)
    }
    /// The cross-fade that stands in for motion under Reduce Motion.
    pub fn reduced() -> Transition {
        Transition { duration: REDUCED_DURATION, ease: Ease::Linear, ..Self::fade() }
    }

    pub fn duration(mut self, seconds: f32) -> Transition {
        self.duration = if seconds.is_nan() { 0.0 } else { seconds.max(0.0) };
        self
    }
    pub fn ease(mut self, ease: Ease) -> Transition {
        self.ease = ease;
        self
    }
    /// Plays over `start..=end` of the eased progress (both clamped to 0..=1); `end < start` plays backwards.
    pub fn range(mut self, start: f32, end: f32) -> Transition {
        let c = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        self.start_progress = c(start);
        self.end_progress = c(end);
        self
    }
    /// The same effect played backwards (a pop for a push).
    pub fn reversed(mut self) -> Transition {
        std::mem::swap(&mut self.start_progress, &mut self.end_progress);
        self
    }
    /// Push parallax: the outgoing content drifts `shift` of the extent and is shaded by `dim` (0..1).
    pub fn parallax(mut self, shift: f32, dim: f32) -> Transition {
        self.under_shift = shift.max(0.0);
        self.under_dim = dim.clamp(0.0, 1.0);
        self
    }

    /// Fraction of the run elapsed at `now` for a run that began at `start` (seconds), 0..=1. A zero duration is done.
    pub fn progress(&self, now: f64, start: f64) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (((now - start) / self.duration as f64) as f32).clamp(0.0, 1.0)
        }
    }

    /// Where the pieces sit at time-progress `p` (0..=1) for content of `w` by `h`.
    pub fn placement(&self, p: f32, w: f32, h: f32) -> Placement {
        let e = self.ease.at(p);
        let k = self.start_progress + (self.end_progress - self.start_progress) * e;
        let (dx, dy) = self.subtype.direction();
        let extent = if self.subtype.is_horizontal() { w } else { h };
        let mut pl = Placement { under_x: 0.0, under_y: 0.0, under_alpha: 1.0, shade: 0.0, over_x: 0.0, over_y: 0.0, over_alpha: 1.0, over_on_top: true };
        match self.kind {
            Kind::Fade => pl.over_alpha = k,
            Kind::MoveIn | Kind::Push => {
                pl.over_x = dx * (extent * (1.0 - k));
                pl.over_y = dy * (extent * (1.0 - k));
                pl.shade = self.under_dim * k;
                if self.kind == Kind::Push {
                    let m = self.under_shift * extent * k;
                    pl.under_x = -dx * m;
                    pl.under_y = -dy * m;
                }
            }
            Kind::Reveal => {
                pl.over_on_top = false;
                pl.under_x = -dx * (extent * k);
                pl.under_y = -dy * (extent * k);
            }
        }
        pl
    }
}

impl Placement {
    /// The shade as an 8-bit alpha.
    pub fn shade_u8(&self) -> u8 {
        (self.shade * 255.0) as u8
    }
}

/// A transition in flight: the effect and the time it began.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub transition: Transition,
    pub start: f64,
}

impl Run {
    pub fn new(transition: Transition, start: f64) -> Run {
        Run { transition, start }
    }
    pub fn is_done(&self, now: f64) -> bool {
        self.transition.progress(now, self.start) >= 1.0
    }
    pub fn placement(&self, now: f64, w: f32, h: f32) -> Placement {
        self.transition.placement(self.transition.progress(now, self.start), w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_changes_only_the_incoming_opacity() {
        let t = Transition::fade().ease(Ease::Linear);
        let p = t.placement(0.25, 100.0, 50.0);
        assert_eq!((p.under_x, p.over_x, p.under_alpha, p.over_alpha), (0.0, 0.0, 1.0, 0.25));
        assert_eq!(t.placement(1.0, 100.0, 50.0).over_alpha, 1.0);
    }

    #[test]
    fn move_in_comes_from_the_named_side() {
        let t = Transition::move_in(Subtype::FromBottom).ease(Ease::Linear);
        let p = t.placement(0.0, 100.0, 50.0);
        assert_eq!((p.over_x, p.over_y, p.under_y), (0.0, 50.0, 0.0));
        assert_eq!(t.placement(1.0, 100.0, 50.0).over_y, 0.0);
        let l = Transition::move_in(Subtype::FromLeft).ease(Ease::Linear).placement(0.5, 100.0, 50.0);
        assert_eq!(l.over_x, -50.0);
        let top = Transition::move_in(Subtype::FromTop).ease(Ease::Linear).placement(0.0, 100.0, 50.0);
        assert_eq!(top.over_y, -50.0);
    }

    #[test]
    fn push_moves_both_pieces() {
        let t = Transition::push(Subtype::FromRight).ease(Ease::Linear);
        let p = t.placement(0.5, 100.0, 50.0);
        assert_eq!((p.over_x, p.under_x), (50.0, -50.0));
        assert!(p.over_on_top);
    }

    #[test]
    fn reveal_moves_the_outgoing_piece_and_draws_the_incoming_below() {
        let t = Transition::reveal(Subtype::FromRight).ease(Ease::Linear);
        let p = t.placement(0.5, 100.0, 50.0);
        assert_eq!((p.under_x, p.over_x), (-50.0, 0.0));
        assert!(!p.over_on_top);
    }

    #[test]
    fn a_reversed_push_is_a_pop() {
        let pop = Transition::push(Subtype::FromRight).parallax(0.3, 0.12).ease(Ease::OutQuartic).reversed();
        let p = pop.placement(0.0, 1000.0, 0.0);
        assert_eq!((p.under_x.round(), p.over_x, p.shade_u8()), (-300.0, 0.0, 30));
        let e = pop.placement(1.0, 1000.0, 0.0);
        assert_eq!((e.under_x, e.over_x, e.shade_u8()), (0.0, 1000.0, 0));
    }

    #[test]
    fn range_is_clamped_and_progress_follows_the_clock() {
        let t = Transition::fade().range(-1.0, 3.0).duration(2.0);
        assert_eq!((t.start_progress, t.end_progress), (0.0, 1.0));
        assert_eq!(t.progress(1.0, 0.0), 0.5);
        assert_eq!(t.progress(10.0, 0.0), 1.0);
        assert_eq!(Transition::fade().duration(0.0).progress(0.0, 0.0), 1.0);
        assert_eq!(Transition::fade().duration(f32::NAN).duration, 0.0);
    }

    #[test]
    fn eases_run_from_zero_to_one() {
        for e in [Ease::Linear, Ease::OutCubic, Ease::OutQuartic, Ease::Curve(Curve::EaseInOut)] {
            assert!(e.at(0.0).abs() < 1e-6 && (e.at(1.0) - 1.0).abs() < 1e-6);
        }
        assert!(Ease::OutCubic.at(0.5) > 0.5);
        assert_eq!(Ease::Linear.at(f32::NAN), 0.0);
    }

    #[test]
    fn a_run_finishes_after_its_duration() {
        let r = Run::new(Transition::fade().duration(0.5), 10.0);
        assert!(!r.is_done(10.2));
        assert!(r.is_done(10.5));
        assert_eq!(Transition::reduced().duration, REDUCED_DURATION);
    }
}
