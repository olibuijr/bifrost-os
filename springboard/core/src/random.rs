//! Seeded random sources and distributions for games and simulations.
//! Every source is deterministic: one seed gives one sequence, on every machine.

/// A stream of random 64-bit words. Distributions draw their values from one of these.
pub trait RandomSource {
    fn next_u64(&mut self) -> u64;

    /// A uniform value in [0, 1).
    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A uniform integer in [0, bound). A bound of 0 gives 0.
    fn next_below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        // Reject the top sliver of the range so every residue is equally likely.
        let limit = u64::MAX / bound * bound;
        loop {
            let x = self.next_u64();
            if x < limit {
                return x % bound;
            }
        }
    }
}

/// Xorshift64: fast, with a single word of state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    /// A zero seed would stay zero forever, so it is replaced by a fixed non-zero constant.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed },
        }
    }
}

impl RandomSource for Xorshift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

/// SplitMix64: a 64-bit generator whose output mixes well from any seed, including 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }
}

impl RandomSource for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Whole numbers in [lowest, highest], both ends included. Every outcome is equally likely.
#[derive(Clone, Debug)]
pub struct RandomDistribution<R: RandomSource> {
    source: R,
    low: i64,
    high: i64,
}

impl<R: RandomSource> RandomDistribution<R> {
    /// Bounds given in the wrong order are swapped.
    pub fn new(source: R, low: i64, high: i64) -> Self {
        let (low, high) = if low <= high { (low, high) } else { (high, low) };
        Self { source, low, high }
    }

    /// A die with `sides` faces, numbered from 1.
    pub fn die(source: R, sides: u32) -> Self {
        Self::new(source, 1, i64::from(sides.max(1)))
    }

    pub fn lowest(&self) -> i64 {
        self.low
    }

    pub fn highest(&self) -> i64 {
        self.high
    }

    /// The number of unique whole values. Saturates at u64::MAX for the full i64 range.
    pub fn number_of_possible_outcomes(&self) -> u64 {
        let span = i128::from(self.high) - i128::from(self.low) + 1;
        u64::try_from(span).unwrap_or(u64::MAX)
    }

    /// A whole number in [lowest, highest].
    pub fn next_int(&mut self) -> i64 {
        let offset = self.source.next_below(self.number_of_possible_outcomes());
        (i128::from(self.low) + i128::from(offset)) as i64
    }

    /// A real number in [lowest, highest + 1).
    pub fn next_uniform(&mut self) -> f64 {
        self.low as f64 + self.source.next_unit() * self.number_of_possible_outcomes() as f64
    }

    pub fn next_bool(&mut self) -> bool {
        self.source.next_u64() & 1 == 1
    }
}

/// Real numbers with a normal (bell-shaped) distribution around `mean`, spread by `deviation`.
#[derive(Clone, Debug)]
pub struct GaussianDistribution<R: RandomSource> {
    source: R,
    mean: f64,
    deviation: f64,
    /// The second value of a Box-Muller pair, kept for the next call.
    spare: Option<f64>,
}

impl<R: RandomSource> GaussianDistribution<R> {
    /// A negative deviation is taken as its absolute value.
    pub fn new(source: R, mean: f64, deviation: f64) -> Self {
        Self {
            source,
            mean,
            deviation: deviation.abs(),
            spare: None,
        }
    }

    pub fn next_gaussian(&mut self) -> f64 {
        let z = match self.spare.take() {
            Some(z) => z,
            None => {
                // Box-Muller turns two uniform values into two independent standard normal values.
                // The first must not be 0, because ln(0) is undefined.
                let mut u1 = self.source.next_unit();
                while u1 == 0.0 {
                    u1 = self.source.next_unit();
                }
                let u2 = self.source.next_unit();
                let r = (-2.0 * u1.ln()).sqrt();
                let theta = std::f64::consts::TAU * u2;
                self.spare = Some(r * theta.sin());
                r * theta.cos()
            }
        };
        self.mean + self.deviation * z
    }
}

/// Whole numbers in [lowest, highest] drawn from a shuffled deck: each outcome comes once per pass,
/// so short runs of the same value are rare. A new pass never starts with the value the last one ended on.
/// The deck holds one entry per outcome, so keep the range dice-sized.
#[derive(Clone, Debug)]
pub struct ShuffledDistribution<R: RandomSource> {
    source: R,
    deck: Vec<i64>,
    next: usize,
    last: Option<i64>,
}

