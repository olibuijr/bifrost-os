//! `midi-throughput`: how many timestamped events per second the MIDI hub, the scheduler and the instrument get
//! through, whether any are dropped, and how far the scheduler's frame offset is from the event's exact (fractional)
//! sample position.
//!
//! The events are spread over the timeline, eight per cycle, at pseudo-random fractional sample positions (a fixed
//! generator, so runs are repeatable). The timeline is simulated (no device, no sleeping): the wall time measured is
//! the work of sending, queueing, scheduling and rendering.

use crate::instrument::{render_cycle, BenchInstrument, BenchMidi};
use crate::json::{hostname, Json, Report};
use crate::schedule::{EventScheduler, Timeline};
use crate::stats::{summarize, Summary};
use bifrost_audio_units::{AudioUnit, RenderConfig};
use std::time::Instant;

pub const EVENTS_PER_CYCLE: u64 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub sent: u64,
    pub rejected: u64,
    pub impulses: u64,
    pub dropped: u64,
    pub late: u64,
    /// |placed - exact| in samples, one per delivered event.
    pub error_samples: Vec<f64>,
    pub wall_secs: f64,
}

impl Outcome {
    pub fn events_per_sec(&self) -> f64 {
        self.sent as f64 / self.wall_secs.max(1e-9)
    }
}

/// The deterministic event generator: `(sample, fraction)` of event `j`.
pub fn event_position(j: u64, quantum: u32, state: &mut u64) -> f64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let offset = (*state >> 33) % quantum as u64;
    // Below 0.98 so the scheduler's rounding slack never pushes an event into the next cycle.
    let frac = ((*state >> 11) % 1000) as f64 / 1024.0;
    (j / EVENTS_PER_CYCLE * quantum as u64 + offset) as f64 + frac
}

pub fn run(rate: u32, quantum: u32, count: usize) -> Result<Outcome, String> {
    let q = quantum as u64;
    let tl = Timeline::new(0.0, 1_000_000_000, rate as f64, 1.0);
    let mut midi = BenchMidi::new();
    let mut sched = EventScheduler::new();
    sched.record_errors = true;
    let mut unit = BenchInstrument::new();
    let cfg = RenderConfig { sample_rate: rate as f64, max_frames: quantum, input_channels: 0, output_channels: 2 };
    unit.initialize(&cfg).map_err(|e| e.to_string())?;
    let mut planes: Vec<Vec<f32>> = vec![vec![0.0; quantum as usize]; 2];
    let mut evs = Vec::new();
    let cycles = (count as u64).div_ceil(EVENTS_PER_CYCLE);

    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let positions: Vec<f64> = (0..count as u64).map(|j| event_position(j, quantum, &mut state)).collect();

    let t0 = Instant::now();
    let mut rejected = 0u64;
    for pos in &positions {
        if !midi.send_note_on(tl.host_of(*pos), 60) {
            rejected += 1;
        }
    }
    for k in 0..cycles {
        midi.drain_into(tl.host_of(((k + 1) * q) as f64), &mut sched);
        sched.take_cycle(&tl, k * q, quantum, &mut evs);
        render_cycle(&mut unit, quantum as usize, (k * q) as f64, &evs, &mut planes);
    }
    let wall_secs = t0.elapsed().as_secs_f64();

    Ok(Outcome {
        sent: count as u64 - rejected,
        rejected,
        impulses: unit.impulses(),
        dropped: (count as u64).saturating_sub(unit.impulses()),
        late: sched.late,
        error_samples: sched.errors.iter().map(|e| e.abs()).collect(),
        wall_secs,
    })
}

pub fn report(driver: &str, rate: u32, quantum: u32, out: &Outcome) -> Report {
    // The statistics are the scheduling error converted to milliseconds.
    let ms: Vec<f64> = out.error_samples.iter().map(|e| e * 1000.0 / rate as f64).collect();
    let stats = summarize(&ms).unwrap_or_else(Summary::empty);
    let max_samples = out.error_samples.iter().fold(0.0f64, |m, e| m.max(*e));
    let extra = vec![
        ("events_per_sec".to_string(), Json::Num(out.events_per_sec())),
        ("events_sent".to_string(), Json::Uint(out.sent)),
        ("events_rejected".to_string(), Json::Uint(out.rejected)),
        ("impulses".to_string(), Json::Uint(out.impulses)),
        ("late".to_string(), Json::Uint(out.late)),
        ("wall_ms".to_string(), Json::Num(out.wall_secs * 1000.0)),
        ("sched_error_max_samples".to_string(), Json::Num(max_samples)),
        ("events_per_cycle".to_string(), Json::Uint(EVENTS_PER_CYCLE)),
    ];
    Report {
        bench: "midi-throughput".to_string(),
        host: hostname(),
        driver: driver.to_string(),
        rate,
        quantum,
        stats,
        reported_ms: None,
        xruns: 0,
        dropped: out.dropped,
        extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_deterministic_and_in_their_cycle() {
        let (mut a, mut b) = (1u64, 1u64);
        for j in 0..100u64 {
            let p = event_position(j, 64, &mut a);
            assert_eq!(p, event_position(j, 64, &mut b));
            let cycle = j / EVENTS_PER_CYCLE;
            assert!(p >= (cycle * 64) as f64 && p < ((cycle + 1) * 64) as f64, "event {j} at {p}");
        }
    }

    #[test]
    fn nothing_is_dropped_and_the_error_is_under_one_sample() {
        let out = run(50_000, 64, 1000).unwrap();
        assert_eq!((out.sent, out.rejected, out.impulses, out.dropped, out.late), (1000, 0, 1000, 0, 0));
        assert_eq!(out.error_samples.len(), 1000);
        // Placed on floor(position): the error is in [0, 1) samples, plus the 1e-3 slack of the scheduler.
        assert!(out.error_samples.iter().all(|e| *e < 1.001), "max {:?}", out.error_samples.iter().fold(0.0f64, |m, e| m.max(*e)));
        assert!(out.events_per_sec() > 0.0);
    }

    #[test]
    fn errors_repeat_between_runs() {
        let a = run(50_000, 128, 333).unwrap();
        let b = run(50_000, 128, 333).unwrap();
        assert_eq!(a.error_samples, b.error_samples);
        assert_eq!(a.impulses, 333);
    }

    #[test]
    fn report_shape() {
        let out = Outcome { sent: 10, rejected: 0, impulses: 9, dropped: 1, late: 0, error_samples: vec![0.5, 0.25], wall_secs: 0.5 };
        let r = report("null", 50_000, 64, &out);
        assert_eq!((r.bench.as_str(), r.dropped, r.stats.n, r.reported_ms), ("midi-throughput", 1, 2, None));
        // 0.5 sample at 50 kHz = 0.01 ms, 0.25 sample = 0.005 ms.
        assert!((r.stats.max - 0.01).abs() < 1e-12 && (r.stats.min - 0.005).abs() < 1e-12);
        let j = r.to_json().to_string();
        assert!(j.contains("\"events_per_sec\":20,"), "{j}");
        assert!(j.contains("\"sched_error_max_samples\":0.5"));
    }
}
