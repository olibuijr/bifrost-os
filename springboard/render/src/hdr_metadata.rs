//! HDR transfer functions and per-frame light metadata, as a pure model (std only).
//!
//! HDR video codes light with one of two transfer functions: the perceptual quantizer
//! (PQ, SMPTE ST 2084, absolute luminance up to 10000 cd/m2) or hybrid log-gamma (HLG,
//! ITU-R BT.2100). This module converts coded values to light and back, and runs a
//! session over a sequence of 10-bit luma frames. For each frame it reports the peak
//! and the mean light, and it keeps the scene peak, which restarts at a scene change.
//! It writes no bitstream and attaches nothing to a buffer; the caller stores the
//! returned values with the frame. The transfer is the `Transfer` tag of `buffer_attach`.

use crate::buffer_attach::Transfer;

/// Luminance of a PQ code value of 1.0, in cd/m2.
pub const PQ_PEAK_NITS: f64 = 10000.0;

/// Nominal peak of an HLG signal, in cd/m2 (the light of a scene-linear value of 1.0).
pub const HLG_NOMINAL_NITS: f64 = 1000.0;

/// Largest 10-bit code value.
pub const CODE_MAX: u16 = 1023;

// SMPTE ST 2084 constants, as rational numbers over 4096 and 16384.
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;

// ITU-R BT.2100 hybrid log-gamma constants.
const HLG_A: f64 = 0.17883277;
const HLG_B: f64 = 1.0 - 4.0 * HLG_A;

/// PQ electro-optical transfer: a coded value in 0..=1 to luminance in cd/m2.
pub fn pq_to_nits(coded: f64) -> f64 {
    let e = coded.clamp(0.0, 1.0);
    let p = e.powf(1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * p;
    PQ_PEAK_NITS * (num / den).powf(1.0 / PQ_M1)
}

/// PQ inverse: luminance in cd/m2 to a coded value in 0..=1.
pub fn nits_to_pq(nits: f64) -> f64 {
    let y = (nits / PQ_PEAK_NITS).clamp(0.0, 1.0);
    let yp = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * yp) / (1.0 + PQ_C3 * yp)).powf(PQ_M2)
}

/// HLG opto-electrical transfer: scene-linear light in 0..=1 to a coded value in 0..=1.
pub fn hlg_oetf(linear: f64) -> f64 {
    let e = linear.clamp(0.0, 1.0);
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt()
    } else {
        HLG_A * (12.0 * e - HLG_B).ln() + (0.5 - HLG_A * (4.0 * HLG_A).ln())
    }
}

