//! High dynamic range transfer curves and extended range headroom, as plain f64 math.
//!
//! Two encoded curves are modelled. The perceptual quantizer (SMPTE ST 2084, used by HDR10) maps absolute
//! luminance in nits (0 to 10000) to a code value in 0..=1. The hybrid log-gamma curve (ARIB STD-B67) maps scene
//! linear light, where 1.0 is the nominal reference white, to a code value in 0..=1. Both are the published
//! formulas; the functions hold no tables.
//!
//! `EdrHeadroom` models extended range output: a display shows SDR content at a reference white, and can show
//! brighter values up to its peak. A value is expressed as a multiple of the reference white and is clamped to
//! the range the display can show. The model does not read the display, it takes the two luminances as given.

/// ST 2084 constants: m1 = 2610/16384, m2 = 2523/4096 * 128, c1 = 3424/4096, c2 = 2413/4096 * 32, c3 = 2392/4096 * 32.
const PQ_M1: f64 = 0.1593017578125;
const PQ_M2: f64 = 78.84375;
const PQ_C1: f64 = 0.8359375;
const PQ_C2: f64 = 18.8515625;
const PQ_C3: f64 = 18.6875;
/// The luminance that a code value of 1.0 stands for, in nits.
const PQ_PEAK_NITS: f64 = 10000.0;

/// ARIB STD-B67 constants.
const HLG_A: f64 = 0.17883277;
const HLG_B: f64 = 0.28466892;
const HLG_C: f64 = 0.55991073;

/// Encodes absolute luminance in nits with the perceptual quantizer. Input is clamped to 0..=10000 nits.
pub fn pq_encode(nits: f64) -> f64 {
    let y = (nits / PQ_PEAK_NITS).clamp(0.0, 1.0);
    let ym = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * ym) / (1.0 + PQ_C3 * ym)).powf(PQ_M2)
}

/// Decodes a perceptual quantizer code value (clamped to 0..=1) to absolute luminance in nits.
pub fn pq_decode(code: f64) -> f64 {
    let e = code.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    let num = (e - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * e;
    (num / den).powf(1.0 / PQ_M1) * PQ_PEAK_NITS
}

/// Encodes scene linear light (1.0 is reference white, negative input is treated as 0) with hybrid log-gamma.
pub fn hlg_encode(scene: f64) -> f64 {
    let e = scene.max(0.0);
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt()
    } else {
        HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
    }
}

