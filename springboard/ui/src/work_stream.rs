//! An ordered stream of GPU or CPU work with dependencies derived from the resources each unit reads and writes.
//! Work encoded to one stream is serialized only where it must be: a read waits for the last write to the same
//! resource, and a write waits for the last write and for every read since then. Units with no path between them
//! land in the same wave and may run concurrently. This is a pure model; nothing here runs work or touches a device.

use std::collections::HashMap;

/// An opaque resource id (a buffer, a texture, a model state).
pub type Resource = u32;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit {
    deps: Vec<usize>,
    wave: usize,
    done: bool,
}

#[derive(Debug, Default)]
pub struct WorkStream {
    units: Vec<Unit>,
    last_writer: HashMap<Resource, usize>,
    readers: HashMap<Resource, Vec<usize>>,
}

impl WorkStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encodes a unit that reads `reads` and writes `writes`. Returns its index in the stream.
    pub fn encode(&mut self, reads: &[Resource], writes: &[Resource]) -> usize {
        let index = self.units.len();
        let mut deps: Vec<usize> = Vec::new();
        for r in reads {
            if let Some(&w) = self.last_writer.get(r) {
                deps.push(w);
            }
        }
        for w in writes {
            if let Some(&last) = self.last_writer.get(w) {
                deps.push(last);
            }
            if let Some(rs) = self.readers.get(w) {
                deps.extend(rs.iter().copied());
            }
        }
        deps.sort_unstable();
        deps.dedup();
        let wave = deps.iter().map(|&d| self.units[d].wave + 1).max().unwrap_or(0);
        for r in reads {
            if !writes.contains(r) {
                self.readers.entry(*r).or_default().push(index);
            }
        }
        for w in writes {
            self.last_writer.insert(*w, index);
            self.readers.remove(w);
        }
        self.units.push(Unit { deps, wave, done: false });
        index
    }

    /// The units that unit `i` waits for, in ascending order.
    pub fn deps(&self, i: usize) -> &[usize] {
        &self.units[i].deps
    }

    /// The wave of unit `i`: 0 when it has no dependencies, else one more than its latest dependency.
    pub fn wave(&self, i: usize) -> usize {
        self.units[i].wave
    }

    /// The number of waves: the length of the longest dependency chain, or 0 for an empty stream.
    pub fn wave_count(&self) -> usize {
        self.units.iter().map(|u| u.wave + 1).max().unwrap_or(0)
    }

    /// Units whose dependencies are all complete and that are not complete themselves, in encode order.
    pub fn ready(&self) -> Vec<usize> {
        (0..self.units.len())
            .filter(|&i| !self.units[i].done && self.units[i].deps.iter().all(|&d| self.units[d].done))
            .collect()
    }

    /// Marks unit `i` complete. Returns false, and changes nothing, when it is not ready.
    pub fn complete(&mut self, i: usize) -> bool {
        if self.units[i].done || !self.units[i].deps.iter().all(|&d| self.units[d].done) {
            return false;
        }
        self.units[i].done = true;
        true
    }

    /// Waits for all encoded work: every unit is marked complete, in encode order. Returns how many were newly done.
    pub fn current_work_completed(&mut self) -> usize {
        let mut newly = 0;
        for i in 0..self.units.len() {
            if !self.units[i].done {
                self.units[i].done = true;
                newly += 1;
            }
        }
        newly
    }

    /// Units not yet complete.
    pub fn pending(&self) -> usize {
        self.units.iter().filter(|u| !u.done).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_of_one_resource_do_not_serialize() {
        let mut s = WorkStream::new();
        let a = s.encode(&[1], &[]);
        let b = s.encode(&[1], &[]);
        assert!(s.deps(a).is_empty() && s.deps(b).is_empty());
        assert_eq!(s.wave(a), 0);
        assert_eq!(s.wave(b), 0);
        assert_eq!(s.ready(), vec![0, 1]);
    }

    #[test]
    fn a_read_waits_for_the_last_write() {
        let mut s = WorkStream::new();
        s.encode(&[], &[1]);
        let r = s.encode(&[1], &[]);
        assert_eq!(s.deps(r), &[0]);
        assert_eq!(s.wave(r), 1);
    }

    #[test]
    fn a_write_waits_for_the_reads_since_the_last_write() {
        let mut s = WorkStream::new();
        s.encode(&[1], &[]);
        s.encode(&[1], &[]);
        let w = s.encode(&[], &[1]);
        assert_eq!(s.deps(w), &[0, 1]);
        assert_eq!(s.wave(w), 1);
    }

    #[test]
    fn a_later_write_forgets_earlier_readers() {
        let mut s = WorkStream::new();
        s.encode(&[1], &[]);
        s.encode(&[], &[1]); // unit 1 waits for the reader 0
        let w2 = s.encode(&[], &[1]); // unit 2 waits only for the write 1, not the reader 0
        assert_eq!(s.deps(w2), &[1]);
    }

    #[test]
    fn waves_follow_the_longest_chain_and_independent_work_shares_a_wave() {
        let mut s = WorkStream::new();
        s.encode(&[], &[1]); // 0
        s.encode(&[], &[2]); // 1
        let c = s.encode(&[1, 2], &[]); // 2 reads both
        assert_eq!(s.deps(c), &[0, 1]);
        assert_eq!(s.wave(c), 1);
        assert_eq!(s.wave_count(), 2);
        assert_eq!(WorkStream::new().wave_count(), 0);
    }

    #[test]
    fn complete_respects_dependencies_and_current_work_completed_retires_the_rest() {
        let mut s = WorkStream::new();
        s.encode(&[], &[1]); // 0
        s.encode(&[], &[1]); // 1 depends on 0
        assert_eq!(s.ready(), vec![0]);
        assert!(!s.complete(1));
        assert!(s.complete(0));
        assert_eq!(s.ready(), vec![1]);
        assert_eq!(s.pending(), 1);
        assert_eq!(s.current_work_completed(), 1);
        assert_eq!(s.pending(), 0);
        assert!(s.ready().is_empty());
    }
}
