//! Background jobs of the Music window (library scan, artwork and logo decodes, the station list) on the shared
//! dispatch pool. A job's result is queued on the main queue, which wakes the app loop; `Music::pump` takes it.

use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::sync::{Arc, Mutex};

/// Results of jobs on one QoS lane, waiting for the window to take them.
pub struct Jobs<T> {
    qos: Qos,
    done: Arc<Mutex<Vec<T>>>,
}

impl<T: Send + 'static> Jobs<T> {
    pub fn new(qos: Qos) -> Self {
        Jobs { qos, done: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Runs `job` on the global queue; its result is delivered on the main queue.
    pub fn spawn<F: FnOnce() -> T + Send + 'static>(&self, job: F) {
        let done = self.done.clone();
        DispatchQueue::global(self.qos).async_(move || {
            let result = job();
            DispatchQueue::main().async_(move || done.lock().unwrap().push(result));
        });
    }

    /// The finished results, oldest first.
    pub fn drain(&self) -> Vec<T> {
        std::mem::take(&mut *self.done.lock().unwrap())
    }
}
