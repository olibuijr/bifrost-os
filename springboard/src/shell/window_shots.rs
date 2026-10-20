//! Pictures of windows the window server composes (`bifrost_window_capture_v1`, docs/window-server.md "Window capture"):
//! Overview tiles for other clients' windows, the Dock's thumbnails of minimized windows and window screenshots.
//!
//! The shell only sees the pixels of the windows it draws itself; a window of another Wayland client (every store app
//! on the direct path, every Linux app) is composed by the window server. This model holds what the shell asked for and
//! got: the newest picture of each window, which requests are in flight, and when each window was last asked about, so
//! a consumer's cadence is a number it passes in rather than a timer of its own:
//!
//! - Overview refreshes the tiles it shows at most [`MISSION_REFRESH_MS`] apart (5 a second) while it is open.
//! - The Dock asks once, when a window is minimized (`request_now`).
//! - A window screenshot asks once for the full picture (`request_now`).
//!
//! A window the window server cannot capture (the Vulkan scanout, a window that is gone) is not asked about again for
//! [`RETRY_AFTER_FAIL_MS`], and every consumer keeps its fallback (the icon in the thumbnail frame, the app icon on a
//! tile, the composed-screen cut) when no picture is there. The window server answers `unchanged` for a window that has
//! not committed since the last picture, so a refresh of an idle window moves no pixels.
//!
//! This module has no Wayland types: the Wayland backend (`bare/wayland.rs`) takes the [`Request`]s, sends them and hands
//! back [`Outcome`]s.

use crate::gfx::Image;
use std::collections::HashMap;

/// Overview refreshes a tile at most this often while it is open: 5 times a second.
pub const MISSION_REFRESH_MS: f64 = 200.0;
/// A window that failed to capture is not asked about again for this long.
pub const RETRY_AFTER_FAIL_MS: f64 = 5_000.0;
/// A request that has not been answered after this long is forgotten (the window server never answers a dead
/// connection), so its window can be asked about again.
pub const INFLIGHT_TIMEOUT_MS: f64 = 4_000.0;
/// No side of a requested picture is larger than this, whatever a consumer asks for (a 64 MB buffer at most).
pub const MAX_SIDE: u32 = 4096;

/// Which window a picture is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A window of another client, by the id of its foreign-toplevel entry (`ForeignWindow::id`).
    Foreign(u32),
    /// A window the shell draws itself, by its presenter id (`desk::win_id`): the shell names its own surface.
    Surface(u32),
}

/// One capture the Wayland backend is to send: the window, the largest picture wanted and the serial of the picture
/// the shell holds (0: none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub token: u64,
    pub source: Source,
    pub max: (u32, u32),
    pub since: u32,
}

/// What came back for a request.
#[derive(Debug)]
pub enum Outcome {
    /// The window as a premultiplied ARGB picture, at most `max`, and the serial that names it.
    Ready { image: Image, serial: u32 },
    /// The window did not change since the picture the shell holds.
    Unchanged { serial: u32 },
    /// No picture: the window is gone, the window server cannot capture, or the shell is not allowed to.
    Failed,
}

/// What `finish` made of an answer, for the consumer that asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// A new picture is stored (`shot`).
    Fresh,
    /// The stored picture is still current.
    Same,
    /// There is no picture.
    Failed,
    /// The answer is for a request the shell no longer waits for.
    Unknown,
}

/// The newest picture of a window.
#[derive(Debug)]
pub struct Shot {
    pub image: Image,
    pub serial: u32,
    /// The largest picture that was asked for; a later request for the same bound may be answered `unchanged`.
    pub max: (u32, u32),
    /// When it was taken (the shell's clock, ms).
    pub at: f64,
}

/// The pictures the shell holds and the requests it has out.
#[derive(Default)]
pub struct WindowShots {
    available: bool,
    shots: HashMap<Source, Shot>,
    /// requests out by token: the window, when it was asked and the bound it was asked at
    inflight: HashMap<u64, (Source, f64, (u32, u32))>,
    asked: HashMap<Source, f64>,
    failed: HashMap<Source, f64>,
    queue: Vec<Request>,
    next_token: u64,
}

