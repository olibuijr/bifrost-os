//! Touch events for apps: the touch objects and frames of `bifrost_ui::touch`, and the routing that decides what an app
//! receives.
//!
//! An app implements [`App::touches`](crate::App::touches) and [`App::wants_touch_events`](crate::App::wants_touch_events)
//! to get [`TouchEvent`]s: every touch of a frame with its phase, location, previous location, tap count, optional
//! force and radius, and type. An app that does not opt in keeps getting `Input::Touch` (one primary pointer), exactly
//! as before, so nothing changes for existing apps.
//!
//! Where the frames come from:
//! - The shell sends `ToApp::Touches` ahead of each `ToApp::Touch`. Once the SDK has seen a frame it takes the touch
//!   stream from the frames and drops the plain `ToApp::Touch` messages for apps that opted in.
//! - A shell that sends only plain touches (a plain pointer, an older shell) still works: [`TouchRouter`] runs
//!   those through a [`TouchTracker`] and the app gets a single indirect touch with phases and tap counts.
//!
//! Locations are surface pixels, like `Input::Touch`. [`TouchEvent::in_view`] converts to the points of a view.
//!
//! ```ignore
//! impl App for Canvas {
//!     fn wants_touch_events(&self) -> bool { true }
//!     fn touches(&mut self, ctx: &Ctx, ev: &TouchEvent) -> bool {
//!         let ev = ev.to_points(ctx.cfg.scale);
//!         for t in ev.in_phase(TouchPhase::Moved) {
//!             self.stroke(t.id, t.previous_location(), t.location());
//!         }
//!         true
//!     }
//! }
//! ```

pub use bifrost_ui::touch::{Raw, Sample, Touch, TouchEvent, TouchPhase, TouchTracker, TouchType, MAX_TOUCHES};

use crate::proto::TouchKind;

/// A wire stamp (the low 32 bits of the shell's monotonic ms clock) on the app's clock at `now`. Unlike a plain touch,
/// a touch of a frame can be old (a finger held down is `Stationary` with the time it last changed), so any age up to
/// about 24 days is kept; a stamp that lies ahead of `now` falls back to `now`.
pub fn local_time(now: f64, wire_ms: u32) -> f64 {
    let ago = (now as u64 as u32).wrapping_sub(wire_ms);
    if ago < i32::MAX as u32 { now - ago as f64 } else { now }
}

/// Moves the wire stamps of a received frame onto the app's clock (see [`local_time`]).
pub fn localize(ev: &mut TouchEvent, now: f64) {
    let local = |t: f64| local_time(now, t as u64 as u32);
    ev.timestamp = local(ev.timestamp);
    for t in &mut ev.touches {
        t.timestamp = local(t.timestamp);
    }
}

/// What to do with a plain `ToApp::Touch`.
#[derive(Debug, PartialEq)]
pub enum Plain {
    /// deliver it as `Input::Touch`
    Deliver,
    /// deliver this frame with `App::touches`
    Frame(TouchEvent),
    /// the touch stream comes from frames, or the sample changes nothing
    Skip,
}

/// Decides, for each incoming touch message, whether the app gets `Input::Touch` or a touch event.
#[derive(Debug)]
pub struct TouchRouter {
    tracker: TouchTracker,
    /// the shell sends frames: plain touches are redundant for apps that take events
    frames_seen: bool,
}

impl TouchRouter {
    /// `scale` is the surface's pixels per point; it sizes the tap slop.
    pub fn new(scale: f32) -> Self {
        TouchRouter { tracker: TouchTracker::with_scale(scale), frames_seen: false }
    }

    /// The surface scale changed.
    pub fn set_scale(&mut self, scale: f32) {
        if scale > 0.0 {
            self.tracker.slop = bifrost_ui::touch::TAP_SLOP * scale;
        }
    }

    /// True once the shell has sent a touch frame.
    pub fn frames_seen(&self) -> bool {
        self.frames_seen
    }

    /// A `ToApp::Touches` frame arrived. Returns the frame for an app that takes touch events, `None` for one that
    /// does not.
    pub fn frame(&mut self, aware: bool, ev: TouchEvent) -> Option<TouchEvent> {
        if !self.frames_seen {
            // the plain stream was being tracked here: forget it, the frames own the touches from now on
            self.frames_seen = true;
            let slop = self.tracker.slop;
            self.tracker = TouchTracker::new();
            self.tracker.slop = slop;
        }
        aware.then_some(ev)
    }

    /// A plain `ToApp::Touch` arrived at `t` (ms, the app's clock).
    pub fn plain(&mut self, aware: bool, kind: TouchKind, id: u32, x: f32, y: f32, t: f64) -> Plain {
        if !aware {
            return Plain::Deliver;
        }
        if self.frames_seen {
            return Plain::Skip;
        }
        let s = Sample::at(id, x, y).with_kind(TouchType::Indirect);
        let ev = match kind {
            TouchKind::Down => self.tracker.down(s, t),
            TouchKind::Move => self.tracker.moved(s, t),
            TouchKind::Up => self.tracker.up(s, t),
            TouchKind::Cancel => self.tracker.cancel(id, t),
        };
        ev.map_or(Plain::Skip, Plain::Frame)
    }

