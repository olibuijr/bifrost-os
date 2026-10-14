//! Parity sdk-tests-18: headless tests of panic isolation in `bifrost_app::dispatch`. The queues run on the process
//! pool. Each test waits with a timeout, so a worker that died would fail the test instead of hanging it. No test
//! opens a window or a device.

use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn a_panicking_background_job_leaves_the_worker_free_for_the_next_job() {
    let queue = DispatchQueue::global(Qos::Background);
    queue.async_(|| panic!("background job failed"));
    let (tx, rx) = mpsc::channel();
    queue.async_(move || tx.send(7u32).unwrap());
    assert_eq!(rx.recv_timeout(WAIT), Ok(7));
}

#[test]
fn a_burst_of_panicking_jobs_does_not_stop_later_jobs_on_the_same_queue() {
    let queue = DispatchQueue::global(Qos::Utility);
    for i in 0..8 {
        queue.async_(move || panic!("job {i} failed"));
    }
    let (tx, rx) = mpsc::channel();
    for i in 0..4u32 {
        let tx = tx.clone();
        queue.async_(move || tx.send(i).unwrap());
    }
    let mut got: Vec<u32> = (0..4).map(|_| rx.recv_timeout(WAIT).unwrap()).collect();
    got.sort();
    assert_eq!(got, vec![0, 1, 2, 3], "every job after the panics still runs once");
}

#[test]
fn a_panicking_job_on_a_serial_queue_leaves_the_queue_usable() {
    let queue = DispatchQueue::serial("parity.sdk-tests-18.panic");
    queue.async_(|| panic!("serial job failed"));
    assert_eq!(queue.sync(|| 21 * 2), 42);
}

#[test]
fn sync_on_a_serial_queue_hands_the_panic_back_to_the_caller() {
    let queue = DispatchQueue::serial("parity.sdk-tests-18.sync-panic");
    let caught = catch_unwind(AssertUnwindSafe(|| queue.sync(|| -> u32 { panic!("inner failure") })));
    assert!(caught.is_err(), "the caller of sync must see the panic");
    assert_eq!(queue.sync(|| 5u32), 5, "the queue keeps serving after a panic in sync");
}

#[test]
fn sync_on_a_global_queue_runs_on_the_caller_and_passes_the_panic_up() {
    let caught = catch_unwind(|| DispatchQueue::global(Qos::UserInitiated).sync(|| -> u32 { panic!("caller side") }));
    assert!(caught.is_err());
    assert_eq!(DispatchQueue::global(Qos::UserInitiated).sync(|| 3u32), 3);
}
