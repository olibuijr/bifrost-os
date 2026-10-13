//! A value the Settings pages read off the UI thread: the font library, crash logs, printers and the
//! like. Opening a page starts the read and builds the page from the last value that landed (the default
//! the first time), so opening never waits on the disk or on a service. `Settings::tick` takes the value
//! when it lands and rebuilds the page on top. `get` gives the value only once a read has landed, for a
//! pane that shows a placeholder until then.

use std::sync::mpsc::{channel, Receiver, TryRecvError};

pub struct Loaded<T> {
    /// the last value a read landed (the default until the first one does)
    pub value: T,
    /// a read has landed, so `value` is the system's answer
    landed: bool,
    rx: Option<Receiver<T>>,
}

impl<T: Default> Default for Loaded<T> {
    fn default() -> Self {
        Loaded { value: T::default(), landed: false, rx: None }
    }
}

impl<T: Send + 'static> Loaded<T> {
    /// Starts `read` on a worker thread named `name`. A read still running is abandoned: the newest request wins.
    pub fn start(&mut self, name: &str, read: impl FnOnce() -> T + Send + 'static) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        crate::bare::sys::spawn_named(name, move || {
            crate::gfx::affinity::background_thread();
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
                self.landed = true;
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

    /// The last value that landed, or None before the first read lands.
    pub fn get(&self) -> Option<&T> {
        self.landed.then_some(&self.value)
    }

    /// The last value that landed, for an edit the page made itself; None before the first read lands.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.landed.then_some(&mut self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait<T: Send + 'static>(l: &mut Loaded<T>) -> bool {
        let began = std::time::Instant::now();
        while !l.poll() {
            if began.elapsed() > std::time::Duration::from_secs(5) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        true
    }

    #[test]
    fn a_read_lands_once_and_its_value_is_kept() {
        let mut l: Loaded<Vec<u8>> = Loaded::default();
        assert!(!l.poll(), "nothing is running");
        l.start("test-read", || vec![1, 2]);
        assert!(l.busy());
        assert!(wait(&mut l), "the read never landed");
        assert_eq!(l.value, vec![1, 2]);
        assert!(!l.busy());
        assert!(!l.poll(), "a value is taken once");
    }

    #[test]
    fn a_newer_read_replaces_one_still_running() {
        let mut l: Loaded<u32> = Loaded::default();
        l.start("test-read", || 1);
        l.start("test-read", || 2);
        assert!(wait(&mut l), "the read never landed");
        assert_eq!(l.value, 2);
    }

    #[test]
    fn nothing_is_loaded_before_a_read_starts() {
        let mut l: Loaded<u32> = Loaded::default();
        assert!(l.get().is_none());
        assert!(!l.busy());
        assert!(!l.poll());
    }

    #[test]
    fn the_answer_arrives_once_and_stays_on_hand() {
        let mut l = Loaded::default();
        l.start("test-read", || 7u32);
        assert!(wait(&mut l));
        assert_eq!(l.get(), Some(&7));
        assert!(!l.busy());
        assert!(!l.poll(), "the answer is taken once");
        assert_eq!(l.get(), Some(&7));
    }

    #[test]
    fn a_new_read_keeps_the_old_answer_until_it_lands() {
        let mut l = Loaded::default();
        l.start("test-read", || 1u32);
        assert!(wait(&mut l));
        l.start("test-read", || 2u32);
        assert_eq!(l.get(), Some(&1), "the old value stays on screen while the new read runs");
        assert!(wait(&mut l));
        assert_eq!(l.get(), Some(&2));
    }
}
