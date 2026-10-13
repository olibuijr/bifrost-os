//! The frame clock: a monotonic media time and per-frame links apps attach to a window.
//!
//! [`media_time`] is seconds on a monotonic clock that starts near zero at the first call. A [`FrameLink`] is called
//! once per display update with a [`FrameTick`] (when this update happens, when the next is due, the interval). The
//! window drives its links from `Window::tick` and asks for wake-ups at the fastest link's interval. A link's
//! [`FrameRateRange`] says the rate it wants; the window resolves it against its display rate.

use bifrost_ui::frame_rate::FrameRateRange;
use std::sync::OnceLock;
use std::time::Instant;

/// Seconds on a monotonic clock (never goes back, unaffected by wall-clock changes).
pub fn media_time() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// One display update, as handed to a link's callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTick {
    /// Time of this update, seconds.
    pub timestamp: f64,
    /// Time the next update is due, seconds.
    pub target_timestamp: f64,
    /// Seconds between updates at the link's resolved rate.
    pub duration: f64,
}

/// A callback tied to display updates.
pub struct FrameLink {
    callback: Box<dyn FnMut(&FrameTick)>,
    pub paused: bool,
    pub preferred_frame_rate_range: FrameRateRange,
    valid: bool,
    last: Option<f64>,
}

impl FrameLink {
    pub fn new(callback: impl FnMut(&FrameTick) + 'static) -> FrameLink {
        FrameLink { callback: Box::new(callback), paused: false, preferred_frame_rate_range: FrameRateRange::DEFAULT, valid: true, last: None }
    }
    /// Stops the link for good; the window drops it at its next tick.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }
    pub fn is_valid(&self) -> bool {
        self.valid
    }
}

/// Handle of a link added to a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinkId(u64);

/// The links of one window and the display rate they resolve against.
pub struct FrameLinks {
    links: Vec<(LinkId, FrameLink)>,
    next: u64,
    pub display_hz: f32,
}

impl Default for FrameLinks {
    fn default() -> Self {
        FrameLinks { links: Vec::new(), next: 1, display_hz: 60.0 }
    }
}

impl FrameLinks {
    pub fn add(&mut self, link: FrameLink) -> LinkId {
        let id = LinkId(self.next);
        self.next += 1;
        self.links.push((id, link));
        id
    }
    pub fn remove(&mut self, id: LinkId) -> Option<FrameLink> {
        let i = self.links.iter().position(|(l, _)| *l == id)?;
        Some(self.links.remove(i).1)
    }
    pub fn get_mut(&mut self, id: LinkId) -> Option<&mut FrameLink> {
        self.links.iter_mut().find(|(l, _)| *l == id).map(|(_, k)| k)
    }
    fn interval_of(&self, l: &FrameLink) -> f64 {
        l.preferred_frame_rate_range.interval(self.display_hz).unwrap_or(1.0 / 60.0)
    }
    /// Seconds to the next wake the active links need, `None` when none is running.
    pub fn wake_interval(&self) -> Option<f64> {
        self.links.iter().filter(|(_, l)| l.valid && !l.paused).map(|(_, l)| self.interval_of(l)).fold(None, |a: Option<f64>, i| Some(a.map_or(i, |a| a.min(i))))
    }
    /// Calls every running link that is due at `now`; drops invalidated ones. Returns how many were called.
    pub fn fire(&mut self, now: f64) -> usize {
        self.links.retain(|(_, l)| l.valid);
        let hz = self.display_hz;
        let mut n = 0;
        for (_, l) in self.links.iter_mut().filter(|(_, l)| !l.paused) {
            let interval = l.preferred_frame_rate_range.interval(hz).unwrap_or(1.0 / 60.0);
            // a frame early by less than a quarter interval still counts as due (ticks jitter)
            if l.last.is_some_and(|last| now - last < interval * 0.75) {
                continue;
            }
            l.last = Some(now);
            (l.callback)(&FrameTick { timestamp: now, target_timestamp: now + interval, duration: interval });
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn media_time_never_goes_back() {
        let a = media_time();
        assert!(media_time() >= a);
    }

    #[test]
    fn a_link_is_called_at_its_rate() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let mut links = FrameLinks::default();
        let mut link = FrameLink::new(move |t| l.borrow_mut().push(*t));
        link.preferred_frame_rate_range = FrameRateRange::fixed(30.0);
        links.add(link);
        for i in 0..7 {
            links.fire(i as f64 / 60.0);
        }
        let log = log.borrow();
        assert_eq!(log.len(), 4, "every other 60 Hz tick");
        assert!((log[0].duration - 1.0 / 30.0).abs() < 1e-9);
        assert!((log[0].target_timestamp - log[0].timestamp - log[0].duration).abs() < 1e-9);
    }

    #[test]
    fn paused_and_invalid_links_do_not_run() {
        let n = Rc::new(RefCell::new(0));
        let c = n.clone();
        let mut links = FrameLinks::default();
        let id = links.add(FrameLink::new(move |_| *c.borrow_mut() += 1));
        assert_eq!(links.wake_interval(), Some(1.0 / 60.0));
        links.get_mut(id).unwrap().paused = true;
        assert_eq!((links.fire(0.0), links.wake_interval()), (0, None));
        links.get_mut(id).unwrap().paused = false;
        assert_eq!(links.fire(1.0), 1);
        links.get_mut(id).unwrap().invalidate();
        assert_eq!(links.fire(2.0), 0);
        assert!(links.remove(id).is_none(), "an invalid link was dropped");
        assert_eq!(*n.borrow(), 1);
    }

    #[test]
    fn the_fastest_link_sets_the_wake() {
        let mut links = FrameLinks::default();
        links.display_hz = 120.0;
        let mut a = FrameLink::new(|_| {});
        a.preferred_frame_rate_range = FrameRateRange::fixed(30.0);
        links.add(a);
        links.add(FrameLink::new(|_| {}));
        assert_eq!(links.wake_interval(), Some(1.0 / 120.0));
    }
}
