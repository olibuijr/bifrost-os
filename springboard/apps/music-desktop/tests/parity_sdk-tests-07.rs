//! Headless test for the background job queue of the Music window. A job runs on the dispatch pool, and its result is
//! queued on the main queue and taken by the window. The test runs the main queue by hand and draws nothing, so it
//! needs no display.

use bifrost_app::dispatch::{drain_main, Qos};
use bifrost_music::music::jobs::Jobs;

#[test]
fn headless_job_result_reaches_the_window_after_the_main_queue_runs() {
    let jobs: Jobs<u32> = Jobs::new(Qos::Utility);
    jobs.spawn(|| 6 * 7);
    let mut done = Vec::new();
    // The job runs on a worker thread; each pass runs whatever the main queue holds and takes what has arrived.
    while done.is_empty() {
        drain_main();
        done = jobs.drain();
        std::thread::yield_now();
    }
    assert_eq!(done, vec![42]);
    assert!(jobs.drain().is_empty(), "a result is taken once");
}

#[test]
fn headless_results_come_back_oldest_first() {
    let jobs: Jobs<&'static str> = Jobs::new(Qos::Background);
    jobs.spawn(|| "first");
    let mut done: Vec<&'static str> = Vec::new();
    while done.len() < 1 {
        drain_main();
        done.extend(jobs.drain());
        std::thread::yield_now();
    }
    jobs.spawn(|| "second");
    while done.len() < 2 {
        drain_main();
        done.extend(jobs.drain());
        std::thread::yield_now();
    }
    assert_eq!(done, vec!["first", "second"]);
}
