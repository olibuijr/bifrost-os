//! Parity sdk-complete-32: headless tests for the app random sources (`bifrost_app::random`). Pure host tests: no clock, no device and no network. Each test drives the public API.

use bifrost_app::random::{shuffle, Algorithm, Mt19937, RandomDistribution, RandomSource, Source};

#[test]
fn mersenne_twister_matches_the_reference_sequence_for_seed_5489() {
    let mut mt = Mt19937::new(5489);
    let first: Vec<u32> = (0..3).map(|_| mt.next_u32()).collect();
    assert_eq!(first, vec![3_499_211_612, 581_869_302, 3_890_346_734]);
}

#[test]
fn mersenne_twister_keeps_one_sequence_across_state_regenerations() {
    let mut a = Mt19937::new(42);
    let mut b = Mt19937::new(42);
    let long: Vec<u32> = (0..2_000).map(|_| a.next_u32()).collect();
    let again: Vec<u32> = (0..2_000).map(|_| b.next_u32()).collect();
    assert_eq!(long, again, "more than three state blocks come out in the same order");
    assert_ne!(long[..624], long[624..1248], "each block of 624 outputs differs from the one before");
}

#[test]
fn every_algorithm_gives_one_sequence_per_seed_and_another_for_another_seed() {
    for algorithm in [Algorithm::Xorshift64, Algorithm::SplitMix64, Algorithm::Mersenne] {
        let mut a = Source::new(algorithm, 9001);
        let mut b = Source::new(algorithm, 9001);
        let mut c = Source::new(algorithm, 9002);
        let first: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let same: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        let other: Vec<u64> = (0..16).map(|_| c.next_u64()).collect();
        assert_eq!(first, same, "{algorithm:?} repeats for one seed");
        assert_ne!(first, other, "{algorithm:?} differs for another seed");
        assert_eq!(a.algorithm(), algorithm);
    }
}

#[test]
fn the_mersenne_source_uses_the_low_32_bits_of_the_seed() {
    let mut low = Source::new(Algorithm::Mersenne, 5489);
    let mut high = Source::new(Algorithm::Mersenne, 5489 | (1 << 40));
    let a: Vec<u64> = (0..8).map(|_| low.next_u64()).collect();
    let b: Vec<u64> = (0..8).map(|_| high.next_u64()).collect();
    assert_eq!(a, b);
}

#[test]
fn a_zero_seed_still_gives_a_moving_sequence_for_the_xorshift_and_splitmix_sources() {
    for algorithm in [Algorithm::Xorshift64, Algorithm::SplitMix64] {
        let mut s = Source::new(algorithm, 0);
        let x = s.next_u64();
        let y = s.next_u64();
        assert!(x != 0 && x != y, "{algorithm:?} stays still from a zero seed");
    }
}

#[test]
fn a_seeded_source_deals_a_die_whose_faces_stay_in_range_and_all_appear() {
    let mut d6 = RandomDistribution::die(Source::new(Algorithm::Mersenne, 77), 6);
    let mut faces = [0u32; 7];
    for _ in 0..1_200 {
        let face = d6.next_int();
        assert!((1..=6).contains(&face));
        faces[face as usize] += 1;
    }
    assert!(faces[1..].iter().all(|&n| n > 100), "faces {faces:?}");
}

#[test]
fn a_shuffle_with_the_same_seed_gives_the_same_order_and_keeps_every_item() {
    let mut a: Vec<u32> = (0..52).collect();
    let mut b = a.clone();
    shuffle(&mut Source::new(Algorithm::SplitMix64, 314), &mut a);
    shuffle(&mut Source::new(Algorithm::SplitMix64, 314), &mut b);
    assert_eq!(a, b);
    assert_ne!(a, (0..52).collect::<Vec<u32>>());
    a.sort();
    assert_eq!(a, (0..52).collect::<Vec<u32>>());
}
