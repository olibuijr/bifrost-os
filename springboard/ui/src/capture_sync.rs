//! Synchronized capture: pairs samples from several capture outputs (a colour frame, a depth map, a
//! batch of metadata) that were taken at the same moment, so the app receives them together. This is
//! pure. The capture pipeline pushes each output's samples with their timestamps in microseconds, and
//! takes complete groups back out.
//!
//! Output 0 is the primary. Each primary sample is an anchor, and a group forms when every other output
//! has a sample within the tolerance of that anchor. The nearest such sample is used. A primary sample
//! that no partner can match is dropped, so a missing partner never stalls the stream for good.

use std::collections::VecDeque;

/// Samples taken at the same moment, one per output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<T> {
    /// The time of the primary output's sample, in microseconds.
    pub time_us: u64,
    /// One entry per output, in output order: that sample's own time and its value.
    pub samples: Vec<(u64, T)>,
}

/// What the next primary sample can do, decided from the partner outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// Every partner has a sample in the window.
    Matched,
    /// A partner has nothing in the window yet, and a later sample may still arrive.
    Wait,
    /// A partner already moved past the window, so no sample will ever match this anchor.
    Dead,
}

/// Pairs samples across outputs by timestamp.
#[derive(Debug)]
pub struct Synchronizer<T> {
    tolerance_us: u64,
    capacity: usize,
    queues: Vec<VecDeque<(u64, T)>>,
    last_time: Vec<Option<u64>>,
}

impl<T> Synchronizer<T> {
    /// A synchronizer for `outputs` outputs that match within `tolerance_us`, holding at most `capacity`
    /// samples per output. None when there are no outputs or the capacity is zero.
    pub fn new(outputs: usize, tolerance_us: u64, capacity: usize) -> Option<Self> {
        if outputs == 0 || capacity == 0 {
            return None;
        }
        Some(Synchronizer {
            tolerance_us,
            capacity,
            queues: (0..outputs).map(|_| VecDeque::new()).collect(),
            last_time: vec![None; outputs],
        })
    }

    /// Adds a sample from `output`, taken at `time_us`. Refused for an output that does not exist, or when
    /// the time goes back for that output. When the output's queue is full, its oldest sample is dropped.
    pub fn push(&mut self, output: usize, time_us: u64, value: T) -> bool {
        if output >= self.queues.len() {
            return false;
        }
        if let Some(last) = self.last_time[output] {
            if time_us < last {
                return false;
            }
        }
        self.last_time[output] = Some(time_us);
        let capacity = self.capacity;
        let queue = &mut self.queues[output];
        if queue.len() >= capacity {
            queue.pop_front();
        }
        queue.push_back((time_us, value));
        true
    }

    /// The samples waiting for `output`.
    pub fn pending(&self, output: usize) -> usize {
        self.queues.get(output).map_or(0, |q| q.len())
    }