/// A request's bound, kept to sensible sizes: each side between 1 and [`MAX_SIDE`].
pub fn clamp_max(max: (u32, u32)) -> (u32, u32) {
    (max.0.clamp(1, MAX_SIDE), max.1.clamp(1, MAX_SIDE))
}

impl WindowShots {
    /// Whether the window server offers window capture (the global is bound). Without it nothing is asked.
    pub fn set_available(&mut self, available: bool) {
        self.available = available;
        if !available {
            self.queue.clear();
        }
    }

    pub fn available(&self) -> bool {
        self.available
    }

    fn issue(&mut self, source: Source, max: (u32, u32), since: u32, now: f64) -> u64 {
        self.next_token += 1;
        let token = self.next_token;
        self.queue.push(Request { token, source, max, since });
        self.inflight.insert(token, (source, now, max));
        self.asked.insert(source, now);
        token
    }

    /// The serial to quote for `source` asked at `max`: the held picture's, when it was taken at that bound.
    fn since(&self, source: Source, max: (u32, u32)) -> u32 {
        self.shots.get(&source).filter(|s| s.max == max).map_or(0, |s| s.serial)
    }

    fn waiting(&self, source: Source, now: f64) -> bool {
        self.inflight.values().any(|(s, at, _)| *s == source && now - *at < INFLIGHT_TIMEOUT_MS)
    }

    /// Asks for a refreshed picture of `source` unless one was asked for less than `min_gap_ms` ago, one is on its way,
    /// or the window failed recently. Returns the token when a request was queued. A window that has not changed
    /// since the held picture costs the window server nothing (it answers `unchanged`).
    pub fn request(&mut self, source: Source, max: (u32, u32), now: f64, min_gap_ms: f64) -> Option<u64> {
        if !self.available || self.waiting(source, now) {
            return None;
        }
        if self.asked.get(&source).is_some_and(|at| now - *at < min_gap_ms) {
            return None;
        }
        if self.failed.get(&source).is_some_and(|at| now - *at < RETRY_AFTER_FAIL_MS) {
            return None;
        }
        let max = clamp_max(max);
        let since = self.since(source, max);
        Some(self.issue(source, max, since, now))
    }

    /// Asks for a picture of `source` now, whatever was asked before and whether it failed recently (the Dock when a
    /// window is minimized, a screenshot): a new picture, not an `unchanged` answer.
    pub fn request_now(&mut self, source: Source, max: (u32, u32), now: f64) -> Option<u64> {
        if !self.available {
            return None;
        }
        self.failed.remove(&source);
        let max = clamp_max(max);
        Some(self.issue(source, max, 0, now))
    }

