//! `midi-latency`: Bench Source -> MIDI hub -> Bench Instrument -> engine cycle -> output device.
//!
//! The source sends note-ons stamped with host time. Each cycle the instrument's queue is drained into the
//! `EventScheduler`, the unit writes an impulse at the event's sample offset, the mix goes through the rig (written
//! to the device one cycle after it was rendered), and the impulse is detected in the mix that is handed to the
//! device (the "process tap"). The impulse reaches the converter `output latency + safety` frames after the frame
//! the device receives it, so
//!
//! `latency = host_of(device frame of the impulse + output latency + output safety) - note host time`.
//!
//! With the note stamped ahead of its cycle, the device frame is `note sample + quantum`, so the latency is exactly
//! `output latency + safety + quantum` frames, which is what the HAL reports.

use crate::instrument::{render_cycle, BenchInstrument, BenchMidi};
use crate::json::{Json, Report};
use crate::onset::onsets;
use crate::rig::Rig;
use crate::schedule::EventScheduler;
use crate::stats::{summarize, Summary};
use bifrost_audio_units::{AudioUnit, RenderConfig};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy)]
pub struct Opts {
    /// Notes to send.
    pub count: usize,
    /// Time between notes.
    pub interval_ms: f64,
    /// Cycles ahead of its stamp each note is sent. 0 with a real clock: a source thread stamps each note with the
    /// host time at which it sends it (an asynchronous source, so the latency includes the wait for the next cycle).
    pub lead: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub latencies_ms: Vec<f64>,
    pub reported_ms: f64,
    pub sent: u64,
    pub late: u64,
    pub dropped: u64,
    pub impulses: u64,
    pub wake_late_max_ms: f64,
    pub threaded: bool,
}

/// The sample of each note: spread over the cycle with a stride of 7 frames so offsets vary.
pub fn note_samples(count: usize, first_cycle: u64, quantum: u32, rate: u32, interval_ms: f64) -> Vec<u64> {
    let q = quantum as u64;
    let interval = ((interval_ms * rate as f64 / 1000.0).round() as u64).max(2 * q);
    (0..count as u64).map(|i| first_cycle * q + i * interval + (i * 7) % q).collect()
}

pub fn run(rig: &mut Rig, o: &Opts) -> Result<Outcome, String> {
    let q = rig.quantum;
    let qn = q as u64;
    let threaded = o.lead == 0 && !rig.time.is_manual();
    let samples = note_samples(o.count, o.lead as u64 + 2, q, rig.rate, o.interval_ms);
    let planned: Vec<u64> = samples.iter().map(|s| rig.tl.host_of(*s as f64)).collect();
    let cycle_of_note: Vec<u64> = samples.iter().map(|s| s / qn).collect();
    let last_cycle = cycle_of_note.last().copied().unwrap_or(0);

    let midi = Arc::new(Mutex::new(BenchMidi::new()));
    let mut unit = BenchInstrument::new();
    let cfg = RenderConfig { sample_rate: rig.rate as f64, max_frames: q, input_channels: 0, output_channels: rig.out_channels() as u32 };
    unit.initialize(&cfg).map_err(|e| e.to_string())?;
    let mut sched = EventScheduler::new();
    let mut planes: Vec<Vec<f32>> = vec![vec![0.0; q as usize]; rig.out_channels()];
    let mut evs = Vec::new();

    let source = if threaded {
        let (m, t, plan) = (midi.clone(), rig.time.clone(), planned.clone());
        Some(std::thread::spawn(move || {
            let mut stamps = Vec::with_capacity(plan.len());
            for ns in plan {
                t.wait_until(ns);
                let now = t.now_ns();
                m.lock().expect("the bench lock").send_note_on(now, 60);
                stamps.push(now);
            }
            stamps
        }))
    } else {
        None
    };

    let mut stamps: Vec<u64> = Vec::new();
    let mut next = 0usize;
    let mut landing: Vec<u64> = Vec::new();
    let mut wake_late_max = 0i64;
    for k in 0..=(last_cycle + 4) {
        wake_late_max = wake_late_max.max(rig.wait_cycle(k));
        if !threaded {
            let mut m = midi.lock().expect("the bench lock");
            while next < o.count && cycle_of_note[next].saturating_sub(o.lead as u64) <= k {
                m.send_note_on(planned[next], 60);
                stamps.push(planned[next]);
                next += 1;
            }
        }
        let end_ns = rig.wake_ns(k + 1);
        midi.lock().expect("the bench lock").drain_into(end_ns, &mut sched);
        sched.take_cycle(&rig.tl, k * qn, q, &mut evs);
        render_cycle(&mut unit, q as usize, (k * qn) as f64, &evs, &mut planes);
        let start = rig.submit(&planes);
        for idx in onsets(&planes[0], 0.5, 1) {
            landing.push(start + idx as u64);
        }
    }
    if let Some(h) = source {
        stamps = h.join().map_err(|_| "the source thread panicked".to_string())?;
    }

    let device_frames = (rig.out_lat.output + rig.out_lat.output_safety) as u64;
    let n = stamps.len().min(landing.len());
    let latencies_ms = (0..n)
        .map(|i| {
            let heard = rig.tl.host_of((landing[i] + device_frames) as f64);
            (heard as f64 - stamps[i] as f64) / 1e6
        })
        .collect();
    Ok(Outcome {
        latencies_ms,
        reported_ms: rig.frames_to_ms(rig.out_total_frames() as f64),
        sent: stamps.len() as u64,
        late: sched.late,
        dropped: (o.count - n) as u64,
        impulses: unit.impulses(),
        wake_late_max_ms: wake_late_max as f64 / 1e6,
        threaded,
    })
}