    /// Takes the next complete group. Returns None when the primary output is empty, or when a partner has
    /// not yet produced a sample that could match the next primary sample.
    pub fn pop_group(&mut self) -> Option<Group<T>> {
        loop {
            let anchor = self.queues[0].front()?.0;
            let tolerance = self.tolerance_us;
            let window_start = anchor.saturating_sub(tolerance);

            // Partner samples older than the window cannot match this anchor or any later one: drop them.
            for queue in self.queues.iter_mut().skip(1) {
                while matches!(queue.front(), Some(s) if s.0 < window_start) {
                    queue.pop_front();
                }
            }

            let mut picks: Vec<usize> = Vec::with_capacity(self.queues.len().saturating_sub(1));
            let mut verdict = Verdict::Matched;
            for queue in self.queues.iter().skip(1) {
                if queue.is_empty() {
                    verdict = Verdict::Wait;
                    break;
                }
                // `k` is the first sample at or after the anchor: the sample just before it and the one at `k`
                // are the only candidates for the closest match.
                let k = queue.partition_point(|s| s.0 < anchor);
                let before = if k > 0 { Some((k - 1, anchor - queue[k - 1].0)) } else { None };
                let after = if k < queue.len() { Some((k, queue[k].0 - anchor)) } else { None };
                let nearest = match (before, after) {
                    (Some(b), Some(a)) => {
                        if a.1 < b.1 {
                            Some(a)
                        } else {
                            Some(b)
                        }
                    }
                    (Some(b), None) => Some(b),
                    (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                match nearest {
                    Some((index, distance)) if distance <= tolerance => picks.push(index),
                    _ => {
                        verdict = if after.is_some() { Verdict::Dead } else { Verdict::Wait };
                        break;
                    }
                }
            }

            match verdict {
                Verdict::Wait => return None,
                Verdict::Dead => {
                    self.queues[0].pop_front();
                    continue;
                }
                Verdict::Matched => {}
            }

            let (anchor_time, anchor_value) = self.queues[0].pop_front()?;
            let mut samples = Vec::with_capacity(self.queues.len());
            samples.push((anchor_time, anchor_value));
            for i in 1..self.queues.len() {
                let index = picks[i - 1];
                let queue = &mut self.queues[i];
                // Popping up to and including the match also drops the older samples in front of it.
                let mut chosen = None;
                for _ in 0..=index {
                    chosen = queue.pop_front();
                }
                samples.push(chosen?);
            }
            return Some(Group { time_us: anchor, samples });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_forms_when_every_output_has_a_sample_in_the_window() {
        let mut s = Synchronizer::new(2, 5_000, 8).unwrap();
        assert!(s.push(0, 1_000, "colour"));
        assert!(s.push(1, 1_002, "depth"));
        let group = s.pop_group().expect("both outputs have a sample in the window");
        assert_eq!(group.time_us, 1_000);
        assert_eq!(group.samples, vec![(1_000, "colour"), (1_002, "depth")]);
        assert!(s.pop_group().is_none(), "nothing is left to group");
    }

    #[test]
    fn the_primary_waits_for_a_partner_that_has_not_arrived() {
        let mut s = Synchronizer::new(2, 5_000, 8).unwrap();
        s.push(0, 10_000, "colour");
        assert!(s.pop_group().is_none(), "the depth sample has not come yet");
        assert_eq!(s.pending(0), 1, "the colour sample is kept while waiting");
        s.push(1, 10_003, "depth");
        assert_eq!(s.pop_group().unwrap().samples.len(), 2);
    }

    #[test]
    fn a_primary_sample_with_no_partner_in_the_window_is_dropped() {
        let mut s = Synchronizer::new(2, 1_000, 8).unwrap();
        s.push(0, 0, "lost");
        s.push(0, 5_000, "matched");
        s.push(1, 5_100, "depth");
        // The first colour sample has no depth sample within 1 ms, and the depth sample is already past it.
        let group = s.pop_group().expect("the second colour sample matches");
        assert_eq!(group.time_us, 5_000);
        assert_eq!(group.samples, vec![(5_000, "matched"), (5_100, "depth")]);
    }

    #[test]
    fn the_nearest_partner_wins_and_a_tie_goes_to_the_earlier_one() {
        let mut s = Synchronizer::new(2, 1_000, 8).unwrap();
        s.push(0, 2_000, "colour");
        s.push(1, 1_500, "early");
        s.push(1, 2_300, "late");
        let group = s.pop_group().unwrap();
        assert_eq!(group.samples[1], (2_300, "late"), "300 us after beats 500 us before");
        assert_eq!(s.pending(1), 0, "the earlier partner is dropped with the match");

        let mut tie = Synchronizer::new(2, 1_000, 8).unwrap();
        tie.push(0, 2_000, "colour");
        tie.push(1, 1_800, "before");
        tie.push(1, 2_200, "after");
        assert_eq!(tie.pop_group().unwrap().samples[1], (1_800, "before"));
    }

    #[test]
    fn samples_for_unknown_outputs_or_going_back_in_time_are_refused() {
        let mut s: Synchronizer<u32> = Synchronizer::new(2, 1_000, 8).unwrap();
        assert!(!s.push(5, 0, 1), "output 5 does not exist");
        assert!(s.push(0, 100, 2));
        assert!(!s.push(0, 50, 3), "time went backwards on output 0");
        assert!(s.push(0, 100, 4), "an equal time is allowed");
        assert_eq!(s.pending(0), 2);
        assert_eq!(s.pending(9), 0);
    }

    #[test]
    fn a_late_partner_sample_older_than_the_window_is_dropped_and_does_not_block() {
        let mut s = Synchronizer::new(2, 100, 8).unwrap();
        s.push(1, 10, "old");
        s.push(0, 1_000, "a");
        assert!(s.pop_group().is_none(), "the old depth sample is useless, so the group waits for a new one");
        assert_eq!(s.pending(1), 0, "the old depth sample was dropped");
        s.push(1, 1_010, "d");
        assert_eq!(s.pop_group().unwrap().samples, vec![(1_000, "a"), (1_010, "d")]);
    }

    #[test]
    fn a_full_queue_drops_its_oldest_sample() {
        let mut s: Synchronizer<u32> = Synchronizer::new(1, 0, 2).unwrap();
        s.push(0, 0, 0);
        s.push(0, 1, 1);
        s.push(0, 2, 2);
        assert_eq!(s.pending(0), 2);
        let first = s.pop_group().unwrap();
        assert_eq!(first.time_us, 1, "the sample at time 0 was dropped to make room");
        assert_eq!(first.samples, vec![(1, 1)]);
    }

    #[test]
    fn a_synchronizer_needs_an_output_and_room_to_hold_samples() {
        assert!(Synchronizer::<u8>::new(0, 10, 4).is_none());
        assert!(Synchronizer::<u8>::new(1, 10, 0).is_none());
        assert!(Synchronizer::<u8>::new(2, 10, 4).is_some());
    }
}