    /// The requests queued since the last call, for the Wayland backend to send.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.queue)
    }

    /// Whether requests are queued (so the loop does not wait).
    pub fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Takes an answer: a new picture replaces the old one, `unchanged` keeps it (and marks it current), a failure
    /// keeps the old picture and stops asking for a while. The source it was for comes back with what happened.
    pub fn finish(&mut self, token: u64, outcome: Outcome, now: f64) -> (Option<Source>, Done) {
        let Some((source, _, max)) = self.inflight.remove(&token) else { return (None, Done::Unknown) };
        let done = match outcome {
            Outcome::Ready { image, serial } => {
                self.shots.insert(source, Shot { image, serial, max, at: now });
                self.failed.remove(&source);
                Done::Fresh
            }
            Outcome::Unchanged { serial } => match self.shots.get_mut(&source) {
                Some(shot) if shot.serial == serial => {
                    shot.at = now;
                    Done::Same
                }
                // an answer about a picture the shell no longer holds: ask again for a whole one
                _ => {
                    self.asked.remove(&source);
                    Done::Failed
                }
            },
            Outcome::Failed => {
                self.failed.insert(source, now);
                Done::Failed
            }
        };
        (Some(source), done)
    }

    pub fn shot(&self, source: Source) -> Option<&Shot> {
        self.shots.get(&source)
    }

    /// Whether a request for `source` is still out.
    pub fn is_waiting(&self, source: Source, now: f64) -> bool {
        self.waiting(source, now)
    }

    /// Forgets the pictures and history of every window `keep` says is gone (closed windows free their picture).
    pub fn retain(&mut self, keep: impl Fn(&Source) -> bool) {
        self.shots.retain(|s, _| keep(s));
        self.asked.retain(|s, _| keep(s));
        self.failed.retain(|s, _| keep(s));
        self.inflight.retain(|_, (s, _, _)| keep(s));
    }

    /// Drops the request log of answers that never came (a stuck connection), so the windows can be asked about again.
    pub fn expire(&mut self, now: f64) {
        self.inflight.retain(|_, (_, at, _)| now - *at < INFLIGHT_TIMEOUT_MS);
    }

    /// How many pictures are held (tests and the memory budget).
    pub fn held(&self) -> usize {
        self.shots.len()
    }
}

use super::Shell;

/// How often the pictures of closed windows are dropped (ms).
const GC_MS: f64 = 2_000.0;

impl Shell {
    /// Whether the window server offers window capture (the Wayland backend says so each frame).
    pub fn set_window_capture(&mut self, available: bool) {
        if self.shots.available() != available {
            self.shots.set_available(available);
        }
    }

    /// The captures queued since the last call, for the Wayland backend to send (before it publishes the next frames, so
    /// the window server still has a window that is about to be minimized or closed).
    pub fn take_capture_requests(&mut self) -> Vec<Request> {
        self.shots.take_requests()
    }

    /// The window server's answers to window captures: (token, outcome).
    pub fn window_capture_results(&mut self, results: Vec<(u64, Outcome)>, t: f64) {
        for (token, outcome) in results {
            let (source, done) = self.shots.finish(token, outcome, t);
            // a window screenshot waiting for it is made, or falls back
            self.window_grab_answer(token, source, done, t);
            if done == Done::Fresh {
                self.overview_shots_changed();
                self.dirty = true;
            }
        }
    }

    /// Housekeeping of the pictures: requests nobody answered are given up, Overview refreshes its tiles (at most 5
    /// a second), and the pictures of windows that closed are freed.
    pub(super) fn shots_tick(&mut self, t: f64) {
        self.shots.expire(t);
        self.overview_refresh_shots(t);
        if t - self.shots_gc >= GC_MS {
            self.shots_gc = t;
            self.shots_forget_closed();
        }
    }

    /// When the loop must wake for the pictures: while Overview shows windows of other clients, to refresh them.
    pub(super) fn shots_wake(&self, t: f64) -> Option<f64> {
        self.overview_wants_shots().then_some(t + MISSION_REFRESH_MS)
    }

    /// Frees the pictures of windows that are gone: a closed desk window, a window of another client the window server no
    /// longer lists.
    fn shots_forget_closed(&mut self) {
        if self.shots.held() == 0 {
            return;
        }
        let surfaces: Vec<u32> = self.desk.as_ref().map(|d| d.wins.iter().filter_map(|w| super::desk::win_id(w.app)).collect()).unwrap_or_default();
        let foreign: Vec<u32> = self.mission_foreign_windows().into_iter().map(|(id, _)| id).collect();
        self.shots.retain(|s| match s {
            Source::Foreign(id) => foreign.contains(id),
            Source::Surface(id) => surfaces.contains(id),
        });
    }
}

