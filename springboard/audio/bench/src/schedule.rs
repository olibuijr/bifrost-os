//! Sample-accurate scheduling: where a host-time-stamped event falls on a device's timeline, and the queue that
//! hands a unit its events cycle by cycle. Pure: nothing here reads a clock.

use bifrost_audio_units::MidiEvent;
use std::collections::VecDeque;

/// Slack (in samples) for the rounding of host times to whole nanoseconds. 1e-3 sample is 20 ns at 48 kHz.
const EPS: f64 = 1e-3;

/// Where an event falls relative to one cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// It belongs to a later cycle: keep it queued.
    Later,
    /// It is already past: play it at frame 0 of this cycle.
    Late,
    /// Frame offset inside the cycle.
    At(u32),
}

/// The zero timestamp of a device and its rate: maps host time to sample time and back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timeline {
    pub zero_sample: f64,
    pub zero_host_ns: u64,
    /// Nominal rate in hertz.
    pub rate: f64,
    /// Actual over nominal rate (0 or negative means 1.0).
    pub rate_scalar: f64,
}

impl Timeline {
    pub fn new(zero_sample: f64, zero_host_ns: u64, rate: f64, rate_scalar: f64) -> Timeline {
        Timeline { zero_sample, zero_host_ns, rate, rate_scalar: if rate_scalar > 0.0 { rate_scalar } else { 1.0 } }
    }

    fn samples_per_sec(&self) -> f64 {
        self.rate * self.rate_scalar
    }

    /// The (fractional) sample time at host time `host_ns`; negative before the zero timestamp.
    pub fn sample_of(&self, host_ns: u64) -> f64 {
        let dt = host_ns as i128 - self.zero_host_ns as i128;
        self.zero_sample + (dt as f64 * self.samples_per_sec()) / 1e9
    }

    /// The host time at which sample `sample` plays (rounded to a nanosecond, 0 at the earliest).
    pub fn host_of(&self, sample: f64) -> u64 {
        let dt = ((sample - self.zero_sample) * 1e9 / self.samples_per_sec()).round();
        let ns = self.zero_host_ns as f64 + dt;
        if ns <= 0.0 {
            0
        } else {
            ns as u64
        }
    }

    /// The cycle (counted from the zero timestamp, `quantum` frames each) that contains `host_ns`;
    /// `None` before the zero timestamp.
    pub fn cycle_of(&self, host_ns: u64, quantum: u32) -> Option<u64> {
        let s = self.sample_of(host_ns) - self.zero_sample;
        if s < 0.0 {
            None
        } else {
            Some((s / quantum as f64).floor() as u64)
        }
    }

    /// Placement of an event stamped `host_ns` in the cycle that starts at sample `cycle_start` and is `quantum`
    /// frames long.
    pub fn place(&self, host_ns: u64, cycle_start: u64, quantum: u32) -> Placement {
        let rel = self.sample_of(host_ns) - cycle_start as f64 + EPS;
        if rel < 0.0 {
            Placement::Late
        } else if rel >= quantum as f64 {
            Placement::Later
        } else {
            Placement::At(rel.floor() as u32)
        }
    }

    /// Where the event ends up minus where it was stamped, in samples: 0 for an event on a whole sample,
    /// in (-1, 0] for a fractional one, positive for a late event.
    pub fn error_samples(&self, host_ns: u64, cycle_start: u64, placed: Placement) -> f64 {
        let at = match placed {
            Placement::At(o) => cycle_start as f64 + o as f64,
            Placement::Late => cycle_start as f64,
            Placement::Later => return 0.0,
        };
        at - self.sample_of(host_ns)
    }
}

/// Timestamped events waiting for their cycle, in stamp order (stable for equal stamps).
#[derive(Debug, Default)]
pub struct EventScheduler {
    pending: VecDeque<(u64, MidiEvent)>,
    /// Events that were already past when their cycle came (played at frame 0).
    pub late: u64,
    /// Events handed to a cycle.
    pub delivered: u64,
    /// Placement error of every delivered event in samples, kept when `record_errors` is true.
    pub errors: Vec<f64>,
    pub record_errors: bool,
}

