//! Buffer ID registry: gives a shared buffer a numeric ID and finds the buffer again from that ID.
//! Pure model inside one process. IDs start at 1 and rise by one. A removed ID is never handed out again,
//! so a stale ID cannot name a newer buffer. When the ID space runs out, `register` returns None.
use std::collections::HashMap;

pub type SurfaceId = u32;

pub struct SurfaceRegistry<T> {
    next: SurfaceId,
    items: HashMap<SurfaceId, T>,
}

impl<T> Default for SurfaceRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SurfaceRegistry<T> {
    pub fn new() -> Self {
        SurfaceRegistry { next: 1, items: HashMap::new() }
    }

    /// Stores `item` and returns its new ID, or None when no ID is left (the item is dropped).
    pub fn register(&mut self, item: T) -> Option<SurfaceId> {
        let id = self.next;
        let following = self.next.checked_add(1)?;
        self.next = following;
        self.items.insert(id, item);
        Some(id)
    }

    pub fn lookup(&self, id: SurfaceId) -> Option<&T> {
        self.items.get(&id)
    }

    pub fn lookup_mut(&mut self, id: SurfaceId) -> Option<&mut T> {
        self.items.get_mut(&id)
    }

    /// Unregisters `id` and returns its item. The ID is not reused.
    pub fn remove(&mut self, id: SurfaceId) -> Option<T> {
        self.items.remove(&id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_start_at_one_and_rise_by_one() {
        let mut r: SurfaceRegistry<&str> = SurfaceRegistry::new();
        assert_eq!(r.register("frame"), Some(1));
        assert_eq!(r.register("cursor"), Some(2));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn lookup_finds_the_registered_item_by_its_id() {
        let mut r: SurfaceRegistry<&str> = SurfaceRegistry::new();
        let id = r.register("frame").unwrap();
        assert_eq!(r.lookup(id), Some(&"frame"));
        assert_eq!(r.lookup(id + 1), None);
    }

    #[test]
    fn lookup_mut_changes_the_stored_item() {
        let mut r: SurfaceRegistry<u32> = SurfaceRegistry::new();
        let id = r.register(10).unwrap();
        *r.lookup_mut(id).unwrap() += 5;
        assert_eq!(r.lookup(id), Some(&15));
    }

    #[test]
    fn removed_ids_are_not_reused() {
        let mut r: SurfaceRegistry<&str> = SurfaceRegistry::new();
        let first = r.register("a").unwrap();
        assert_eq!(r.remove(first), Some("a"));
        assert_eq!(r.lookup(first), None);
        assert_eq!(r.remove(first), None);
        assert_eq!(r.register("b"), Some(2));
        assert!(!r.is_empty());
    }

    #[test]
    fn register_returns_none_when_the_id_space_is_used_up() {
        let mut r: SurfaceRegistry<&str> = SurfaceRegistry { next: u32::MAX, items: HashMap::new() };
        assert_eq!(r.register("last"), None);
        assert!(r.is_empty());
    }
}
