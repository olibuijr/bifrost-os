//! A page's live view (what its rows read from files or services) kept between builds, so that building the page
//! never waits on IO. `refresh` loads the view on a worker thread; the page keeps showing the last view it had until
//! the new one lands, and `poll` (called from the page owner's `tick`) takes it.
use std::sync::mpsc::{channel, Receiver, TryRecvError};

#[derive(Default)]
pub struct ViewCache<T> {
    value: T,
    rx: Option<Receiver<T>>,
}

impl<T: Send + 'static> ViewCache<T> {
    /// The view as it was last loaded (the default until the first load lands).
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Edits the held view in place, for a change the page already knows the outcome of.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// True while a load is on its way.
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Loads the view again on a worker thread; a load still going is superseded.
    pub fn refresh(&mut self, load: fn() -> T) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        crate::bare::sys::spawn_named("view-cache", move || {
            crate::gfx::affinity::background_thread();
            let _ = tx.send(load());
        });
    }

    /// Takes a finished load; true when the view changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(v) => {
                self.value = v;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_cache_holds_the_default_and_is_idle() {
        let c: ViewCache<Vec<u8>> = ViewCache::default();
        assert!(c.get().is_empty());
        assert!(!c.busy());
    }

    #[test]
    fn a_refresh_lands_through_poll() {
        let mut c: ViewCache<u32> = ViewCache::default();
        c.refresh(|| 7);
        let mut landed = false;
        for _ in 0..1000 {
            if c.poll() {
                landed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(landed);
        assert_eq!(*c.get(), 7);
        assert!(!c.busy());
        assert!(!c.poll());
    }

    #[test]
    fn an_edit_in_place_is_kept() {
        let mut c: ViewCache<u32> = ViewCache::default();
        *c.get_mut() = 3;
        assert_eq!(*c.get(), 3);
    }
}
