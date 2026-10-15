//! Name-keyed notifications with integer tokens: polled checks, per-name state values and an optional datagram
//! per token. A token is a handle into one `NotifyCenter`. A post to a name marks every live, unsuspended token on
//! that name as pending, and `check` reads and clears that mark. A signal token queues its signal number on each post,
//! and `take_signals` hands the queue to the caller, which raises the signals itself: no kernel signal path is used.
//! `SYSTEM_EVENTS` fixes the spelling of the system change names the shell may post.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::os::unix::net::UnixDatagram;

/// The time zone changed.
pub const TIME_ZONE_CHANGE: &str = "system.timezone.change";
/// The wall clock was set.
pub const CLOCK_SET: &str = "system.clock.set";
/// A volume was mounted.
pub const VOLUME_MOUNT: &str = "system.volume.mount";
/// A volume was unmounted.
pub const VOLUME_UNMOUNT: &str = "system.volume.unmount";
/// A volume is nearly full.
pub const VOLUME_LOW_DISK_SPACE: &str = "system.volume.low-disk-space";
/// The network connection went up or down.
pub const NETWORK_CHANGE: &str = "system.network.change";
/// The contacts store changed.
pub const CONTACTS_CHANGE: &str = "system.contacts.change";
/// The events store changed.
pub const CALENDAR_CHANGE: &str = "system.calendar.change";
/// The host name changed.
pub const HOSTNAME_CHANGE: &str = "system.hostname.change";
/// Cached disk data is stale.
pub const DISK_CACHE_INVALIDATE: &str = "system.disk-cache.invalidate";
/// The log database was updated.
pub const LOG_DB_UPDATE: &str = "system.log-db.update";

/// Names for system changes (time zone, clock, volumes, network, host name, caches). The shell decides when to post them.
pub const SYSTEM_EVENTS: [&str; 11] = [
    TIME_ZONE_CHANGE,
    CLOCK_SET,
    VOLUME_MOUNT,
    VOLUME_UNMOUNT,
    VOLUME_LOW_DISK_SPACE,
    NETWORK_CHANGE,
    CONTACTS_CHANGE,
    CALENDAR_CHANGE,
    HOSTNAME_CHANGE,
    DISK_CACHE_INVALIDATE,
    LOG_DB_UPDATE,
];

/// The system name for a change in the number of mounted volumes, or `None` when the count is the same. The count
/// is the only thing compared, so a mount and an unmount between two looks cancel out.
pub fn volume_change(before: usize, after: usize) -> Option<&'static str> {
    match after.cmp(&before) {
        std::cmp::Ordering::Greater => Some(VOLUME_MOUNT),
        std::cmp::Ordering::Less => Some(VOLUME_UNMOUNT),
        std::cmp::Ordering::Equal => None,
    }
}

#[derive(Debug)]
struct Registration {
    name: String,
    pending: bool,
    suspended: bool,
    /// Sending end of a datagram pair, present for tokens made with `register_fd`.
    sink: Option<UnixDatagram>,
    /// Signal number queued on each post, present for tokens made with `register_signal`.
    signal: Option<i32>,
}

/// Largest signal number a signal token may name. The range is 1 to this value, inclusive.
pub const MAX_SIGNAL: i32 = 64;
/// Most queued signals kept between two `take_signals` calls. A caller that stops reading must not grow the queue
/// without bound; past this, the oldest signals are dropped (the pending marks still record every post).
pub const MAX_QUEUED_SIGNALS: usize = 1024;

/// The table of registrations and the state values. Not shared between threads; a caller wraps it in its own lock.
#[derive(Debug, Default)]
pub struct NotifyCenter {
    last: u32,
    tokens: BTreeMap<u32, Registration>,
    states: HashMap<String, u64>,
    /// Signals queued by posts, in post order, until `take_signals` reads them.
    signals: Vec<i32>,
}

