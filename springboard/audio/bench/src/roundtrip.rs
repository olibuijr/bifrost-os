//! `audio-roundtrip`: an impulse goes out of the output device and is captured back (the null driver's cable, the
//! loopback driver, or a physical cable on `--device`). The round trip is the distance in frames between the frame
//! at which the client wrote the impulse and the frame at which the client saw it come back, with the server's
//! one-cycle buffers modelled in both directions (see `rig`). It is compared with the HAL latency properties of both
//! scopes: `(output latency + quantum + output safety) + (input latency + quantum + input safety)`.

use crate::json::{Json, Report};
use crate::onset::{onsets, peak};
use crate::rig::Rig;
use crate::stats::{summarize, Summary};

#[derive(Debug, Clone, Copy)]
pub struct Opts {
    pub count: usize,
    pub interval_ms: f64,
    /// Onset threshold on the captured signal (the impulse is full scale; a cable may attenuate it).
    pub threshold: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub frames: Vec<u64>,
    pub latencies_ms: Vec<f64>,
    pub reported_ms: f64,
    pub dropped: u64,
    pub peak: f32,
}

/// For each sent frame, the first found onset in `[sent[i], sent[i+1])` (the last window is open-ended).
pub fn pair_onsets(sent: &[u64], found: &[usize]) -> Vec<Option<u64>> {
    sent.iter()
        .enumerate()
        .map(|(i, &s)| {
            let end = sent.get(i + 1).copied().unwrap_or(u64::MAX);
            found.iter().map(|&f| f as u64).find(|&f| f >= s && f < end)
        })
        .collect()
}

pub fn run(rig: &mut Rig, o: &Opts) -> Result<Outcome, String> {
    let q = rig.quantum;
    let qn = q as u64;
    let interval_cycles = (((o.interval_ms * rig.rate as f64 / 1000.0) / q as f64).ceil() as u64).max(4);
    let send_cycles: Vec<u64> = (0..o.count as u64).map(|i| 2 + i * interval_cycles).collect();
    let sent: Vec<u64> = send_cycles.iter().map(|c| c * qn).collect();
    let last = send_cycles.last().copied().unwrap_or(0);
    // Wait up to 200 ms for the last impulse to come back.
    let tail = 6 + (0.2 * rig.rate as f64 / q as f64).ceil() as u64;

    let channels = rig.out_channels();
    let mut planes: Vec<Vec<f32>> = vec![vec![0.0; q as usize]; channels];
    let mut captured: Vec<f32> = Vec::new();
    let mut next = 0usize;
    for k in 0..=(last + tail) {
        rig.wait_cycle(k);
        for p in planes.iter_mut() {
            p.iter_mut().for_each(|s| *s = 0.0);
        }
        if next < send_cycles.len() && send_cycles[next] == k {
            planes.iter_mut().for_each(|p| p[0] = 1.0);
            next += 1;
        }
        rig.submit(&planes);
        let cap = rig.capture();
        match cap.buffers.first() {
            Some(b) => captured.extend_from_slice(&b.data),
            None => captured.extend(std::iter::repeat(0.0).take(q as usize)),
        }
    }

    let found = onsets(&captured, o.threshold, q as usize);
    let paired = pair_onsets(&sent, &found);
    let frames: Vec<u64> = paired.iter().zip(&sent).filter_map(|(f, s)| f.map(|f| f - s)).collect();
    let latencies_ms = frames.iter().map(|f| rig.frames_to_ms(*f as f64)).collect();
    let reported = rig.out_total_frames() as f64 + rig.in_total_frames() as f64;
    Ok(Outcome {
        dropped: (o.count - frames.len()) as u64,
        frames,
        latencies_ms,
        reported_ms: rig.frames_to_ms(reported),
        peak: peak(&captured),
    })
}

pub fn report(rig: &Rig, o: &Opts, out: &Outcome) -> Report {
    let stats = summarize(&out.latencies_ms).unwrap_or_else(Summary::empty);
    let frames_f: Vec<f64> = out.frames.iter().map(|f| *f as f64).collect();
    let p50_frames = summarize(&frames_f).map_or(0.0, |s| s.p50);
    let extra = vec![
        ("impulses".to_string(), Json::Uint(o.count as u64)),
        ("threshold".to_string(), Json::Num(o.threshold as f64)),
        ("capture_peak".to_string(), Json::Num(out.peak as f64)),
        ("measured_frames_p50".to_string(), Json::Num(p50_frames)),
        ("out_total_frames".to_string(), Json::Uint(rig.out_total_frames() as u64)),
        ("in_total_frames".to_string(), Json::Uint(rig.in_total_frames() as u64)),
        ("reported_frames".to_string(), Json::Uint(rig.out_total_frames() as u64 + rig.in_total_frames() as u64)),
    ];
    rig.report("audio-roundtrip", stats, Some(out.reported_ms), out.dropped, extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::DriverSpec;
    use crate::rig::RigOptions;

    fn opts() -> Opts {
        Opts { count: 10, interval_ms: 20.0, threshold: 0.5 }
    }

    #[test]
    fn pairing_takes_the_first_onset_of_each_window() {
        let sent = [10u64, 100, 200];
        assert_eq!(pair_onsets(&sent, &[12, 13, 250]), vec![Some(12), None, Some(250)]);
        assert_eq!(pair_onsets(&sent, &[]), vec![None, None, None]);
        // an onset before the first impulse belongs to nobody
        assert_eq!(pair_onsets(&sent, &[5, 105]), vec![None, Some(105), None]);
    }

    #[test]
    fn null_cable_comes_back_after_two_quanta() {
        for q in [16u32, 64, 128] {
            let mut rig = Rig::null(50_000, q, true);
            let out = run(&mut rig, &opts()).unwrap();
            // Written to the device one cycle late, read one cycle later and delivered one cycle after that.
            assert_eq!(out.frames, vec![2 * q as u64; 10], "q {q}");
            assert_eq!(out.dropped, 0);
            assert_eq!(rig.xruns, 0);
            assert_eq!(out.peak, 1.0);
            let r = report(&rig, &opts(), &out);
            assert_eq!(r.stats.n, 10);
            assert!(r.stats.jitter().abs() < 1e-12);
            assert!((r.stats.p50 - rig.frames_to_ms(2.0 * q as f64)).abs() < 1e-12);
        }
    }

    #[test]
    fn loopback_matches_the_hal_latency_properties() {
        let mut rig = Rig::open(&RigOptions { driver: DriverSpec::Loopback, rate: 50_000, quantum: 64, device: None, input: true, manual: true }).unwrap();
        let out = run(&mut rig, &opts()).unwrap();
        // Output: driver 64 + quantum 64; input: quantum 64. Measured: the loopback's 64 frames of pre-roll plus the
        // server's one cycle in each direction.
        assert_eq!(rig.out_total_frames() + rig.in_total_frames(), 192);
        assert_eq!(out.frames, vec![192; 10]);
        assert_eq!(rig.xruns, 0);
        assert!((out.latencies_ms[0] - out.reported_ms).abs() < 1e-12);
        assert!((out.reported_ms - 3.84).abs() < 1e-12);
        let r = report(&rig, &opts(), &out);
        assert_eq!(r.driver, "loopback");
        assert_eq!(r.reported_ms, Some(out.reported_ms));
    }

    #[test]
    fn nothing_captured_without_input() {
        let mut rig = Rig::null(50_000, 64, false);
        let out = run(&mut rig, &opts()).unwrap();
        assert!(out.frames.is_empty());
        assert_eq!(out.dropped, 10);
        assert_eq!(report(&rig, &opts(), &out).stats.n, 0);
    }
}