/// HLG inverse: a coded value in 0..=1 to scene-linear light in 0..=1.
pub fn hlg_eotf(coded: f64) -> f64 {
    let c = coded.clamp(0.0, 1.0);
    if c == 1.0 {
        1.0
    } else if c <= 0.5 {
        c * c / 3.0
    } else {
        (((c - (0.5 - HLG_A * (4.0 * HLG_A).ln())) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// Light of a 10-bit code value under a transfer. Only PQ and HLG are HDR transfers.
pub fn code_to_nits(code: u16, transfer: Transfer) -> Result<f64, HdrError> {
    if code > CODE_MAX {
        return Err(HdrError::SampleOutOfRange(code));
    }
    let coded = f64::from(code) / f64::from(CODE_MAX);
    match transfer {
        Transfer::Pq => Ok(pq_to_nits(coded)),
        Transfer::Hlg => Ok(hlg_eotf(coded) * HLG_NOMINAL_NITS),
        _ => Err(HdrError::UnsupportedTransfer),
    }
}

/// Why a session or a frame was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum HdrError {
    /// The frame rate is zero, negative or not finite.
    InvalidFrameRate,
    /// The transfer is not PQ or HLG.
    UnsupportedTransfer,
    /// The frame has no samples.
    EmptyFrame,
    /// A sample is above the 10-bit maximum.
    SampleOutOfRange(u16),
}

/// Light metadata for one frame. Values are in cd/m2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLight {
    /// Zero-based position of the frame in the session.
    pub index: u64,
    /// Brightest sample of this frame.
    pub frame_peak_nits: f64,
    /// Mean light of the frame samples (a plain average, not weighted by area).
    pub frame_average_nits: f64,
    /// Brightest frame peak since the last scene change, including this frame.
    pub scene_peak_nits: f64,
}

/// Generates per-frame light metadata for a sequence of frames with one transfer.
#[derive(Clone, Debug)]
pub struct FrameLightSession {
    frames_per_second: f64,
    transfer: Transfer,
    next_index: u64,
    scene_peak: f64,
}

impl FrameLightSession {
    /// A session for frames at `frames_per_second`, coded with PQ or HLG.
    pub fn new(frames_per_second: f64, transfer: Transfer) -> Result<Self, HdrError> {
        if !(frames_per_second.is_finite() && frames_per_second > 0.0) {
            return Err(HdrError::InvalidFrameRate);
        }
        match transfer {
            Transfer::Pq | Transfer::Hlg => {}
            _ => return Err(HdrError::UnsupportedTransfer),
        }
        Ok(Self { frames_per_second, transfer, next_index: 0, scene_peak: 0.0 })
    }

    /// The frame rate the session was made for.
    pub fn frames_per_second(&self) -> f64 {
        self.frames_per_second
    }

    /// Metadata for one frame of 10-bit luma codes. A scene change restarts the scene peak
    /// before this frame is counted.
    pub fn attach(&mut self, samples: &[u16], scene_change: bool) -> Result<FrameLight, HdrError> {
        if samples.is_empty() {
            return Err(HdrError::EmptyFrame);
        }
        let mut peak = 0.0_f64;
        let mut sum = 0.0_f64;
        for &code in samples {
            let nits = code_to_nits(code, self.transfer)?;
            peak = peak.max(nits);
            sum += nits;
        }
        if scene_change {
            self.scene_peak = 0.0;
        }
        self.scene_peak = self.scene_peak.max(peak);
        let light = FrameLight {
            index: self.next_index,
            frame_peak_nits: peak,
            frame_average_nits: sum / samples.len() as f64,
            scene_peak_nits: self.scene_peak,
        };
        self.next_index += 1;
        Ok(light)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn pq_endpoints_are_exact() {
        assert_eq!(pq_to_nits(0.0), 0.0);
        assert!(close(pq_to_nits(1.0), 10000.0, 1e-6));
        // the curve's offset c1 makes black code to c1^m2 (about 7.3e-7), not exactly 0
        assert!(close(nits_to_pq(0.0), 0.8359375_f64.powf(78.84375), 1e-12));
        assert!(nits_to_pq(0.0) < 1e-6);
        assert!(close(nits_to_pq(10000.0), 1.0, 1e-9));
    }

    #[test]
    fn pq_round_trips_across_the_range() {
        for nits in [1.0_f64, 50.0, 100.0, 1000.0, 4000.0] {
            let back = pq_to_nits(nits_to_pq(nits));
            assert!(close(back, nits, nits * 1e-6), "{nits} came back as {back}");
        }
    }

    #[test]
    fn hlg_joins_its_two_branches_at_one_twelfth() {
        assert_eq!(hlg_oetf(0.0), 0.0);
        assert!(close(hlg_oetf(1.0 / 12.0), 0.5, 1e-12));
        assert!(close(hlg_eotf(0.5), 1.0 / 12.0, 1e-12));
        assert!(close(hlg_eotf(hlg_oetf(0.25)), 0.25, 1e-9));
    }

    #[test]
    fn frame_peak_and_average_follow_the_samples() {
        let mut s = FrameLightSession::new(24.0, Transfer::Pq).unwrap();
        let light = s.attach(&[0, 0, 0, CODE_MAX], false).unwrap();
        assert_eq!(light.index, 0);
        assert_eq!(light.frame_peak_nits, 10000.0);
        assert!(close(light.frame_average_nits, 2500.0, 1e-6));
        assert_eq!(light.scene_peak_nits, 10000.0);
    }

    #[test]
    fn scene_peak_restarts_only_at_a_scene_change() {
        let mut s = FrameLightSession::new(30.0, Transfer::Pq).unwrap();
        let first = s.attach(&[CODE_MAX], false).unwrap();
        assert_eq!(first.scene_peak_nits, 10000.0);
        let dark = s.attach(&[0], false).unwrap();
        assert_eq!(dark.frame_peak_nits, 0.0);
        assert_eq!(dark.scene_peak_nits, 10000.0, "no cut, so the scene keeps its peak");
        let cut = s.attach(&[0], true).unwrap();
        assert_eq!(cut.scene_peak_nits, 0.0, "a cut restarts the scene peak");
        assert_eq!(cut.index, 2);
    }

    #[test]
    fn invalid_input_is_refused() {
        assert_eq!(FrameLightSession::new(0.0, Transfer::Pq).unwrap_err(), HdrError::InvalidFrameRate);
        assert_eq!(FrameLightSession::new(f64::NAN, Transfer::Hlg).unwrap_err(), HdrError::InvalidFrameRate);
        assert_eq!(FrameLightSession::new(25.0, Transfer::Srgb).unwrap_err(), HdrError::UnsupportedTransfer);
        let mut s = FrameLightSession::new(25.0, Transfer::Hlg).unwrap();
        assert_eq!(s.attach(&[], false).unwrap_err(), HdrError::EmptyFrame);
        assert_eq!(s.attach(&[1024], false).unwrap_err(), HdrError::SampleOutOfRange(1024));
    }
}