impl NotifyCenter {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, sink: Option<UnixDatagram>, signal: Option<i32>) -> u32 {
        // Zero is never handed out, and the counter starts again at one if it ever wraps.
        self.last = self.last.checked_add(1).unwrap_or(1);
        let token = self.last;
        self.tokens.insert(
            token,
            Registration { name: name.to_string(), pending: false, suspended: false, sink, signal },
        );
        token
    }

    /// Registers a name for polling with `check`.
    pub fn register(&mut self, name: &str) -> u32 {
        self.add(name, None, None)
    }

    /// Registers a name for signal delivery. Each post to the name queues `signal` for the caller, read with
    /// `take_signals`. The number must be from 1 to `MAX_SIGNAL`, or the call fails with `InvalidInput`.
    pub fn register_signal(&mut self, name: &str, signal: i32) -> io::Result<u32> {
        if !(1..=MAX_SIGNAL).contains(&signal) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "signal number out of range"));
        }
        Ok(self.add(name, None, Some(signal)))
    }

    /// Returns the signals queued by posts since the last call, in post order, and clears the queue. Signals
    /// queued before a token is cancelled stay in the queue.
    pub fn take_signals(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.signals)
    }

    /// Registers a name and returns the token with the reading end of a datagram pair. Each delivered post writes
    /// one byte to that end, so the reader can wait on it.
    pub fn register_fd(&mut self, name: &str) -> io::Result<(u32, UnixDatagram)> {
        let (sink, reader) = UnixDatagram::pair()?;
        sink.set_nonblocking(true)?;
        Ok((self.add(name, Some(sink), None), reader))
    }

    pub fn is_valid(&self, token: u32) -> bool {
        self.tokens.contains_key(&token)
    }

    /// Removes the token. Its name stops receiving posts through it.
    pub fn cancel(&mut self, token: u32) -> bool {
        self.tokens.remove(&token).is_some()
    }

    /// Stops delivery to the token. Posts that arrive while it is suspended are not kept.
    pub fn suspend(&mut self, token: u32) -> bool {
        self.set_suspended(token, true)
    }

    pub fn resume(&mut self, token: u32) -> bool {
        self.set_suspended(token, false)
    }

    fn set_suspended(&mut self, token: u32, on: bool) -> bool {
        match self.tokens.get_mut(&token) {
            Some(reg) => {
                reg.suspended = on;
                true
            }
            None => false,
        }
    }

    /// Returns whether the token was posted since its last check, and clears the mark.
    pub fn check(&mut self, token: u32) -> bool {
        match self.tokens.get_mut(&token) {
            Some(reg) => std::mem::take(&mut reg.pending),
            None => false,
        }
    }

    /// Posts a name. Returns how many tokens received it. Suspended tokens are skipped.
    pub fn post(&mut self, name: &str) -> usize {
        let mut count = 0;
        for reg in self.tokens.values_mut() {
            if reg.name != name || reg.suspended {
                continue;
            }
            reg.pending = true;
            count += 1;
            if let Some(signal) = reg.signal {
                self.signals.push(signal);
            }
            if let Some(sink) = &reg.sink {
                // A full buffer means the reader is behind. The pending mark is still set, so nothing is lost.
                let _ = sink.send(&[1]);
            }
        }
        if self.signals.len() > MAX_QUEUED_SIGNALS {
            let excess = self.signals.len() - MAX_QUEUED_SIGNALS;
            self.signals.drain(..excess);
        }
        count
    }

    /// Sets the state value of a name. The value is shared by every token on that name.
    pub fn set_state(&mut self, name: &str, state: u64) {
        self.states.insert(name.to_string(), state);
    }

    /// Returns the state value of a name, or zero when none was set.
    pub fn state(&self, name: &str) -> u64 {
        self.states.get(name).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_start_at_one_and_are_unique() {
        let mut c = NotifyCenter::new();
        let a = c.register("a");
        let b = c.register("a");
        assert_eq!(a, 1);
        assert_ne!(a, b);
        assert!(c.is_valid(a) && c.is_valid(b));
    }

    #[test]
    fn check_reports_a_post_once_and_clears_it() {
        let mut c = NotifyCenter::new();
        let t = c.register("system.clock.set");
        assert!(!c.check(t));
        assert_eq!(c.post("system.clock.set"), 1);
        assert!(c.check(t));
        assert!(!c.check(t));
    }

    #[test]
    fn a_post_reaches_only_tokens_on_that_name() {
        let mut c = NotifyCenter::new();
        let mine = c.register("x");
        let other = c.register("y");
        assert_eq!(c.post("x"), 1);
        assert!(c.check(mine));
        assert!(!c.check(other));
        assert_eq!(c.post("nobody"), 0);
    }

    #[test]
    fn suspended_tokens_miss_posts_and_resume_restores_delivery() {
        let mut c = NotifyCenter::new();
        let t = c.register("n");
        assert!(c.suspend(t));
        assert_eq!(c.post("n"), 0);
        assert!(!c.check(t));
        assert!(c.resume(t));
        assert_eq!(c.post("n"), 1);
        assert!(c.check(t));
    }

    #[test]
    fn cancel_invalidates_the_token() {
        let mut c = NotifyCenter::new();
        let t = c.register("n");
        assert!(c.cancel(t));
        assert!(!c.is_valid(t));
        assert!(!c.cancel(t));
        assert_eq!(c.post("n"), 0);
        assert!(!c.suspend(t));
    }

    #[test]
    fn state_is_per_name_and_defaults_to_zero() {
        let mut c = NotifyCenter::new();
        assert_eq!(c.state("n"), 0);
        c.set_state("n", 42);
        c.set_state("m", 7);
        assert_eq!(c.state("n"), 42);
        assert_eq!(c.state("m"), 7);
    }

    #[test]
    fn fd_token_gets_one_byte_per_post() {
        let mut c = NotifyCenter::new();
        let (t, reader) = c.register_fd("n").expect("socket pair");
        reader.set_nonblocking(true).expect("nonblocking reader");
        assert_eq!(c.post("n"), 1);
        assert_eq!(c.post("n"), 1);
        let mut buf = [0u8; 4];
        assert_eq!(reader.recv(&mut buf).expect("first datagram"), 1);
        assert_eq!(reader.recv(&mut buf).expect("second datagram"), 1);
        assert!(c.check(t));
    }

    #[test]
    fn system_event_names_are_unique_and_dotted() {
        for (i, a) in SYSTEM_EVENTS.iter().enumerate() {
            assert!(a.starts_with("system."));
            for b in &SYSTEM_EVENTS[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn signal_numbers_must_be_from_one_to_sixty_four() {
        let mut c = NotifyCenter::new();
        assert!(c.register_signal("n", 0).is_err());
        assert!(c.register_signal("n", -1).is_err());
        assert!(c.register_signal("n", 65).is_err());
        assert_eq!(c.register_signal("n", 1).expect("one is valid"), 1);
        assert_eq!(c.register_signal("n", 64).expect("sixty-four is valid"), 2);
    }

    #[test]
    fn each_post_queues_one_signal_per_signal_token_in_token_order() {
        let mut c = NotifyCenter::new();
        let a = c.register_signal("n", 10).expect("valid");
        let b = c.register_signal("n", 12).expect("valid");
        let plain = c.register("n");
        assert_eq!((a, b, plain), (1, 2, 3));
        assert_eq!(c.post("n"), 3);
        assert_eq!(c.take_signals(), vec![10, 12]);
        assert!(c.take_signals().is_empty());
    }

    #[test]
    fn repeated_posts_queue_repeated_signals_and_keep_the_pending_mark() {
        let mut c = NotifyCenter::new();
        let t = c.register_signal("n", 10).expect("valid");
        c.post("n");
        c.post("n");
        assert_eq!(c.take_signals(), vec![10, 10]);
        assert!(c.check(t));
        assert!(!c.check(t));
    }

    #[test]
    fn suspended_signal_tokens_queue_nothing() {
        let mut c = NotifyCenter::new();
        let t = c.register_signal("n", 10).expect("valid");
        assert!(c.suspend(t));
        assert_eq!(c.post("n"), 0);
        assert!(c.take_signals().is_empty());
        assert!(c.resume(t));
        c.post("n");
        assert_eq!(c.take_signals(), vec![10]);
    }

    #[test]
    fn a_post_to_another_name_queues_no_signal() {
        let mut c = NotifyCenter::new();
        c.register_signal("x", 10).expect("valid");
        assert_eq!(c.post("y"), 0);
        assert!(c.take_signals().is_empty());
    }

    #[test]
    fn an_unread_signal_queue_stays_bounded_and_keeps_the_newest_signals() {
        let mut c = NotifyCenter::new();
        c.register_signal("n", 1).expect("valid");
        c.register_signal("n", 2).expect("valid");
        for _ in 0..(MAX_QUEUED_SIGNALS * 3) {
            c.post("n");
        }
        assert!(c.signals.len() <= MAX_QUEUED_SIGNALS, "queue holds {}", c.signals.len());
        let got = c.take_signals();
        assert_eq!(got.len(), MAX_QUEUED_SIGNALS);
        assert_eq!(*got.last().expect("non-empty"), 2, "the newest post is kept");
        assert!(c.check(1) && c.check(2), "pending marks still record every post");
    }
}
