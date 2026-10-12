//! Per-frame HDR metadata for video pictures, as plain std code.
//! Two transfer curves carry HDR: SMPTE ST 2084 (PQ) and ARIB STD-B67 (HLG). `pq_encode` and `hlg_encode` map
//! linear light to a signal in 0..=1, and the decoders invert them. A `FrameSession` takes the luminance samples of
//! each frame and returns the metadata that travels with that picture: the peak and average signal of the frame as
//! 12-bit codes, a scene-change flag, and the running content light level (the largest frame peak and the largest
//! frame average seen so far). Sessions hold no buffers and no device; the caller attaches the metadata to its
//! picture. The curve is the `Transfer` a picture's attachments carry (`buffer_attach`).

use crate::buffer_attach::Transfer;

/// The luminance the PQ curve maps to signal 1.0, in cd/m2 (nits).
pub const PQ_PEAK_NITS: f64 = 10000.0;
/// The nominal peak the HLG signal is referred to in this model, in nits.
pub const HLG_NOMINAL_PEAK_NITS: f64 = 1000.0;

const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;

const HLG_A: f64 = 0.17883277;
const HLG_B: f64 = 0.28466892;
const HLG_C: f64 = 0.55991073;

/// PQ encode: linear light in nits to a signal in 0..=1. Input is clamped to 0..=10000 nits.
pub fn pq_encode(nits: f64) -> f64 {
    let y = (nits / PQ_PEAK_NITS).clamp(0.0, 1.0);
    let p = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * p) / (1.0 + PQ_C3 * p)).powf(PQ_M2)
}

/// PQ decode: a signal in 0..=1 back to nits.
pub fn pq_decode(signal: f64) -> f64 {
    let e = signal.clamp(0.0, 1.0);
    let p = e.powf(1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * p;
    (num / den).powf(1.0 / PQ_M1) * PQ_PEAK_NITS
}

/// HLG encode (the OETF): scene linear light in 0..=1 to a signal in 0..=1.
pub fn hlg_encode(linear: f64) -> f64 {
    let e = linear.clamp(0.0, 1.0);
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt()
    } else {
        HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
    }
}

