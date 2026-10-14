//! Parity sdk-tests-17: headless tests of the memory-pressure source in `bifrost_app::dispatch`. The kernel never
//! reports pressure to a test, so each test reports a level through `bifrost_render::purgeable::notify_pressure`,
//! the entry point the watcher uses, and asserts what the handlers and the caches do. No display, no device.

use bifrost_app::dispatch::{DispatchQueue, DispatchSource, MemoryPressure};
use bifrost_render::purgeable::{notify_pressure, Cache};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, ThreadId};

/// Pressure events reach every listener in the process, so the tests that send them run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn a_pressure_level_reaches_the_handler_on_its_queue() {
    let _guard = serial();
    let queue = DispatchQueue::serial("bifrost.tests.pressure.levels");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    DispatchSource::memory_pressure(&queue, move |level| sink.lock().unwrap().push(level));
    notify_pressure(MemoryPressure::Warning);
    // A serial queue runs the handler before this block returns.
    queue.sync(|| ());
    assert_eq!(*seen.lock().unwrap(), vec![MemoryPressure::Warning]);
}

#[test]
fn every_level_is_delivered_in_the_order_it_was_reported() {
    let _guard = serial();
    let queue = DispatchQueue::serial("bifrost.tests.pressure.order");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    DispatchSource::memory_pressure(&queue, move |level| sink.lock().unwrap().push(level));
    for level in [MemoryPressure::Warning, MemoryPressure::Critical, MemoryPressure::Normal] {
        notify_pressure(level);
    }
    queue.sync(|| ());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![MemoryPressure::Warning, MemoryPressure::Critical, MemoryPressure::Normal]
    );
}

#[test]
fn the_handler_runs_on_the_queue_thread_not_the_reporting_thread() {
    let _guard = serial();
    let queue = DispatchQueue::serial("bifrost.tests.pressure.thread");
    let ran_on: Arc<Mutex<Option<ThreadId>>> = Arc::new(Mutex::new(None));
    let sink = ran_on.clone();
    DispatchSource::memory_pressure(&queue, move |_| {
        *sink.lock().unwrap() = Some(thread::current().id());
    });
    notify_pressure(MemoryPressure::Critical);
    let queue_thread = queue.sync(|| thread::current().id());
    let handler_thread = (*ran_on.lock().unwrap()).expect("the handler ran");
    assert_eq!(handler_thread, queue_thread);
    assert_ne!(handler_thread, thread::current().id());
}

#[test]
fn every_registered_handler_receives_the_level_on_its_own_queue() {
    let _guard = serial();
    let first = DispatchQueue::serial("bifrost.tests.pressure.first");
    let second = DispatchQueue::serial("bifrost.tests.pressure.second");
    let a = Arc::new(Mutex::new(Vec::new()));
    let b = Arc::new(Mutex::new(Vec::new()));
    let (sa, sb) = (a.clone(), b.clone());
    DispatchSource::memory_pressure(&first, move |level| sa.lock().unwrap().push(level));
    DispatchSource::memory_pressure(&second, move |level| sb.lock().unwrap().push(level));
    notify_pressure(MemoryPressure::Critical);
    first.sync(|| ());
    second.sync(|| ());
    assert_eq!(*a.lock().unwrap(), vec![MemoryPressure::Critical]);
    assert_eq!(*b.lock().unwrap(), vec![MemoryPressure::Critical]);
}

#[test]
fn a_warning_purges_half_the_cache_cost_oldest_first_and_a_critical_purges_the_rest() {
    let _guard = serial();
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    for i in 0..10 {
        cache.set(i, i, 10);
    }
    notify_pressure(MemoryPressure::Warning);
    assert_eq!(cache.total_cost(), 50, "a warning drops half of the 100 cost held");
    assert_eq!(cache.get(&0), None, "the oldest entry goes first");
    assert_eq!(cache.get(&9), Some(9), "the newest entry stays");
    notify_pressure(MemoryPressure::Critical);
    assert_eq!(cache.total_cost(), 0, "a critical drops 90 percent, which takes every 10-cost entry left");
}

#[test]
fn a_normal_level_leaves_the_caches_alone() {
    let _guard = serial();
    let cache: Cache<u32, u32> = Cache::new(0, 0);
    cache.set(1, 1, 7);
    notify_pressure(MemoryPressure::Normal);
    assert_eq!(cache.total_cost(), 7);
    assert_eq!(cache.get(&1), Some(1));
}
