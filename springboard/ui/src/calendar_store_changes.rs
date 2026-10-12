//! Change notices for a shared calendar store. The store calls [`StoreChangeHub::post`] after every save. Each
//! observer remembers the last generation it handled, so it learns whether anything changed since it last looked
//! and can refetch once, however many saves happened in between. A new observer starts up to date and does not
//! replay earlier changes.
//!
//! This is the pure model of the official change notification (a notice posted when the calendar database changes,
//! including changes made by another app). It has no delivery thread and no callback. The caller polls or
//! acknowledges, which keeps the model testable and free of shared state.

/// Identifies one observer of a [`StoreChangeHub`]. Identifiers are not reused after an observer leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObserverId(u64);

/// Counts changes to a store and tracks what each observer has handled.
#[derive(Clone, Debug, Default)]
pub struct StoreChangeHub {
    generation: u64,
    next_id: u64,
    /// Each observer with the generation it last handled.
    observers: Vec<(ObserverId, u64)>,
}

impl StoreChangeHub {
    /// A hub at generation zero with no observers.
    pub fn new() -> Self {
        StoreChangeHub::default()
    }

    /// The current generation. It rises by one for every [`post`](Self::post).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Records one change to the store and returns the new generation.
    pub fn post(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    /// Adds an observer that starts up to date. Changes posted before this call are not reported to it.
    pub fn subscribe(&mut self) -> ObserverId {
        self.next_id += 1;
        let id = ObserverId(self.next_id);
        self.observers.push((id, self.generation));
        id
    }

    /// Removes an observer. Returns `false` when it was not subscribed.
    pub fn unsubscribe(&mut self, id: ObserverId) -> bool {
        let before = self.observers.len();
        self.observers.retain(|(o, _)| *o != id);
        self.observers.len() != before
    }

    /// The number of subscribed observers.
    pub fn len(&self) -> usize {
        self.observers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.observers.is_empty()
    }

    /// How many changes the observer has not handled yet. `None` when the observer is not subscribed.
    pub fn missed(&self, id: ObserverId) -> Option<u64> {
        self.observers
            .iter()
            .find(|(o, _)| *o == id)
            .map(|(_, seen)| self.generation - seen)
    }

    /// Marks everything up to the current generation as handled. Returns whether the observer had pending changes,
    /// which tells the caller whether a refetch is due. `None` when the observer is not subscribed.
    pub fn acknowledge(&mut self, id: ObserverId) -> Option<bool> {
        let generation = self.generation;
        let slot = self.observers.iter_mut().find(|(o, _)| *o == id)?;
        let pending = slot.1 < generation;
        slot.1 = generation;
        Some(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_hub_has_no_observers_and_generation_zero() {
        let hub = StoreChangeHub::new();
        assert_eq!(hub.generation(), 0);
        assert!(hub.is_empty());
        assert_eq!(hub.missed(ObserverId(1)), None);
    }

    #[test]
    fn posts_are_counted_and_a_pending_observer_sees_every_one() {
        let mut hub = StoreChangeHub::new();
        let watcher = hub.subscribe();
        assert_eq!(hub.missed(watcher), Some(0));
        assert_eq!(hub.post(), 1);
        hub.post();
        hub.post();
        assert_eq!(hub.generation(), 3);
        assert_eq!(hub.missed(watcher), Some(3));
    }

    #[test]
    fn acknowledging_coalesces_many_changes_into_one_refetch() {
        let mut hub = StoreChangeHub::new();
        let watcher = hub.subscribe();
        hub.post();
        hub.post();
        assert_eq!(hub.acknowledge(watcher), Some(true));
        assert_eq!(hub.missed(watcher), Some(0));
        assert_eq!(hub.acknowledge(watcher), Some(false), "nothing new since the last acknowledge");
    }

    #[test]
    fn observers_track_their_own_position() {
        let mut hub = StoreChangeHub::new();
        let first = hub.subscribe();
        hub.post();
        let second = hub.subscribe();
        hub.post();
        assert_eq!(hub.missed(first), Some(2));
        assert_eq!(hub.missed(second), Some(1), "the second observer subscribed after the first change");
        assert_eq!(hub.acknowledge(first), Some(true));
        assert_eq!(hub.missed(second), Some(1), "handling one observer leaves the others pending");
    }

    #[test]
    fn a_late_observer_does_not_replay_earlier_changes() {
        let mut hub = StoreChangeHub::new();
        hub.post();
        hub.post();
        let late = hub.subscribe();
        assert_eq!(hub.acknowledge(late), Some(false));
    }

    #[test]
    fn unsubscribed_observers_are_forgotten_and_ids_are_not_reused() {
        let mut hub = StoreChangeHub::new();
        let gone = hub.subscribe();
        assert!(hub.unsubscribe(gone));
        assert!(!hub.unsubscribe(gone));
        assert_eq!(hub.missed(gone), None);
        assert_eq!(hub.acknowledge(gone), None);
        let next = hub.subscribe();
        assert_ne!(gone, next);
        assert_eq!(hub.len(), 1);
    }
}