/// HLG decode (the inverse OETF): a signal in 0..=1 back to scene linear light.
pub fn hlg_decode(signal: f64) -> f64 {
    let s = signal.clamp(0.0, 1.0);
    if s <= 0.5 {
        s * s / 3.0
    } else {
        (((s - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// The signal a luminance in nits takes under a transfer: PQ against 10000 nits, HLG against the nominal 1000 nits.
/// None for transfers that carry no HDR.
pub fn signal_for_nits(transfer: Transfer, nits: f64) -> Option<f64> {
    match transfer {
        Transfer::Pq => Some(pq_encode(nits)),
        Transfer::Hlg => Some(hlg_encode(nits / HLG_NOMINAL_PEAK_NITS)),
        _ => None,
    }
}

/// Quantizes a signal to the 12-bit code the metadata carries (0..=4095).
pub fn code12(signal: f64) -> u16 {
    (signal.clamp(0.0, 1.0) * 4095.0).round() as u16
}

/// Peak and average luminance of one frame, in nits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLuminance {
    pub max_nits: f64,
    pub average_nits: f64,
}

/// Peak and average of a frame's samples in nits. Negative samples count as 0. None for an empty frame.
pub fn frame_luminance(nits: &[f64]) -> Option<FrameLuminance> {
    if nits.is_empty() {
        return None;
    }
    let mut max_nits = 0.0_f64;
    let mut sum = 0.0_f64;
    for &v in nits {
        let v = v.max(0.0);
        if v > max_nits {
            max_nits = v;
        }
        sum += v;
    }
    Some(FrameLuminance { max_nits, average_nits: sum / nits.len() as f64 })
}

/// The metadata one picture carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMetadata {
    pub transfer: Transfer,
    /// Peak and average signal of this frame, 12-bit.
    pub max_code: u16,
    pub average_code: u16,
    pub scene_change: bool,
    /// Running content light level after this frame (MaxCLL and MaxFALL), in whole nits.
    pub max_cll: u32,
    pub max_fall: u32,
}

/// Generates the metadata for a run of frames. It starts from an HDR transfer and a frame rate, and keeps the
/// running content light level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSession {
    transfer: Transfer,
    fps: f64,
    frames: u64,
    max_cll: u32,
    max_fall: u32,
}

impl FrameSession {
    /// A session for an HDR transfer (PQ or HLG) at a positive, finite frame rate. None for any other input.
    pub fn new(transfer: Transfer, fps: f64) -> Option<Self> {
        let hdr = matches!(transfer, Transfer::Pq | Transfer::Hlg);
        if !hdr || !fps.is_finite() || fps <= 0.0 {
            return None;
        }
        Some(FrameSession { transfer, fps, frames: 0, max_cll: 0, max_fall: 0 })
    }

    pub fn frame_rate(&self) -> f64 {
        self.fps
    }

    /// Frames pushed so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Largest frame peak seen so far, in whole nits.
    pub fn max_cll(&self) -> u32 {
        self.max_cll
    }

    /// Largest frame average seen so far, in whole nits.
    pub fn max_fall(&self) -> u32 {
        self.max_fall
    }

    /// Adds one frame from its per-sample nits and returns its metadata. An empty frame returns None and leaves the
    /// session unchanged.
    pub fn push_frame(&mut self, nits: &[f64], scene_change: bool) -> Option<FrameMetadata> {
        let lum = frame_luminance(nits)?;
        let max_signal = signal_for_nits(self.transfer, lum.max_nits)?;
        let avg_signal = signal_for_nits(self.transfer, lum.average_nits)?;
        let max_whole = lum.max_nits.round() as u32;
        let avg_whole = lum.average_nits.round() as u32;
        self.frames += 1;
        self.max_cll = self.max_cll.max(max_whole);
        self.max_fall = self.max_fall.max(avg_whole);
        Some(FrameMetadata {
            transfer: self.transfer,
            max_code: code12(max_signal),
            average_code: code12(avg_signal),
            scene_change,
            max_cll: self.max_cll,
            max_fall: self.max_fall,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_is_exact_at_the_peak_and_near_zero_at_black() {
        assert_eq!(pq_encode(10000.0), 1.0);
        assert!(pq_encode(0.0) < 1e-5, "black encodes to a very small signal, not exactly zero");
        assert_eq!(pq_decode(1.0), 10000.0);
        assert_eq!(pq_decode(0.0), 0.0);
    }

    #[test]
    fn pq_round_trips_across_the_range() {
        for nits in [1.0, 100.0, 1000.0, 4000.0] {
            let back = pq_decode(pq_encode(nits));
            assert!((back - nits).abs() < 1e-6 * nits, "{} nits came back as {}", nits, back);
        }
    }

    #[test]
    fn hlg_matches_its_breakpoint_and_peak() {
        assert_eq!(hlg_encode(0.0), 0.0);
        assert!((hlg_encode(1.0 / 12.0) - 0.5).abs() < 1e-3, "the square-root segment meets 0.5 at 1/12");
        assert!((hlg_encode(1.0) - 1.0).abs() < 1e-3);
        assert!((hlg_decode(0.5) - 1.0 / 12.0).abs() < 1e-9);
        assert!((hlg_decode(hlg_encode(0.3)) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn frame_luminance_clamps_negatives_and_rejects_empty_frames() {
        let lum = frame_luminance(&[0.0, 100.0, 400.0, -5.0]).unwrap();
        assert_eq!(lum.max_nits, 400.0);
        assert_eq!(lum.average_nits, 125.0);
        assert_eq!(frame_luminance(&[]), None);
    }

    #[test]
    fn sessions_accept_only_hdr_transfers_and_positive_rates() {
        assert!(FrameSession::new(Transfer::Srgb, 24.0).is_none());
        assert!(FrameSession::new(Transfer::Pq, 0.0).is_none());
        assert!(FrameSession::new(Transfer::Hlg, f64::NAN).is_none());
        let s = FrameSession::new(Transfer::Pq, 23.976).unwrap();
        assert_eq!(s.frame_rate(), 23.976);
        assert_eq!(s.frames(), 0);
    }

    #[test]
    fn running_content_light_level_keeps_the_largest_peak_and_average() {
        let mut s = FrameSession::new(Transfer::Pq, 24.0).unwrap();
        s.push_frame(&[100.0, 200.0], false).unwrap();
        assert_eq!((s.max_cll(), s.max_fall()), (200, 150));
        let m = s.push_frame(&[50.0, 300.0], true).unwrap();
        assert_eq!((m.max_cll, m.max_fall), (300, 175));
        assert!(m.scene_change);
        assert_eq!(s.frames(), 2);
        assert!(s.push_frame(&[], false).is_none());
        assert_eq!(s.frames(), 2, "an empty frame leaves the session unchanged");
    }

    #[test]
    fn metadata_codes_span_the_twelve_bit_range_for_both_curves() {
        let mut pq = FrameSession::new(Transfer::Pq, 24.0).unwrap();
        let peak = pq.push_frame(&[10000.0], true).unwrap();
        assert_eq!((peak.max_code, peak.average_code), (4095, 4095));
        let black = pq.push_frame(&[0.0], false).unwrap();
        assert_eq!((black.max_code, black.average_code), (0, 0));
        assert_eq!(black.transfer, Transfer::Pq);

        let mut hlg = FrameSession::new(Transfer::Hlg, 25.0).unwrap();
        let m = hlg.push_frame(&[1000.0], false).unwrap();
        assert_eq!(m.max_code, 4095, "1000 nits is the HLG nominal peak");
    }
}
