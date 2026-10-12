//! `xrun-sweep`: runs the IO cycle at quantum 16, 32, 64, 128 and 256 for a while each (optionally with busy
//! threads competing for the CPUs) and reports the smallest quantum with zero xruns. An xrun is a device over- or
//! underrun reported by the driver, or a missed deadline: the cycle's work finished after the wake of the next cycle.

use crate::args::DriverSpec;
use crate::json::{hostname, Json, Report};
use crate::instrument::{render_cycle, BenchInstrument};
use crate::rig::{Rig, RigOptions};
use crate::stats::{summarize, Summary};
use bifrost_audio_units::{AudioUnit, RenderConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const QUANTA: [u32; 5] = [16, 32, 64, 128, 256];

#[derive(Debug, Clone)]
pub struct Opts {
    pub quanta: Vec<u32>,
    /// Run time per quantum.
    pub seconds: f64,
    /// Busy threads for the whole sweep (ignored on a manual clock, where nothing can compete).
    pub stress: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub quantum: u32,
    pub cycles: u64,
    /// Device xruns plus deadline misses.
    pub xruns: u64,
    pub deadline_misses: u64,
    /// How late the thread woke for each cycle, in ms.
    pub wake_late: Summary,
    pub reported_ms: f64,
}

/// Busy threads that stop when dropped.
struct Stress {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Stress {
    fn start(n: u32) -> Stress {
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..n)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut x = 1u64;
                    while !stop.load(Ordering::Relaxed) {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        std::hint::black_box(x);
                    }
                })
            })
            .collect();
        Stress { stop, threads }
    }
}

