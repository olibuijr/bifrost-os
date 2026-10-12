//! Results from background work (curl probes, icon fetches, page sources) back to the app loop. A worker runs on the
//! global queue, fills a shared slot and queues an empty closure on the main queue, which wakes the loop; `tick`
//! takes whatever is in the slot. No channel and no polling thread.
use std::sync::{Arc, Mutex};

use bifrost_app::dispatch::{DispatchQueue, Qos};

/// One result, filled once by a worker and taken once by the loop.
pub type Slot<T> = Arc<Mutex<Option<T>>>;

pub fn slot<T>() -> Slot<T> {
    Arc::new(Mutex::new(None))
}

/// Runs `work` on the global queue; its result lands in the returned slot and wakes the loop.
pub fn spawn<T, F>(work: F) -> Slot<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let out = slot();
    let sink = out.clone();
    DispatchQueue::global(Qos::Utility).async_(move || {
        let v = work();
        if let Ok(mut g) = sink.lock() {
            *g = Some(v);
        }
        DispatchQueue::main().async_(|| {});
    });
    out
}

/// The result, once the worker has filled the slot.
pub fn take<T>(slot: &Slot<T>) -> Option<T> {
    slot.lock().ok().and_then(|mut g| g.take())
}

/// Wakes the loop so the next tick collects what a worker queued.
pub fn wake() {
    DispatchQueue::main().async_(|| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_slot_takes_nothing_and_a_filled_one_yields_once() {
        let s: Slot<u32> = slot();
        assert_eq!(take(&s), None);
        *s.lock().unwrap() = Some(7);
        assert_eq!(take(&s), Some(7));
        assert_eq!(take(&s), None);
    }
}