impl<R: RandomSource> ShuffledDistribution<R> {
    /// Bounds given in the wrong order are swapped.
    pub fn new(source: R, low: i64, high: i64) -> Self {
        let (low, high) = if low <= high { (low, high) } else { (high, low) };
        let mut d = Self {
            source,
            deck: (low..=high).collect(),
            next: 0,
            last: None,
        };
        d.shuffle_deck();
        d
    }

    fn shuffle_deck(&mut self) {
        shuffle(&mut self.source, &mut self.deck);
        if self.deck.len() > 1 && self.deck.first() == self.last.as_ref() {
            self.deck.swap(0, 1);
        }
        self.next = 0;
    }

    /// The next whole number. The deck is never empty, so this always returns a value.
    pub fn next_int(&mut self) -> i64 {
        if self.next >= self.deck.len() {
            self.shuffle_deck();
        }
        let value = self.deck[self.next];
        self.next += 1;
        self.last = Some(value);
        value
    }
}

/// Fisher-Yates shuffle in place: every order is equally likely.
pub fn shuffle<T, R: RandomSource>(source: &mut R, items: &mut [T]) {
    for i in (1..items.len()).rev() {
        let j = source.next_below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_seed_gives_one_sequence_and_a_zero_seed_still_moves() {
        let mut a = Xorshift64::new(42);
        let mut b = Xorshift64::new(42);
        let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let second: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(first, second);
        let mut zero = Xorshift64::new(0);
        let x = zero.next_u64();
        let y = zero.next_u64();
        assert!(x != 0 && x != y);
    }

    #[test]
    fn next_below_stays_in_range_and_reaches_every_value() {
        let mut s = SplitMix64::new(7);
        let mut seen = [false; 6];
        for _ in 0..600 {
            let v = s.next_below(6) as usize;
            assert!(v < 6);
            seen[v] = true;
        }
        assert!(seen.iter().all(|&hit| hit));
        assert_eq!(s.next_below(0), 0);
    }

    #[test]
    fn dice_and_ranges_respect_their_ends_and_swapped_bounds() {
        let mut d6 = RandomDistribution::die(SplitMix64::new(1), 6);
        let mut faces = [0u32; 7];
        for _ in 0..1200 {
            let face = d6.next_int();
            assert!((1..=6).contains(&face));
            faces[face as usize] += 1;
        }
        assert!(faces[1..].iter().all(|&n| n > 100), "faces {faces:?}");

        let mut r = RandomDistribution::new(SplitMix64::new(9), 9, 3);
        assert_eq!((r.lowest(), r.highest()), (3, 9));
        assert_eq!(r.number_of_possible_outcomes(), 7);
        let v = r.next_int();
        assert!((3..=9).contains(&v));

        let mut full = RandomDistribution::new(SplitMix64::new(9), i64::MIN, i64::MAX);
        assert_eq!(full.number_of_possible_outcomes(), u64::MAX);
        let _ = full.next_int();
    }

    #[test]
    fn gaussian_centres_on_the_mean_with_the_given_spread() {
        let mut g = GaussianDistribution::new(SplitMix64::new(3), 10.0, 2.0);
        let n = 20_000;
        let xs: Vec<f64> = (0..n).map(|_| g.next_gaussian()).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        assert!((mean - 10.0).abs() < 0.1, "mean {mean}");
        assert!((var.sqrt() - 2.0).abs() < 0.1, "deviation {}", var.sqrt());
    }

    #[test]
    fn shuffled_passes_use_each_value_once_and_never_repeat_at_the_seam() {
        let mut s = ShuffledDistribution::new(SplitMix64::new(5), 1, 6);
        let draws: Vec<i64> = (0..600).map(|_| s.next_int()).collect();
        for pass in draws.chunks(6) {
            let mut sorted = pass.to_vec();
            sorted.sort();
            assert_eq!(sorted, vec![1, 2, 3, 4, 5, 6]);
        }
        assert!(draws.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn shuffle_keeps_every_item_and_follows_the_seed() {
        let mut a: Vec<u32> = (0..20).collect();
        let mut b = a.clone();
        shuffle(&mut SplitMix64::new(11), &mut a);
        shuffle(&mut SplitMix64::new(11), &mut b);
        assert_eq!(a, b);
        assert_ne!(a, (0..20).collect::<Vec<u32>>());
        a.sort();
        assert_eq!(a, (0..20).collect::<Vec<u32>>());
    }
}