pub fn report(rig: &Rig, o: &Opts, out: &Outcome) -> Report {
    let stats = summarize(&out.latencies_ms).unwrap_or_else(Summary::empty);
    let extra = vec![
        ("notes_sent".to_string(), Json::Uint(out.sent)),
        ("impulses".to_string(), Json::Uint(out.impulses)),
        ("late".to_string(), Json::Uint(out.late)),
        ("lead_cycles".to_string(), Json::Uint(o.lead as u64)),
        ("interval_ms".to_string(), Json::Num(o.interval_ms)),
        ("async_source".to_string(), Json::Bool(out.threaded)),
        ("out_total_frames".to_string(), Json::Uint(rig.out_total_frames() as u64)),
        ("wake_late_max_ms".to_string(), Json::Num(out.wake_late_max_ms)),
    ];
    rig.report("midi-latency", stats, Some(out.reported_ms), out.dropped, extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(lead: u32) -> Opts {
        Opts { count: 20, interval_ms: 20.0, lead }
    }

    #[test]
    fn note_samples_spread_and_stride() {
        // first cycle 2, quantum 64, 50 kHz, 20 ms = 1000 samples apart; offsets 0, 7, 14.
        assert_eq!(note_samples(3, 2, 64, 50_000, 20.0), vec![128, 128 + 1000 + 7, 128 + 2000 + 14]);
        // an interval shorter than two cycles is stretched to two cycles.
        assert_eq!(note_samples(2, 0, 64, 50_000, 0.1), vec![0, 128 + 7]);
    }

    #[test]
    fn null_driver_latency_is_exactly_output_plus_safety_plus_quantum() {
        for q in [16u32, 32, 64, 128] {
            let mut rig = Rig::null(50_000, q, false);
            let out = run(&mut rig, &opts(3)).unwrap();
            // Null driver: output latency 0, safety = quantum; the HAL adds the quantum: 2 * quantum frames.
            let expected_frames = rig.out_lat.output + rig.out_lat.output_safety + q;
            assert_eq!(expected_frames, 2 * q);
            assert_eq!(rig.out_total_frames(), expected_frames);
            assert_eq!(out.latencies_ms.len(), 20);
            assert_eq!((out.late, out.dropped, out.impulses, out.sent), (0, 0, 20, 20));
            let exact_ms = expected_frames as f64 * 1000.0 / 50_000.0;
            assert!((out.reported_ms - exact_ms).abs() < 1e-12);
            for l in &out.latencies_ms {
                assert!((l - exact_ms).abs() < 1e-9, "q {q}: {l} vs {exact_ms}");
                // and in whole frames
                assert_eq!((l * 50_000.0 / 1000.0).round() as u32, expected_frames);
            }
            assert!(!out.threaded);
        }
    }

    #[test]
    fn stamping_on_time_without_lead_gives_the_same_latency() {
        let mut rig = Rig::null(50_000, 64, false);
        let out = run(&mut rig, &opts(0)).unwrap();
        assert_eq!((out.late, out.dropped), (0, 0));
        assert!(out.latencies_ms.iter().all(|l| (l - 2.56).abs() < 1e-9));
    }

    #[test]
    fn report_has_the_shared_shape() {
        let mut rig = Rig::null(50_000, 64, false);
        let o = opts(3);
        let out = run(&mut rig, &o).unwrap();
        let r = report(&rig, &o, &out);
        assert_eq!(r.bench, "midi-latency");
        assert_eq!(r.driver, "null");
        assert_eq!((r.rate, r.quantum, r.stats.n, r.xruns, r.dropped), (50_000, 64, 20, 0, 0));
        assert!((r.stats.p50 - 2.56).abs() < 1e-9);
        assert!(r.stats.jitter().abs() < 1e-9);
        assert!((r.reported_ms.unwrap() - 2.56).abs() < 1e-9);
        let j = r.to_json().to_string();
        assert!(j.starts_with("{\"bench\":\"midi-latency\""));
        assert!(j.contains("\"late\":0"));
    }
}
