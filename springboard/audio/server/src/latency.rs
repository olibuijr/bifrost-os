//! `LATENCY` and `SAFETY_OFFSET` per scope: the driver's figures plus the server's own buffer, the device table
//! that records measured values, and the round-trip measurement on a loopback. Owner: task 10.

use bifrost_audio_driver::Latency;
use bifrost_audio_types::Scope;
use std::collections::HashMap;

/// A device's reported latency in frames for one scope (what `LATENCY` returns): the driver's figure plus the
/// graph quantum (the server's own buffer).
pub fn latency_frames(driver: &Latency, scope: Scope, quantum: u32) -> u32 {
    match scope {
        Scope::Output => driver.output + quantum,
        Scope::Input => driver.input + quantum,
        _ => 0,
    }
}

/// `SAFETY_OFFSET` per scope.
pub fn safety_offset(driver: &Latency, scope: Scope) -> u32 {
    match scope {
        Scope::Output => driver.output_safety,
        Scope::Input => driver.input_safety,
        _ => 0,
    }
}

/// Total output latency in nanoseconds: latency + safety offset at `rate`.
pub fn output_total_ns(driver: &Latency, quantum: u32, rate: f64) -> u64 {
    let frames = latency_frames(driver, Scope::Output, quantum) + safety_offset(driver, Scope::Output);
    (frames as f64 * 1e9 / rate).round() as u64
}

/// The pro-mode budget: <= 4 ms of output latency.
pub const PRO_BUDGET_NS: u64 = 4_000_000;

/// Round-trip frames from hardware positions: the impulse written at `sent_frame` came back at `received_frame`.
pub fn round_trip_frames(sent_frame: u64, received_frame: u64) -> Option<u32> {
    received_frame.checked_sub(sent_frame).map(|d| d as u32)
}

/// Finds the impulse (first sample with |x| >= threshold) in captured samples; its index is the round trip.
pub fn find_impulse(captured: &[f32], threshold: f32) -> Option<usize> {
    captured.iter().position(|s| s.abs() >= threshold)
}

/// The measured values recorded per device uid (the device table).
#[derive(Default, Debug)]
pub struct DeviceTable {
    measured: HashMap<String, Latency>,
}

impl DeviceTable {
    pub fn new() -> DeviceTable {
        DeviceTable::default()
    }
    pub fn record(&mut self, uid: &str, l: Latency) {
        self.measured.insert(uid.to_string(), l);
    }
    /// The recorded figures, else the driver's.
    pub fn get(&self, uid: &str, driver: &Latency) -> Latency {
        self.measured.get(uid).copied().unwrap_or(*driver)
    }
}

/// One line for `docs/perf-history.jsonl`.
pub fn perf_line(date: &str, device: &str, quantum: u32, rate: f64, round_trip_frames: u32) -> String {
    let ms = round_trip_frames as f64 * 1000.0 / rate;
    format!(
        "{{\"date\":\"{date}\",\"bench\":\"audio_round_trip\",\"device\":\"{device}\",\"quantum\":{quantum},\"rate\":{rate},\"frames\":{round_trip_frames},\"ms\":{ms:.3}}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn l() -> Latency {
        Latency { input: 40, output: 32, input_safety: 24, output_safety: 16 }
    }

    #[test]
    fn per_scope_figures() {
        assert_eq!(latency_frames(&l(), Scope::Output, 64), 96);
        assert_eq!(latency_frames(&l(), Scope::Input, 64), 104);
        assert_eq!(safety_offset(&l(), Scope::Output), 16);
        assert_eq!(safety_offset(&l(), Scope::Input), 24);
        assert_eq!(latency_frames(&l(), Scope::Global, 64), 0);
    }

    #[test]
    fn pro_budget_at_64_frames() {
        // (32 + 64 + 16) = 112 frames at 48 kHz = 2.3333 ms.
        let ns = output_total_ns(&l(), 64, 48000.0);
        assert_eq!(ns, 2_333_333);
        assert!(ns <= PRO_BUDGET_NS);
    }

    #[test]
    fn impulse_round_trip() {
        let mut buf = vec![0.0f32; 200];
        buf[130] = 0.9;
        assert_eq!(find_impulse(&buf, 0.5), Some(130));
        assert_eq!(round_trip_frames(1000, 1130), Some(130));
        assert_eq!(round_trip_frames(1000, 900), None);
    }

    #[test]
    fn table_prefers_measured() {
        let mut t = DeviceTable::new();
        assert_eq!(t.get("a", &l()), l());
        let m = Latency { input: 1, output: 2, input_safety: 3, output_safety: 4 };
        t.record("a", m);
        assert_eq!(t.get("a", &l()), m);
    }

    #[test]
    fn perf_line_format() {
        // 192 frames at 48 kHz = 4.000 ms
        assert_eq!(
            perf_line("2026-10-12", "loop", 64, 48000.0, 192),
            "{\"date\":\"2026-10-12\",\"bench\":\"audio_round_trip\",\"device\":\"loop\",\"quantum\":64,\"rate\":48000,\"frames\":192,\"ms\":4.000}"
        );
    }
}
