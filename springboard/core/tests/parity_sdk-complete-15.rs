//! Parity sdk-complete-15: headless tests of the recorded sensor history. The log is pure in-memory state: no sensor,
//! no file and no clock.

use bifrost_core::sensor_log::SensorLog;

fn filled(count: usize) -> SensorLog<usize> {
    let mut log = SensorLog::new(100);
    for i in 0..count {
        assert!(log.record(i as f64 * 0.1, i));
    }
    log
}

#[test]
fn headless_a_range_splits_into_full_batches_and_a_short_last_batch() {
    let log = filled(7);
    let sizes: Vec<usize> = log.batches(0.0, 10.0, 3).iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![3, 3, 1]);
}

#[test]
fn headless_batches_keep_the_samples_in_time_order() {
    let log = filled(7);
    let flat: Vec<usize> = log.batches(0.0, 10.0, 2).into_iter().flatten().map(|(_, v)| v).collect();
    assert_eq!(flat, vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn headless_a_batch_size_at_least_the_range_gives_one_batch() {
    let log = filled(4);
    let batches = log.batches(0.0, 10.0, 4);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].first(), Some(&(0.0, 0)));
    assert_eq!(log.batches(0.0, 10.0, 99).len(), 1);
}

#[test]
fn headless_a_zero_batch_size_gives_no_batches() {
    assert!(filled(5).batches(0.0, 10.0, 0).is_empty());
}

#[test]
fn headless_a_range_with_no_samples_gives_no_batches() {
    let log = filled(5);
    assert!(log.batches(50.0, 60.0, 2).is_empty());
    assert!(log.batches(0.3, 0.1, 2).is_empty(), "a backwards range holds nothing");
}

#[test]
fn headless_batches_respect_the_range_ends() {
    // Samples sit at 0.0, 0.1, 0.2, 0.3 and 0.4: the range 0.1..=0.4 holds four of them, the first at 0.1.
    let log = filled(5);
    let batches = log.batches(0.1, 0.4, 3);
    let sizes: Vec<usize> = batches.iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![3, 1]);
    assert_eq!(batches[0].first(), Some(&(0.1, 1)));
}

#[test]
fn headless_a_full_history_is_kept_only_up_to_its_capacity_before_batching() {
    let mut log = SensorLog::new(4);
    for i in 0..6 {
        log.record(i as f64, i);
    }
    let flat: Vec<usize> = log.batches(f64::MIN, f64::MAX, 3).into_iter().flatten().map(|(_, v)| v).collect();
    assert_eq!(flat, vec![2, 3, 4, 5], "the oldest samples are dropped before batching");
}