    /// The app is leaving the foreground: the touches the plain stream still has down are cancelled.
    pub fn cancel_all(&mut self, aware: bool, t: f64) -> Option<TouchEvent> {
        if aware && !self.frames_seen {
            self.tracker.cancel_all(t)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_of(p: Plain) -> TouchEvent {
        match p {
            Plain::Frame(f) => f,
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    #[test]
    fn old_stamps_stay_old_and_future_ones_clamp() {
        assert_eq!(local_time(10_000.0, 9_990), 9_990.0);
        assert_eq!(local_time(10_000.0, 4_000), 4_000.0, "a held finger keeps the time it last changed");
        assert_eq!(local_time(10_000.0, 10_050), 10_000.0, "ahead of now");
        // the low 32 bits wrapped between the stamp and now
        let now = (u32::MAX as f64) + 101.0;
        assert_eq!(local_time(now, u32::MAX - 9), now - 110.0);
    }

    #[test]
    fn a_frame_is_localized_touch_by_touch() {
        let mut tr = TouchTracker::new();
        tr.down(Sample::at(1, 0.0, 0.0), 4_000.0);
        let mut ev = tr.down(Sample::at(2, 5.0, 5.0), 9_990.0).unwrap();
        localize(&mut ev, 10_000.0);
        assert_eq!(ev.timestamp, 9_990.0);
        assert_eq!(ev.touch(1).unwrap().timestamp, 4_000.0);
        assert_eq!(ev.touch(2).unwrap().timestamp, 9_990.0);
    }

    #[test]
    fn apps_without_events_get_plain_touches() {
        let mut r = TouchRouter::new(1.0);
        assert_eq!(r.plain(false, TouchKind::Down, 0, 1.0, 2.0, 0.0), Plain::Deliver);
        assert_eq!(r.plain(false, TouchKind::Up, 0, 1.0, 2.0, 5.0), Plain::Deliver);
        let ev = TouchEvent { timestamp: 1.0, touches: vec![] };
        assert_eq!(r.frame(false, ev), None);
        assert!(r.frames_seen());
        assert_eq!(r.plain(false, TouchKind::Down, 0, 1.0, 2.0, 9.0), Plain::Deliver, "plain touches stay for them");
    }

    #[test]
    fn a_lone_pointer_becomes_a_single_indirect_touch() {
        let mut r = TouchRouter::new(1.0);
        let e = frame_of(r.plain(true, TouchKind::Down, 0, 10.0, 10.0, 100.0));
        assert_eq!((e.touches.len(), e.touches[0].phase, e.touches[0].kind), (1, TouchPhase::Began, TouchType::Indirect));
        let e = frame_of(r.plain(true, TouchKind::Move, 0, 14.0, 10.0, 110.0));
        assert_eq!((e.touches[0].phase, e.touches[0].previous_location()), (TouchPhase::Moved, (10.0, 10.0)));
        let e = frame_of(r.plain(true, TouchKind::Up, 0, 14.0, 10.0, 120.0));
        assert_eq!(e.touches[0].phase, TouchPhase::Ended);
        assert!(e.all_finished());
    }

    #[test]
    fn pointer_clicks_count_taps() {
        let mut r = TouchRouter::new(1.0);
        r.plain(true, TouchKind::Down, 0, 50.0, 50.0, 0.0);
        r.plain(true, TouchKind::Up, 0, 50.0, 50.0, 40.0);
        let e = frame_of(r.plain(true, TouchKind::Down, 0, 50.0, 50.0, 150.0));
        assert_eq!(e.touches[0].tap_count, 2);
    }

    #[test]
    fn a_cancel_and_a_stray_up_are_handled() {
        let mut r = TouchRouter::new(1.0);
        assert_eq!(r.plain(true, TouchKind::Up, 3, 0.0, 0.0, 0.0), Plain::Skip, "up without down");
        assert_eq!(r.plain(true, TouchKind::Cancel, 3, 0.0, 0.0, 1.0), Plain::Skip, "cancel without down");
        r.plain(true, TouchKind::Down, 3, 5.0, 6.0, 2.0);
        let e = frame_of(r.plain(true, TouchKind::Cancel, 3, 99.0, 99.0, 3.0));
        assert_eq!((e.touches[0].phase, e.touches[0].location()), (TouchPhase::Cancelled, (5.0, 6.0)));
    }

    #[test]
    fn frames_replace_the_plain_stream_for_apps_with_events() {
        let mut r = TouchRouter::new(1.0);
        r.plain(true, TouchKind::Down, 0, 1.0, 1.0, 0.0);
        let mut tr = TouchTracker::new();
        let ev = tr.down(Sample::at(4, 20.0, 30.0), 5.0).unwrap();
        assert_eq!(r.frame(true, ev.clone()), Some(ev));
        assert!(r.frames_seen());
        assert_eq!(r.plain(true, TouchKind::Move, 0, 2.0, 2.0, 6.0), Plain::Skip);
        assert_eq!(r.cancel_all(true, 7.0), None, "the shell owns the touches now");
    }

    #[test]
    fn leaving_the_foreground_cancels_plain_touches() {
        let mut r = TouchRouter::new(1.0);
        assert_eq!(r.cancel_all(true, 0.0), None);
        r.plain(true, TouchKind::Down, 0, 1.0, 1.0, 0.0);
        let e = r.cancel_all(true, 5.0).unwrap();
        assert_eq!(e.touches[0].phase, TouchPhase::Cancelled);
        assert_eq!(r.cancel_all(false, 6.0), None);
    }

    #[test]
    fn the_scale_sets_the_tap_slop() {
        let mut r = TouchRouter::new(1.0);
        r.set_scale(2.0);
        r.plain(true, TouchKind::Down, 0, 0.0, 0.0, 0.0);
        r.plain(true, TouchKind::Up, 0, 20.0, 0.0, 10.0);
        let e = frame_of(r.plain(true, TouchKind::Down, 0, 20.0, 0.0, 50.0));
        assert_eq!(e.touches[0].tap_count, 2, "20 px is 10 pt at scale 2");
    }
}
