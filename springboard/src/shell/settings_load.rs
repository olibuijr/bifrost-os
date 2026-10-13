//! A value the Settings pages read off the UI thread: the font library, crash logs, printers and the
//! like. Opening a page starts the read and builds the page from the last value that landed (the default
//! the first time), so opening never waits on the disk or on a service. `Settings::tick` takes the value
//! when it lands and rebuilds the page on top.

use std::sync::mpsc::{channel, Receiver, TryRecvError};

pub struct Loaded<T> {
    /// the last value a read landed (the default until the first one does)
    pub value: T,
    rx: Option<Receiver<T>>,
}

impl<T: Default> Default for Loaded<T> {
    fn default() -> Self {
        Loaded { value: T::default(), rx: None }
    }
}

impl<T: Send + 'static> Loaded<T> {
    /// Starts `read` on a worker thread. A read still running is abandoned: the newest request wins.
    pub fn start(&mut self, read: impl FnOnce() -> T + Send + 'static) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            // the receiver is gone when a newer read replaced this one: the answer is dropped
            let _ = tx.send(read());
        });
    }

    /// Takes the value a finished read landed; true when `value` changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(value) => {
                self.value = value;
                self.rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                false
            }
        }
    }

    /// A read is running.
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_lands_once_and_its_value_is_kept() {
        let mut l: Loaded<Vec<u8>> = Loaded::default();
        assert!(!l.poll(), "nothing is running");
        l.start(|| vec![1, 2]);
        assert!(l.busy());
        let began = std::time::Instant::now();
        while !l.poll() {
            assert!(began.elapsed() < std::time::Duration::from_secs(5), "the read never landed");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(l.value, vec![1, 2]);
        assert!(!l.busy());
        assert!(!l.poll(), "a value is taken once");
    }

    #[test]
    fn a_newer_read_replaces_one_still_running() {
        let mut l: Loaded<u32> = Loaded::default();
        l.start(|| 1);
        l.start(|| 2);
        let began = std::time::Instant::now();
        while !l.poll() {
            assert!(began.elapsed() < std::time::Duration::from_secs(5), "the read never landed");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(l.value, 2);
    }
}