impl EventScheduler {
    pub fn new() -> EventScheduler {
        EventScheduler::default()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Queues an event stamped `host_ns`; events with equal stamps keep their arrival order.
    pub fn push(&mut self, host_ns: u64, ev: MidiEvent) {
        let at = self.pending.partition_point(|e| e.0 <= host_ns);
        self.pending.insert(at, (host_ns, ev));
    }

    /// Fills `out` (cleared first) with the events of the cycle `[cycle_start, cycle_start + quantum)`, offsets set.
    /// Later events stay queued; late ones come out at offset 0 and count in `late`.
    pub fn take_cycle(&mut self, tl: &Timeline, cycle_start: u64, quantum: u32, out: &mut Vec<MidiEvent>) {
        out.clear();
        while let Some(&(host_ns, ev)) = self.pending.front() {
            let placed = tl.place(host_ns, cycle_start, quantum);
            let offset = match placed {
                Placement::Later => break,
                Placement::Late => {
                    self.late += 1;
                    0
                }
                Placement::At(o) => o,
            };
            self.pending.pop_front();
            self.delivered += 1;
            if self.record_errors {
                self.errors.push(tl.error_samples(host_ns, cycle_start, placed));
            }
            out.push(MidiEvent { offset, ..ev });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 50 kHz: one sample is exactly 20 000 ns. Zero timestamp: sample 0 at 1 s.
    fn tl() -> Timeline {
        Timeline::new(0.0, 1_000_000_000, 50_000.0, 1.0)
    }

    fn at(sample: u64) -> u64 {
        1_000_000_000 + 20_000 * sample
    }

    #[test]
    fn sample_and_host_round_trip() {
        let t = tl();
        assert_eq!(t.sample_of(at(144)), 144.0);
        assert_eq!(t.host_of(144.0), at(144));
        assert_eq!(t.sample_of(1_000_000_000 - 20_000), -1.0);
        assert_eq!(t.host_of(0.0), 1_000_000_000);
    }

    #[test]
    fn rate_scalar_stretches_time() {
        // 2x fast: 1 ms after zero is 50000 * 2 * 0.001 = 100 samples.
        let t = Timeline::new(0.0, 1_000_000_000, 50_000.0, 2.0);
        assert_eq!(t.sample_of(1_001_000_000), 100.0);
        assert_eq!(t.host_of(100.0), 1_001_000_000);
        // an unset scalar is 1.
        assert_eq!(Timeline::new(0.0, 0, 50_000.0, 0.0).rate_scalar, 1.0);
    }

    #[test]
    fn nonzero_zero_sample() {
        let t = Timeline::new(1000.0, 5_000_000_000, 50_000.0, 1.0);
        assert_eq!(t.sample_of(5_000_000_000 + 20_000 * 48), 1048.0);
        assert_eq!(t.host_of(1048.0), 5_000_000_000 + 20_000 * 48);
    }

    #[test]
    fn cycle_of_counts_from_zero() {
        let t = tl();
        assert_eq!(t.cycle_of(at(0), 64), Some(0));
        assert_eq!(t.cycle_of(at(63), 64), Some(0));
        assert_eq!(t.cycle_of(at(64), 64), Some(1));
        assert_eq!(t.cycle_of(at(640), 64), Some(10));
        assert_eq!(t.cycle_of(1_000_000_000 - 1, 64), None);
    }

    #[test]
    fn placement_in_a_64_frame_cycle() {
        let t = tl();
        // The cycle covers samples 96..160.
        assert_eq!(t.place(at(144), 96, 64), Placement::At(48));
        assert_eq!(t.place(at(96), 96, 64), Placement::At(0));
        assert_eq!(t.place(at(159), 96, 64), Placement::At(63));
        assert_eq!(t.place(at(160), 96, 64), Placement::Later);
        assert_eq!(t.place(at(95), 96, 64), Placement::Late);
    }

    #[test]
    fn a_rounded_host_time_still_lands_on_its_sample() {
        // 48 kHz: sample 100 is at 2 083 333.33 ns; the stamp is rounded to ...333 ns.
        let t = Timeline::new(0.0, 0, 48_000.0, 1.0);
        let h = t.host_of(100.0);
        assert_eq!(h, 2_083_333);
        assert_eq!(t.place(h, 64, 64), Placement::At(36));
    }

    #[test]
    fn error_in_samples() {
        let t = tl();
        assert_eq!(t.error_samples(at(5), 0, Placement::At(5)), 0.0);
        // stamped at sample 10, played at the start of the cycle at 64: 54 samples late.
        assert_eq!(t.error_samples(at(10), 64, Placement::Late), 54.0);
        // half a sample after sample 5 (10 000 ns), placed on frame 5: -0.5.
        assert_eq!(t.error_samples(at(5) + 10_000, 0, Placement::At(5)), -0.5);
        assert_eq!(t.error_samples(at(500), 0, Placement::Later), 0.0);
    }

    #[test]
    fn scheduler_hands_events_to_their_cycle() {
        let t = tl();
        let mut s = EventScheduler::new();
        s.push(at(200), MidiEvent::note_on(0, 0, 62, 100));
        s.push(at(70), MidiEvent::note_on(0, 0, 61, 100));
        s.push(at(5), MidiEvent::note_on(0, 0, 60, 100));
        let mut out = Vec::new();
        s.take_cycle(&t, 0, 64, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].offset, out[0].data1), (5, 60));
        s.take_cycle(&t, 64, 64, &mut out);
        assert_eq!((out.len(), out[0].offset, out[0].data1), (1, 6, 61));
        s.take_cycle(&t, 128, 64, &mut out);
        assert!(out.is_empty());
        assert_eq!(s.len(), 1);
        s.take_cycle(&t, 192, 64, &mut out);
        assert_eq!((out.len(), out[0].offset, out[0].data1), (1, 8, 62));
        assert!(s.is_empty());
        assert_eq!((s.delivered, s.late), (3, 0));
    }

    #[test]
    fn late_events_play_at_zero_and_count() {
        let t = tl();
        let mut s = EventScheduler::new();
        s.record_errors = true;
        s.push(at(10), MidiEvent::note_on(0, 0, 60, 100));
        s.push(at(70), MidiEvent::note_on(0, 0, 61, 100));
        let mut out = Vec::new();
        s.take_cycle(&t, 64, 64, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].offset, out[1].offset), (0, 6));
        assert_eq!(s.late, 1);
        assert_eq!(s.errors, vec![54.0, 0.0]);
    }

    #[test]
    fn equal_stamps_keep_arrival_order() {
        let t = tl();
        let mut s = EventScheduler::new();
        s.push(at(3), MidiEvent::note_on(0, 0, 1, 100));
        s.push(at(3), MidiEvent::note_on(0, 0, 2, 100));
        s.push(at(3), MidiEvent::note_on(0, 0, 3, 100));
        let mut out = Vec::new();
        s.take_cycle(&t, 0, 64, &mut out);
        let keys: Vec<u8> = out.iter().map(|e| e.data1).collect();
        assert_eq!(keys, vec![1, 2, 3]);
    }
}
