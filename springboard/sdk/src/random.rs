//! Random sources for apps (the GameplayKit `GKRandomSource` family). The generators and distributions are in
//! `bifrost_core::random`, re-exported here, so an app takes one import. This module adds the Mersenne Twister
//! (MT19937, the reference generator behind GameplayKit's Mersenne source) and [`Algorithm`], a choice an app makes
//! once and seeds from `testing::seed()` so a test kit run deals the same sequence.
//!
//! A source is deterministic: one algorithm and one seed give one sequence on every machine. Nothing here reads the
//! clock. See docs/parity/gameplaykit-random.md.

pub use bifrost_core::random::{
    shuffle, GaussianDistribution, RandomDistribution, RandomSource, ShuffledDistribution, SplitMix64, Xorshift64,
};

const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_MATRIX_A: u32 = 0x9908_b0df;
const MT_UPPER_MASK: u32 = 0x8000_0000;
const MT_LOWER_MASK: u32 = 0x7fff_ffff;

/// MT19937: a 32-bit Mersenne Twister with a 19937-bit period. Its sequence for a 32-bit seed is the published
/// reference sequence, so a game can reproduce a deal from another system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mt19937 {
    state: [u32; MT_N],
    index: usize,
}

impl Mt19937 {
    /// A generator seeded with `seed`, the standard initialisation of the reference algorithm.
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; MT_N];
        state[0] = seed;
        for i in 1..MT_N {
            let prev = state[i - 1];
            state[i] = 1_812_433_253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        Mt19937 { state, index: MT_N }
    }

    /// The next 32-bit output.
    pub fn next_u32(&mut self) -> u32 {
        if self.index >= MT_N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// Regenerates the whole state block in place, as the reference algorithm does after every 624 outputs.
    fn twist(&mut self) {
        for i in 0..MT_N {
            let y = (self.state[i] & MT_UPPER_MASK) | (self.state[(i + 1) % MT_N] & MT_LOWER_MASK);
            let mut next = self.state[(i + MT_M) % MT_N] ^ (y >> 1);
            if y & 1 == 1 {
                next ^= MT_MATRIX_A;
            }
            self.state[i] = next;
        }
        self.index = 0;
    }
}

impl RandomSource for Mt19937 {
    fn next_u64(&mut self) -> u64 {
        let high = u64::from(self.next_u32());
        (high << 32) | u64::from(self.next_u32())
    }
}

/// The generator an app asks for. Each one is deterministic for a given seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// Fast, one word of state. The default for games whose deals only need to look random.
    Xorshift64,
    /// Mixes well from any seed, including zero.
    SplitMix64,
    /// The Mersenne Twister, for sequences that must match the published reference.
    Mersenne,
}

/// A generator chosen at run time. It implements [`RandomSource`], so every distribution takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Xorshift(Xorshift64),
    SplitMix(SplitMix64),
    Mersenne(Mt19937),
}

impl Source {
    /// A source of `algorithm` seeded with `seed`. The Mersenne Twister takes the low 32 bits of the seed, so two
    /// seeds that differ only above bit 31 deal the same sequence with that algorithm.
    pub fn new(algorithm: Algorithm, seed: u64) -> Self {
        match algorithm {
            Algorithm::Xorshift64 => Source::Xorshift(Xorshift64::new(seed)),
            Algorithm::SplitMix64 => Source::SplitMix(SplitMix64::new(seed)),
            Algorithm::Mersenne => Source::Mersenne(Mt19937::new(seed as u32)),
        }
    }

    /// The algorithm this source was built with.
    pub fn algorithm(&self) -> Algorithm {
        match self {
            Source::Xorshift(_) => Algorithm::Xorshift64,
            Source::SplitMix(_) => Algorithm::SplitMix64,
            Source::Mersenne(_) => Algorithm::Mersenne,
        }
    }
}

impl RandomSource for Source {
    fn next_u64(&mut self) -> u64 {
        match self {
            Source::Xorshift(s) => s.next_u64(),
            Source::SplitMix(s) => s.next_u64(),
            Source::Mersenne(s) => s.next_u64(),
        }
    }
}
