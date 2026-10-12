//! Quantum negotiation: the graph runs at the smallest buffer size any running client asked for, clamped to the
//! device range (64 frames is the floor on HDA), and the change takes effect at a cycle boundary. Owner: task 10.

use std::collections::HashMap;

/// The pro floor on HDA.
pub const FLOOR_FRAMES: u32 = 64;
/// The default quantum when nobody asks (256 frames).
pub const DEFAULT_FRAMES: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quantum {
    range: (u32, u32),
    /// client id -> requested frames (0 = the device's current size, i.e. no preference).
    requests: HashMap<u32, u32>,
    current: u32,
    pending: Option<u32>,
}

impl Quantum {
    pub fn new(range: (u32, u32)) -> Quantum {
        let lo = range.0.max(FLOOR_FRAMES);
        let hi = range.1.max(lo);
        Quantum { range: (lo, hi), requests: HashMap::new(), current: DEFAULT_FRAMES.clamp(lo, hi), pending: None }
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    pub fn pending(&self) -> Option<u32> {
        self.pending
    }

    /// The target for the present requests: the smallest nonzero one, clamped; the default when none.
    pub fn target(&self) -> u32 {
        let want = self.requests.values().copied().filter(|&n| n != 0).min().unwrap_or(DEFAULT_FRAMES);
        want.clamp(self.range.0, self.range.1)
    }

    fn renegotiate(&mut self) {
        let t = self.target();
        self.pending = if t != self.current { Some(t) } else { None };
    }

    /// A client starts (or changes) its request. Returns the pending quantum, if it differs from the current.
    pub fn request(&mut self, client: u32, frames: u32) -> Option<u32> {
        self.requests.insert(client, frames);
        self.renegotiate();
        self.pending
    }

    /// A client stops or leaves.
    pub fn withdraw(&mut self, client: u32) -> Option<u32> {
        self.requests.remove(&client);
        self.renegotiate();
        self.pending
    }

    /// Called by the IO cycle between cycles: applies a pending change; returns the new size when it changed.
    pub fn at_cycle_boundary(&mut self) -> Option<u32> {
        let p = self.pending.take()?;
        self.current = p;
        Some(p)
    }

    /// How many device cycles pass between calls of a client with `client_frames` (a larger buffer is called once
    /// every N cycles; the server accumulates). Rounded up.
    pub fn cycles_per_call(&self, client_frames: u32) -> u32 {
        if client_frames == 0 {
            return 1;
        }
        client_frames.div_ceil(self.current).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smallest_request_wins_at_boundary() {
        let mut q = Quantum::new((32, 4096));
        assert_eq!(q.current(), 256);
        assert_eq!(q.request(1, 1024), Some(1024));
        assert_eq!(q.request(2, 64), Some(64));
        assert_eq!(q.current(), 256);
        assert_eq!(q.at_cycle_boundary(), Some(64));
        assert_eq!(q.current(), 64);
        assert_eq!(q.at_cycle_boundary(), None);
    }

    #[test]
    fn floor_and_clamp() {
        let mut q = Quantum::new((16, 512));
        q.request(1, 16);
        assert_eq!(q.target(), 64);
        q.request(1, 8192);
        assert_eq!(q.target(), 512);
    }

    #[test]
    fn withdraw_returns_to_default_and_zero_means_no_preference() {
        let mut q = Quantum::new((64, 4096));
        q.request(1, 64);
        q.at_cycle_boundary();
        q.request(2, 0);
        assert_eq!(q.target(), 64);
        assert_eq!(q.withdraw(1), Some(256));
        q.at_cycle_boundary();
        assert_eq!(q.current(), 256);
        assert_eq!(q.withdraw(2), None);
    }

    #[test]
    fn larger_client_is_called_every_n_cycles() {
        let mut q = Quantum::new((64, 4096));
        q.request(1, 64);
        q.at_cycle_boundary();
        assert_eq!(q.cycles_per_call(256), 4);
        assert_eq!(q.cycles_per_call(64), 1);
        assert_eq!(q.cycles_per_call(100), 2);
        assert_eq!(q.cycles_per_call(0), 1);
    }
}
