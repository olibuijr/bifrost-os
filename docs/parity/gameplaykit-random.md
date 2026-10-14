# GameplayKit randomization (app random sources)

Mirrors: the GameplayKit random family (`GKRandomSource`, `GKMersenneTwisterRandomSource`, `GKRandomDistribution`,
`GKGaussianDistribution`, `GKShuffledDistribution`).

Code:

- `springboard/core/src/random.rs`: the generators (xorshift64, SplitMix64), the distributions (uniform whole numbers, real
  numbers, Gaussian, shuffled bag) and the Fisher-Yates shuffle.
- `springboard/sdk/src/random.rs` (`bifrost_app::random`): re-exports the core types, adds `Mt19937` (the reference
  Mersenne Twister), `Algorithm` (the choice an app makes) and `Source` (a generator chosen at run time).
- `springboard/sdk/tests/parity_sdk-complete-32.rs`: headless tests of the reference sequence, per-seed repeatability,
  the seed truncation rule and the distributions over `Source`.

Wired: the Games app draws its deals from `bifrost_core::random`. Apps that need the SDK surface take `bifrost_app::random`.

Not offered: `GKARC4RandomSource`. RC4 is a broken cipher and no game needs it.

Needs the lab machine: only a check that a seeded deal on Midget matches the host sequence.