/// The largest picture a tile of `w` by `h` px needs: the tile's size at 2x, so a magnified or resized tile stays
/// sharp, never above [`MAX_SIDE`].
pub fn bound_for_tile(w: f32, h: f32) -> (u32, u32) {
    clamp_max(((w.max(1.0) * 2.0).round() as u32, (h.max(1.0) * 2.0).round() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize) -> Image {
        Image { w, h, px: vec![0xff00_00ff; w * h] }
    }

    fn shots() -> WindowShots {
        let mut s = WindowShots::default();
        s.set_available(true);
        s
    }

    const A: Source = Source::Foreign(1);
    const B: Source = Source::Surface(7);

    #[test]
    fn nothing_is_asked_without_the_window_server_global() {
        let mut s = WindowShots::default();
        assert!(s.request(A, (100, 100), 0.0, MISSION_REFRESH_MS).is_none());
        assert!(s.request_now(A, (100, 100), 0.0).is_none());
        assert!(s.take_requests().is_empty());
    }

    #[test]
    fn mission_control_asks_at_most_five_times_a_second() {
        let mut s = shots();
        let t1 = s.request(A, (400, 300), 1000.0, MISSION_REFRESH_MS).expect("the first ask goes out");
        assert_eq!(s.finish(t1, Outcome::Ready { image: image(400, 250), serial: 3 }, 1010.0), (Some(A), Done::Fresh));
        assert!(s.request(A, (400, 300), 1100.0, MISSION_REFRESH_MS).is_none(), "100 ms after the last ask is too soon");
        assert!(s.request(A, (400, 300), 1199.0, MISSION_REFRESH_MS).is_none());
        assert!(s.request(A, (400, 300), 1200.0, MISSION_REFRESH_MS).is_some(), "200 ms later it may ask again");
        let sent: Vec<f64> = (0..50).filter_map(|i| {
            let now = 2000.0 + i as f64 * 20.0;
            let mut probe = shots();
            probe.asked.insert(A, 2000.0 - 1.0);
            probe.request(A, (400, 300), now, MISSION_REFRESH_MS).map(|_| now)
        }).collect();
        assert!(sent.first().is_some_and(|&t| t >= 2199.0), "{sent:?}");
    }

    #[test]
    fn a_window_with_a_request_on_its_way_is_not_asked_twice() {
        let mut s = shots();
        assert!(s.request(A, (100, 100), 0.0, 0.0).is_some());
        assert!(s.request(A, (100, 100), 50.0, 0.0).is_none(), "the first has not been answered");
        assert!(s.request(B, (100, 100), 50.0, 0.0).is_some(), "another window is independent");
        assert_eq!(s.take_requests().len(), 2);
    }

    #[test]
    fn a_request_nobody_answers_is_given_up_so_the_window_can_be_asked_again() {
        let mut s = shots();
        assert!(s.request(A, (100, 100), 0.0, 0.0).is_some());
        assert!(s.request(A, (100, 100), INFLIGHT_TIMEOUT_MS - 1.0, 0.0).is_none());
        assert!(s.request(A, (100, 100), INFLIGHT_TIMEOUT_MS + 1.0, 0.0).is_some());
    }

    #[test]
    fn an_unchanged_window_keeps_its_picture_and_the_next_ask_quotes_its_serial() {
        let mut s = shots();
        let t1 = s.request(A, (400, 300), 0.0, 0.0).unwrap();
        s.finish(t1, Outcome::Ready { image: image(400, 250), serial: 9 }, 5.0);
        // the backend took the first request (it left the queue), then a second ask quotes the serial 9
        s.take_requests();
        let t2 = s.request(A, (400, 300), 300.0, MISSION_REFRESH_MS).unwrap();
        let sent = s.take_requests();
        assert_eq!(sent, vec![Request { token: t2, source: A, max: (400, 300), since: 9 }]);
        assert_eq!(s.finish(t2, Outcome::Unchanged { serial: 9 }, 310.0), (Some(A), Done::Same));
        assert_eq!(s.shot(A).map(|x| x.serial), Some(9));
        assert_eq!(s.shot(A).map(|x| x.at), Some(310.0));
    }

    #[test]
    fn a_different_bound_is_asked_for_a_whole_picture_not_quoted() {
        let mut s = shots();
        let t1 = s.request(A, (400, 300), 0.0, 0.0).unwrap();
        s.take_requests();
        s.finish(t1, Outcome::Ready { image: image(400, 250), serial: 9 }, 5.0);
        s.request(A, (800, 600), 500.0, 0.0).unwrap();
        assert_eq!(s.take_requests()[0].since, 0, "the held picture was taken for another bound");
    }

    #[test]
    fn request_now_always_asks_for_a_whole_picture() {
        let mut s = shots();
        let t1 = s.request_now(B, (256, 256), 0.0).unwrap();
        s.take_requests();
        s.finish(t1, Outcome::Ready { image: image(256, 128), serial: 4 }, 1.0);
        s.request_now(B, (256, 256), 2.0).expect("a minimize asks right away");
        assert_eq!(s.take_requests()[0].since, 0);
    }

    #[test]
    fn a_failure_keeps_the_old_picture_and_pauses_the_window() {
        let mut s = shots();
        let t1 = s.request(A, (100, 100), 0.0, 0.0).unwrap();
        s.take_requests();
        s.finish(t1, Outcome::Ready { image: image(100, 50), serial: 2 }, 1.0);
        let t2 = s.request(A, (100, 100), 300.0, 0.0).unwrap();
        assert_eq!(s.finish(t2, Outcome::Failed, 310.0), (Some(A), Done::Failed));
        assert!(s.shot(A).is_some(), "the last good picture stays for the consumer to show");
        assert!(s.request(A, (100, 100), 400.0, 0.0).is_none(), "not asked again at once");
        assert!(s.request(A, (100, 100), 310.0 + RETRY_AFTER_FAIL_MS + 1.0, 0.0).is_some());
        // a screenshot or a minimize asks regardless
        assert!(s.request_now(A, (100, 100), 500.0).is_some());
    }

    #[test]
    fn an_answer_nobody_waits_for_is_ignored() {
        let mut s = shots();
        assert_eq!(s.finish(42, Outcome::Failed, 0.0), (None, Done::Unknown));
        assert_eq!(s.held(), 0);
    }

    #[test]
    fn an_unchanged_answer_for_a_picture_not_held_asks_again() {
        let mut s = shots();
        let t = s.request(A, (100, 100), 0.0, 0.0).unwrap();
        assert_eq!(s.finish(t, Outcome::Unchanged { serial: 5 }, 1.0), (Some(A), Done::Failed));
        assert!(s.request(A, (100, 100), 2.0, 200.0).is_some(), "no wait: the shell holds nothing");
    }

    #[test]
    fn closed_windows_free_their_pictures() {
        let mut s = shots();
        for (i, src) in [A, B].into_iter().enumerate() {
            let t = s.request_now(src, (50, 50), 0.0).unwrap();
            s.finish(t, Outcome::Ready { image: image(50, 25), serial: 1 + i as u32 }, 1.0);
        }
        assert_eq!(s.held(), 2);
        s.retain(|src| *src == B);
        assert_eq!(s.held(), 1);
        assert!(s.shot(A).is_none() && s.shot(B).is_some());
    }

    #[test]
    fn losing_the_global_drops_what_was_queued() {
        let mut s = shots();
        s.request(A, (10, 10), 0.0, 0.0);
        s.set_available(false);
        assert!(s.take_requests().is_empty());
    }

    #[test]
    fn requested_sizes_stay_sensible() {
        assert_eq!(clamp_max((0, 0)), (1, 1));
        assert_eq!(clamp_max((100_000, 50)), (MAX_SIDE, 50));
        assert_eq!(bound_for_tile(300.0, 200.0), (600, 400), "twice the tile");
        assert_eq!(bound_for_tile(0.0, 0.0), (2, 2));
        assert_eq!(bound_for_tile(10_000.0, 10.0), (MAX_SIDE, 20));
    }
}
