//! Seeded random source for screen saver motion. The `SSRandom...` helpers take unit values that the caller samples; this
//! module supplies those values from a seed, so a saver's motion can be replayed exactly. The generator is SplitMix64: it
//! keeps no OS entropy and no clock, and two sources with the same seed give the same sequence.

use crate::screensaver::{ss_random_float_between, ss_random_int_between, ss_random_point_for_size_within_rect};
use crate::Rect;

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// A deterministic random source (SplitMix64).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaverRandom {
    state: u64,
}

impl SaverRandom {
    /// A source that starts from `seed`.
    pub fn new(seed: u64) -> Self {
        SaverRandom { state: seed }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The next unit value in [0, 1), built from the top 53 bits so every value is exact in an `f64`.
    pub fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `SSRandomIntBetween` with the next unit value.
    pub fn next_int_between(&mut self, a: i32, b: i32) -> i32 {
        let u = self.next_unit();
        ss_random_int_between(u, a, b)
    }

    /// `SSRandomFloatBetween` with the next unit value.
    pub fn next_float_between(&mut self, a: f32, b: f32) -> f32 {
        let u = self.next_unit();
        ss_random_float_between(u, a, b)
    }

    /// `SSRandomPointForSizeWithinRect` with two unit values, one for each axis.
    pub fn next_point_within(&mut self, size: (f32, f32), rect: Rect) -> (f32, f32) {
        let ux = self.next_unit();
        let uy = self.next_unit();
        ss_random_point_for_size_within_rect(ux, uy, size, rect)
    }
}
