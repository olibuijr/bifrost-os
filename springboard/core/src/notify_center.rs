//! Name-keyed notifications with integer tokens: polled checks, per-name state values and an optional datagram
//! per token. A token is a handle into one `NotifyCenter`. A post to a name marks every live, unsuspended token on
//! that name as pending, and `check` reads and clears that mark. Signal delivery is not modelled (it needs a kernel
//! signal path). `SYSTEM_EVENTS` fixes the spelling of the system change names the shell may post.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::os::unix::net::UnixDatagram;

/// Names for system changes (time zone, clock, volumes, network, host name, caches). The shell decides when to post them.
pub const SYSTEM_EVENTS: [&str; 9] = [
    "system.timezone.change",
    "system.clock.set",
    "system.volume.mount",
    "system.volume.unmount",
    "system.volume.low-disk-space",
    "system.network.change",
    "system.hostname.change",
    "system.disk-cache.invalidate",
    "system.log-db.update",
];

#[derive(Debug)]
struct Registration {
    name: String,
    pending: bool,
    suspended: bool,
    /// Sending end of a datagram pair, present for tokens made with `register_fd`.
    sink: Option<UnixDatagram>,
}

/// The table of registrations and the state values. Not shared between threads; a caller wraps it in its own lock.
#[derive(Debug, Default)]
pub struct NotifyCenter {
    last: u32,
    tokens: BTreeMap<u32, Registration>,
    states: HashMap<String, u64>,
}

impl NotifyCenter {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, sink: Option<UnixDatagram>) -> u32 {
        // Zero is never handed out, and the counter starts again at one if it ever wraps.
        self.last = self.last.checked_add(1).unwrap_or(1);
        let token = self.last;
        self.tokens.insert(
            token,
            Registration { name: name.to_string(), pending: false, suspended: false, sink },
        );
        token
    }

    /// Registers a name for polling with `check`.
    pub fn register(&mut self, name: &str) -> u32 {
        self.add(name, None)
    }

    /// Registers a name and returns the token with the reading end of a datagram pair. Each delivered post writes
    /// one byte to that end, so the reader can wait on it.
    pub fn register_fd(&mut self, name: &str) -> io::Result<(u32, UnixDatagram)> {
        let (sink, reader) = UnixDatagram::pair()?;
        sink.set_nonblocking(true)?;
        Ok((self.add(name, Some(sink)), reader))
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
            if let Some(sink) = &reg.sink {
                // A full buffer means the reader is behind. The pending mark is still set, so nothing is lost.
                let _ = sink.send(&[1]);
            }
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
}
