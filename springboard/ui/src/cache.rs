//! A cost-limited cache (`NativeCache`): evicts the least recently used entries when the total cost passes the limit.
//! O(1) get and put (index-linked list over a slab; no unsafe).
use std::collections::HashMap;
use std::hash::Hash;

const NIL: usize = usize::MAX;
struct Node<K, V> {
    key: K,
    value: V,
    cost: usize,
    prev: usize,
    next: usize,
}

pub struct Cache<K, V> {
    map: HashMap<K, usize>,
    slab: Vec<Option<Node<K, V>>>,
    free: Vec<usize>,
    head: usize, // most recent
    tail: usize, // least recent
    cost: usize,
    limit: usize,
}

impl<K: Eq + Hash + Clone, V> Cache<K, V> {
    pub fn new(cost_limit: usize) -> Self {
        Cache { map: HashMap::new(), slab: Vec::new(), free: Vec::new(), head: NIL, tail: NIL, cost: 0, limit: cost_limit }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// Whether a value is held. Unlike `get`, this does not mark the entry as recently used.
    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }
    pub fn total_cost(&self) -> usize {
        self.cost
    }
    fn node(&mut self, i: usize) -> &mut Node<K, V> {
        self.slab[i].as_mut().expect("live node")
    }
    fn unlink(&mut self, i: usize) {
        let (p, n) = {
            let x = self.node(i);
            (x.prev, x.next)
        };
        if p != NIL { self.node(p).next = n } else { self.head = n }
        if n != NIL { self.node(n).prev = p } else { self.tail = p }
    }
    fn push_front(&mut self, i: usize) {
        let h = self.head;
        {
            let x = self.node(i);
            x.prev = NIL;
            x.next = h;
        }
        if h != NIL { self.node(h).prev = i } else { self.tail = i }
        self.head = i;
    }
    /// Reads a value and marks it most recently used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let i = *self.map.get(key)?;
        self.unlink(i);
        self.push_front(i);
        Some(&self.node(i).value)
    }
    /// Stores a value with its cost (bytes, pixels, any unit). An item costlier than the limit is not kept.
    pub fn put(&mut self, key: K, value: V, cost: usize) {
        self.remove(&key);
        if cost > self.limit {
            return;
        }
        let node = Node { key: key.clone(), value, cost, prev: NIL, next: NIL };
        let i = match self.free.pop() {
            Some(i) => {
                self.slab[i] = Some(node);
                i
            }
            None => {
                self.slab.push(Some(node));
                self.slab.len() - 1
            }
        };
        self.map.insert(key, i);
        self.push_front(i);
        self.cost += cost;
        while self.cost > self.limit && self.tail != NIL {
            let t = self.tail;
            let k = self.node(t).key.clone();
            self.remove(&k);
        }
    }
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let i = self.map.remove(key)?;
        self.unlink(i);
        let n = self.slab[i].take().expect("live node");
        self.free.push(i);
        self.cost -= n.cost;
        Some(n.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evicts_least_recently_used_by_cost() {
        let mut c = Cache::new(10);
        c.put("a", 1, 4);
        c.put("b", 2, 4);
        assert_eq!(c.get(&"a"), Some(&1)); // a is now newer than b
        c.put("c", 3, 4); // 12 > 10: b goes
        assert!(c.get(&"b").is_none());
        assert!(c.get(&"a").is_some() && c.get(&"c").is_some());
        assert_eq!(c.total_cost(), 8);
    }
    #[test]
    fn replacing_updates_cost_and_oversize_is_refused() {
        let mut c = Cache::new(10);
        c.put("a", 1, 6);
        c.put("a", 2, 3);
        assert_eq!((c.len(), c.total_cost(), c.get(&"a").copied()), (1, 3, Some(2)));
        c.put("big", 9, 11);
        assert!(c.get(&"big").is_none());
    }
    #[test]
    fn contains_key_does_not_refresh_recency() {
        let mut c = Cache::new(8);
        c.put("a", 1, 4);
        c.put("b", 2, 4);
        assert!(c.contains_key(&"a"));
        c.put("c", 3, 4); // "a" was not touched, so it is the oldest and goes
        assert!(!c.contains_key(&"a"));
        assert!(c.contains_key(&"b") && c.contains_key(&"c"));
    }
    #[test]
    fn slots_are_reused() {
        let mut c = Cache::new(2);
        for i in 0..100 {
            c.put(i, i, 1);
        }
        assert_eq!(c.len(), 2);
        assert!(c.slab.len() <= 3);
    }
}