impl Drop for Stress {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Runs one quantum for `seconds`.
pub fn run_quantum(base: &RigOptions, quantum: u32, seconds: f64) -> Result<Run, String> {
    let mut rig = Rig::open(&RigOptions { quantum, input: false, ..base.clone() })?;
    let cycles = ((seconds * rig.rate as f64) / quantum as f64).ceil().max(1.0) as u64;
    let mut unit = BenchInstrument::new();
    let cfg = RenderConfig { sample_rate: rig.rate as f64, max_frames: quantum, input_channels: 0, output_channels: rig.out_channels() as u32 };
    unit.initialize(&cfg).map_err(|e| e.to_string())?;
    let mut planes: Vec<Vec<f32>> = vec![vec![0.0; quantum as usize]; rig.out_channels()];
    let mut late_ms = Vec::with_capacity(cycles as usize);
    let mut misses = 0u64;
    for k in 0..cycles {
        late_ms.push(rig.wait_cycle(k).max(0) as f64 / 1e6);
        render_cycle(&mut unit, quantum as usize, (k * quantum as u64) as f64, &[], &mut planes);
        rig.submit(&planes);
        if rig.time.now_ns() > rig.wake_ns(k + 1) {
            misses += 1;
        }
    }
    Ok(Run {
        quantum,
        cycles,
        xruns: rig.xruns + misses,
        deadline_misses: misses,
        wake_late: summarize(&late_ms).unwrap_or_else(Summary::empty),
        reported_ms: rig.frames_to_ms(rig.out_total_frames() as f64),
    })
}

/// The smallest quantum among the runs with zero xruns.
pub fn smallest_clean(runs: &[Run]) -> Option<u32> {
    runs.iter().filter(|r| r.xruns == 0).map(|r| r.quantum).min()
}

pub fn run(base: &RigOptions, o: &Opts) -> Result<Vec<Run>, String> {
    // A manual clock cannot be starved, so the stress threads would only slow the host down.
    let manual = match base.driver {
        DriverSpec::Null => true,
        DriverSpec::Loopback => base.manual,
        DriverSpec::Alsa(_) => false,
    };
    let _stress = (o.stress > 0 && !manual).then(|| Stress::start(o.stress));
    o.quanta.iter().map(|q| run_quantum(base, *q, o.seconds)).collect()
}

pub fn report(base: &RigOptions, o: &Opts, runs: &[Run]) -> Report {
    let best = smallest_clean(runs);
    // The reported run: the smallest clean quantum, else the largest one tried.
    let shown = best
        .and_then(|q| runs.iter().find(|r| r.quantum == q))
        .or_else(|| runs.iter().max_by_key(|r| r.quantum));
    let sweep: Vec<Json> = runs
        .iter()
        .map(|r| {
            Json::obj(vec![
                ("quantum", Json::Uint(r.quantum as u64)),
                ("cycles", Json::Uint(r.cycles)),
                ("xruns", Json::Uint(r.xruns)),
                ("deadline_misses", Json::Uint(r.deadline_misses)),
                ("wake_late_p99_ms", Json::Num(r.wake_late.p99)),
                ("wake_late_max_ms", Json::Num(r.wake_late.max)),
            ])
        })
        .collect();
    let extra = vec![
        ("smallest_clean_quantum".to_string(), best.map_or(Json::Null, |q| Json::Uint(q as u64))),
        ("seconds_per_quantum".to_string(), Json::Num(o.seconds)),
        ("stress_threads".to_string(), Json::Uint(o.stress as u64)),
        ("sweep".to_string(), Json::Arr(sweep)),
    ];
    let (quantum, xruns, stats, reported) = match shown {
        Some(r) => (r.quantum, r.xruns, Summary { n: r.cycles as usize, ..r.wake_late }, Some(r.reported_ms)),
        None => (base.quantum, 0, Summary::empty(), None),
    };
    Report {
        bench: "xrun-sweep".to_string(),
        host: hostname(),
        driver: base.driver.label(),
        rate: base.rate,
        quantum,
        stats,
        reported_ms: reported,
        xruns,
        dropped: 0,
        extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RigOptions {
        RigOptions { driver: DriverSpec::Null, rate: 50_000, quantum: 64, device: None, input: false, manual: true }
    }

    fn fake(quantum: u32, xruns: u64) -> Run {
        Run { quantum, cycles: 10, xruns, deadline_misses: 0, wake_late: Summary::empty(), reported_ms: 1.0 }
    }

    #[test]
    fn smallest_clean_quantum() {
        let runs = [fake(16, 3), fake(32, 1), fake(64, 0), fake(128, 0), fake(256, 0)];
        assert_eq!(smallest_clean(&runs), Some(64));
        assert_eq!(smallest_clean(&[fake(16, 1), fake(32, 2)]), None);
        assert_eq!(smallest_clean(&[]), None);
        // not required to be monotonic: a clean 32 beats a dirty 64
        assert_eq!(smallest_clean(&[fake(64, 5), fake(32, 0)]), Some(32));
    }

    #[test]
    fn manual_clock_sweep_is_clean_at_every_quantum() {
        let o = Opts { quanta: QUANTA.to_vec(), seconds: 0.1, stress: 2 };
        let runs = run(&base(), &o).unwrap();
        assert_eq!(runs.len(), 5);
        // 0.1 s at 50 kHz = 5000 frames: ceil(5000 / q).
        let cycles: Vec<u64> = runs.iter().map(|r| r.cycles).collect();
        assert_eq!(cycles, vec![313, 157, 79, 40, 20]);
        assert!(runs.iter().all(|r| r.xruns == 0 && r.deadline_misses == 0));
        assert_eq!(smallest_clean(&runs), Some(16));
        // reported latency = 2 * quantum frames on the null driver: 32 frames = 0.64 ms at quantum 16.
        assert!((runs[0].reported_ms - 0.64).abs() < 1e-12);
    }

    #[test]
    fn report_picks_the_smallest_clean_quantum() {
        let runs = vec![fake(16, 3), fake(32, 0), fake(64, 0)];
        let o = Opts { quanta: vec![16, 32, 64], seconds: 1.0, stress: 4 };
        let r = report(&base(), &o, &runs);
        assert_eq!((r.bench.as_str(), r.driver.as_str(), r.quantum, r.xruns, r.stats.n), ("xrun-sweep", "null", 32, 0, 10));
        let j = r.to_json().to_string();
        assert!(j.contains("\"smallest_clean_quantum\":32"));
        assert!(j.contains("\"stress_threads\":4"));
        assert!(j.contains("\"sweep\":[{\"quantum\":16,\"cycles\":10,\"xruns\":3,"));
    }

    #[test]
    fn report_without_a_clean_quantum_shows_the_largest() {
        let runs = vec![fake(16, 3), fake(64, 2)];
        let o = Opts { quanta: vec![16, 64], seconds: 1.0, stress: 0 };
        let r = report(&base(), &o, &runs);
        assert_eq!((r.quantum, r.xruns), (64, 2));
        assert!(r.to_json().to_string().contains("\"smallest_clean_quantum\":null"));
    }
}