/// Decodes a hybrid log-gamma code value (clamped to 0..=1) back to scene linear light.
pub fn hlg_decode(code: f64) -> f64 {
    let v = code.clamp(0.0, 1.0);
    if v <= 0.5 {
        v * v / 3.0
    } else {
        (((v - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// Tone maps a luminance from a source mastered to `source_peak` nits onto a display with `display_peak` nits, with the
/// BT.2390 EETF (ITU-R BT.2390, section 5.4.2) applied in the PQ domain. Luminance below the knee passes unchanged; above
/// it a Hermite spline rolls off so that the source peak lands on the display peak. A display at least as bright as the
/// source keeps the luminance. The input is clamped to 0..=`source_peak`. None unless both peaks are finite and positive.
pub fn tone_map_nits(nits: f64, source_peak: f64, display_peak: f64) -> Option<f64> {
    if !source_peak.is_finite() || !display_peak.is_finite() || source_peak <= 0.0 || display_peak <= 0.0 {
        return None;
    }
    let x = nits.clamp(0.0, source_peak);
    if display_peak >= source_peak {
        return Some(x);
    }
    let source_code = pq_encode(source_peak);
    let max_lum = pq_encode(display_peak) / source_code;
    let e1 = pq_encode(x) / source_code;
    let knee = (1.5 * max_lum - 0.5).clamp(0.0, 1.0);
    let e2 = if e1 <= knee { e1 } else { bt2390_spline(e1, knee, max_lum) };
    Some(pq_decode(e2 * source_code))
}

/// The BT.2390 roll-off above the knee `ks`, for an input `e` in `ks..=1` and a target maximum `max_lum`. It is
/// the identity at the knee and maps 1 to `max_lum`.
fn bt2390_spline(e: f64, ks: f64, max_lum: f64) -> f64 {
    let t = (e - ks) / (1.0 - ks);
    let t2 = t * t;
    let t3 = t2 * t;
    (2.0 * t3 - 3.0 * t2 + 1.0) * ks + (t3 - 2.0 * t2 + t) * (1.0 - ks) + (-2.0 * t3 + 3.0 * t2) * max_lum
}

/// Extended range headroom of a display: the reference white (SDR white, 1.0) and the peak it can show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdrHeadroom {
    reference_nits: f64,
    peak_nits: f64,
}

impl EdrHeadroom {
    /// A headroom for a display with `reference_nits` of SDR white and `peak_nits` at most. A peak below the
    /// reference is raised to it, so the headroom is never below 1. None unless both are finite and positive.
    pub fn new(reference_nits: f64, peak_nits: f64) -> Option<Self> {
        if !reference_nits.is_finite() || !peak_nits.is_finite() || reference_nits <= 0.0 || peak_nits <= 0.0 {
            return None;
        }
        Some(EdrHeadroom { reference_nits, peak_nits: peak_nits.max(reference_nits) })
    }

    /// The SDR reference white in nits.
    pub fn reference_nits(&self) -> f64 {
        self.reference_nits
    }

    /// The brightest value the display shows, as a multiple of the reference white. At least 1.
    pub fn headroom(&self) -> f64 {
        self.peak_nits / self.reference_nits
    }

    /// A luminance in nits as a multiple of the reference white, clamped to 0..=headroom.
    pub fn extended_value(&self, nits: f64) -> f64 {
        (nits / self.reference_nits).clamp(0.0, self.headroom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_decodes_half_code_to_about_92_nits() {
        let nits = pq_decode(0.5);
        assert!(nits > 92.0 && nits < 92.6, "got {nits}");
    }

    #[test]
    fn pq_full_code_is_the_ten_thousand_nit_peak() {
        assert!((pq_encode(10000.0) - 1.0).abs() < 1e-12);
        assert!((pq_decode(1.0) - 10000.0).abs() < 1e-6);
    }

    #[test]
    fn pq_round_trips_bright_luminance() {
        for nits in [1.0, 100.0, 1000.0, 4000.0] {
            let back = pq_decode(pq_encode(nits));
            assert!((back - nits).abs() / nits < 1e-9, "{nits} came back as {back}");
        }
    }

    #[test]
    fn hlg_knee_is_at_one_twelfth_and_half_code() {
        assert_eq!(hlg_encode(0.0), 0.0);
        assert!((hlg_encode(1.0 / 12.0) - 0.5).abs() < 1e-12);
        assert!((hlg_decode(0.5) - 1.0 / 12.0).abs() < 1e-12);
    }

    #[test]
    fn hlg_reference_white_encodes_to_full_code() {
        assert!((hlg_encode(1.0) - 1.0).abs() < 1e-4);
        assert!((hlg_decode(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn hlg_round_trips_scene_light() {
        for e in [0.05, 0.2, 0.5, 0.9] {
            let back = hlg_decode(hlg_encode(e));
            assert!((back - e).abs() < 1e-9, "{e} came back as {back}");
        }
    }

    #[test]
    fn headroom_is_peak_over_reference() {
        let h = EdrHeadroom::new(100.0, 1000.0).unwrap();
        assert_eq!(h.headroom(), 10.0);
        assert_eq!(h.reference_nits(), 100.0);
    }

    #[test]
    fn peak_below_reference_is_raised_to_reference() {
        let h = EdrHeadroom::new(100.0, 80.0).unwrap();
        assert_eq!(h.headroom(), 1.0);
    }

    #[test]
    fn invalid_luminance_is_refused() {
        assert_eq!(EdrHeadroom::new(0.0, 1000.0), None);
        assert_eq!(EdrHeadroom::new(100.0, -5.0), None);
        assert_eq!(EdrHeadroom::new(f64::NAN, 1000.0), None);
    }

    #[test]
    fn extended_value_is_clamped_to_the_display_range() {
        let h = EdrHeadroom::new(100.0, 1000.0).unwrap();
        assert_eq!(h.extended_value(500.0), 5.0);
        assert_eq!(h.extended_value(5000.0), 10.0);
        assert_eq!(h.extended_value(-20.0), 0.0);
    }
}
