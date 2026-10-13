//! A value read on a worker thread, for a Settings pane that must not wait on the system. The pane starts the read when
//! it opens, shows a placeholder until `poll` takes the answer, and rebuilds then. A read started again replaces the
//! one still going; the value on hand stays on screen until the new one lands.
use std::sync::mpsc::{channel, Receiver, TryRecvError};

pub struct Loaded<T> {
    rx: Option<Receiver<T>>,
    value: Option<T>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Loaded { rx: None, value: None }
    }
}

impl<T: Send + 'static> Loaded<T> {
    /// Starts `read` on a worker thread. A read still going is superseded: its answer is dropped.
    pub fn start(&mut self, name: &str, read: impl FnOnce() -> T + Send + 'static) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        crate::bare::sys::spawn_named(name, move || {
            crate::gfx::affinity::background_thread();
            let _ = tx.send(read());
        });
    }

    /// True while a read is running.
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Takes the answer once it has arrived; true when it just did.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(v) => {
                self.value = Some(v);
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

    /// The latest answer, or None before the first read lands.
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// The latest answer for an edit the page made itself (a switch it just flipped), or None before the first read lands.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait<T: Send + 'static>(l: &mut Loaded<T>) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if l.poll() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
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
