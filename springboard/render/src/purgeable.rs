//! `NativeCache` / `NativePurgeableData` parity: a cost-bounded LRU cache that gives memory back under pressure.
//!
//! Every [`Cache`] registers itself process-wide. The first one starts a single watcher thread that blocks in
//! `poll()` on a Linux PSI trigger (`/proc/pressure/memory`); when the kernel signals memory pressure every live
//! cache drops half its cost, oldest entries first. Without PSI (old kernel, no permission) no thread is started
//! and the caches are plain LRUs. Values are cloned out by `get`, so store `Arc<T>` for large values.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

struct Entry<V> {
    v: V,
    cost: usize,
    tick: u64,
}

struct Inner<K, V> {
    map: HashMap<K, Entry<V>>,
    /// tick -> key; the first entry is the least recently used
    order: BTreeMap<u64, K>,
    tick: u64,
    cost: usize,
    count_limit: usize,
    cost_limit: usize,
}

struct Shared<K, V>(Mutex<Inner<K, V>>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl<K: Hash + Eq + Clone, V> Inner<K, V> {
    fn drop_oldest(&mut self) -> bool {
        let Some((&t, _)) = self.order.iter().next() else { return false };
        if let Some(k) = self.order.remove(&t) {
            if let Some(e) = self.map.remove(&k) {
                self.cost -= e.cost;
            }
        }
        true
    }
    fn trim(&mut self) {
        while (self.count_limit > 0 && self.map.len() > self.count_limit) || (self.cost_limit > 0 && self.cost > self.cost_limit) {
            if !self.drop_oldest() {
                break;
            }
        }
    }
}

/// What the watcher calls on every registered cache.
trait Purge: Send + Sync {
    fn purge(&self, fraction: f64) -> usize;
}

impl<K: Hash + Eq + Clone + Send, V: Send> Purge for Shared<K, V> {
    fn purge(&self, fraction: f64) -> usize {
        let mut s = lock(&self.0);
        let before = s.cost;
        if fraction >= 1.0 {
            s.map.clear();
            s.order.clear();
            s.cost = 0;
        } else {
            let target = (before as f64 * (1.0 - fraction.max(0.0))) as usize;
            while s.cost > target && s.drop_oldest() {}
        }
        before - s.cost
    }
}

/// `NativeCache<K, V>`: `count_limit` and `total_cost_limit` (0 means unlimited), least-recently-used eviction.
pub struct Cache<K, V> {
    shared: Arc<Shared<K, V>>,
}

impl<K, V> Clone for Cache<K, V> {
    fn clone(&self) -> Self {
        Cache { shared: self.shared.clone() }
    }
}

impl<K: Hash + Eq + Clone + Send + 'static, V: Clone + Send + 'static> Cache<K, V> {
    /// A cache registered for memory-pressure purging. `count_limit` / `total_cost_limit`: 0 is unlimited.
    pub fn new(count_limit: usize, total_cost_limit: usize) -> Self {
        let shared = Arc::new(Shared(Mutex::new(Inner {
            map: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            cost: 0,
            count_limit,
            cost_limit: total_cost_limit,
        })));
        let dynamic: Arc<dyn Purge> = shared.clone();
        lock(registry()).push(Arc::downgrade(&dynamic));
        start_watcher();
        Cache { shared }
    }

    pub fn count_limit(&self) -> usize {
        lock(&self.shared.0).count_limit
    }
    pub fn set_count_limit(&self, n: usize) {
        let mut s = lock(&self.shared.0);
        s.count_limit = n;
        s.trim();
    }
    pub fn total_cost_limit(&self) -> usize {
        lock(&self.shared.0).cost_limit
    }
    pub fn set_total_cost_limit(&self, n: usize) {
        let mut s = lock(&self.shared.0);
        s.cost_limit = n;
        s.trim();
    }

    /// `setObject(_:forKey:cost:)`: replaces any value, then evicts the oldest until both limits hold (a value
    /// costing more than the whole limit is evicted at once, as in `NativeCache`).
    pub fn set(&self, k: K, v: V, cost: usize) {
        let mut s = lock(&self.shared.0);
        if let Some(old) = s.map.remove(&k) {
            s.order.remove(&old.tick);
            s.cost -= old.cost;
        }
        s.tick += 1;
        let tick = s.tick;
        s.order.insert(tick, k.clone());
        s.map.insert(k, Entry { v, cost, tick });
        s.cost += cost;
        s.trim();
    }

    /// `object(forKey:)`: a hit becomes the most recently used entry.
    pub fn get(&self, k: &K) -> Option<V> {
        let mut s = lock(&self.shared.0);
        s.tick += 1;
        let tick = s.tick;
        let e = s.map.get_mut(k)?;
        let old = std::mem::replace(&mut e.tick, tick);
        let v = e.v.clone();
        s.order.remove(&old);
        s.order.insert(tick, k.clone());
        Some(v)
    }

    pub fn remove(&self, k: &K) {
        let mut s = lock(&self.shared.0);
        if let Some(e) = s.map.remove(k) {
            s.order.remove(&e.tick);
            s.cost -= e.cost;
        }
    }

    /// `removeAllObjects()`.
    pub fn remove_all(&self) {
        self.shared.purge(1.0);
    }

    pub fn len(&self) -> usize {
        lock(&self.shared.0).map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Sum of the costs of the entries held.
    pub fn total_cost(&self) -> usize {
        lock(&self.shared.0).cost
    }

    /// Drops `fraction` (0..=1) of the held cost, oldest first; returns the cost freed. The memory-pressure
    /// watcher calls this with 0.5; callers may use it too.
    pub fn purge(&self, fraction: f64) -> usize {
        self.shared.purge(fraction)
    }
}

fn registry() -> &'static Mutex<Vec<Weak<dyn Purge>>> {
    static R: OnceLock<Mutex<Vec<Weak<dyn Purge>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

/// Purges `fraction` of every live cache (dead registrations are dropped). Returns the total cost freed.
pub fn purge_all(fraction: f64) -> usize {
    let live: Vec<Arc<dyn Purge>> = {
        let mut r = lock(registry());
        r.retain(|w| w.strong_count() > 0);
        r.iter().filter_map(Weak::upgrade).collect()
    };
    live.iter().map(|c| c.purge(fraction)).sum()
}

/// `DispatchSource.MemoryPressureEvent`: `normal` again after a quiet spell, `warning` when some tasks stalled on
/// memory, `critical` when all runnable tasks did (the two Linux PSI thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

type PressureListener = Box<dyn Fn(MemoryPressure) + Send + Sync>;

fn listeners() -> &'static Mutex<Vec<PressureListener>> {
    static L: OnceLock<Mutex<Vec<PressureListener>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Vec::new()))
}

/// Registers `f` for pressure events (called on the watcher thread; it lives for the process). Starts the one shared
/// PSI watcher if it is not running; the caches use the same watcher.
pub fn on_memory_pressure(f: impl Fn(MemoryPressure) + Send + Sync + 'static) {
    lock(listeners()).push(Box::new(f));
    start_watcher();
}

fn notify_pressure(level: MemoryPressure) {
    match level {
        MemoryPressure::Normal => {}
        MemoryPressure::Warning => {
            purge_all(0.5);
        }
        MemoryPressure::Critical => {
            purge_all(0.9);
        }
    }
    for f in lock(listeners()).iter() {
        f(level);
    }
}

#[cfg(target_os = "linux")]
static WATCH_ONCE: std::sync::Once = std::sync::Once::new();

/// The memory PSI triggers on one window of 2 s (unprivileged windows are multiples of 2 s):
/// warning: 150 ms of "some" stall; critical: 100 ms of "full" stall.
#[cfg(target_os = "linux")]
fn open_triggers() -> Option<(std::fs::File, std::fs::File)> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let open = |trigger: &[u8]| {
        let mut f = OpenOptions::new().read(true).write(true).open("/proc/pressure/memory").ok()?;
        f.write_all(trigger).ok()?;
        Some(f)
    };
    Some((open(b"some 150000 2000000\0")?, open(b"full 100000 2000000\0")?))
}

/// The pressure watcher as fds plus a handler, for a host that already blocks in `poll(2)` on its own thread
/// (the shell's sys-watch), instead of a thread of its own. `ready[i]` says whether `fds[i]` signalled (POLLPRI
/// or an error); the handler returns its next deadline in ms on the caller's clock, `now` (after an event, two
/// quiet windows, 4 s, mean the pressure is gone). None when the watcher already runs or PSI is missing.
#[cfg(target_os = "linux")]
pub fn hosted_watcher() -> Option<(Vec<i32>, Box<dyn FnMut(&[bool], f64) -> Option<f64> + Send>)> {
    use std::os::fd::AsRawFd;
    let mut out = None;
    WATCH_ONCE.call_once(|| {
        let Some((warn, crit)) = open_triggers() else { return };
        let fds = vec![warn.as_raw_fd(), crit.as_raw_fd()];
        let mut level = MemoryPressure::Normal;
        let run = move |ready: &[bool], now: f64| -> Option<f64> {
            let _keep = (&warn, &crit);
            let next = if ready[1] {
                MemoryPressure::Critical
            } else if ready[0] {
                MemoryPressure::Warning
            } else {
                // the quiet deadline passed
                MemoryPressure::Normal
            };
            if next != MemoryPressure::Normal || level != MemoryPressure::Normal {
                level = next;
                notify_pressure(level);
            }
            (level != MemoryPressure::Normal).then_some(now + 4000.0)
        };
        out = Some((fds, Box::new(run) as Box<dyn FnMut(&[bool], f64) -> Option<f64> + Send>));
    });
    out
}

#[cfg(target_os = "linux")]
fn start_watcher() {
    use std::os::fd::AsRawFd;
    WATCH_ONCE.call_once(|| {
        #[repr(C)]
        struct PollFd {
            fd: i32,
            events: i16,
            revents: i16,
        }
        extern "C" {
            fn poll(fds: *mut PollFd, n: u64, timeout: i32) -> i32;
        }
        const POLLPRI: i16 = 0x2;
        const POLLERR: i16 = 0x8;
        let Some((warn, crit)) = open_triggers() else { return };
        let _ = std::thread::Builder::new().name("memory-pressure".into()).spawn(move || {
            let mut fds = [PollFd { fd: warn.as_raw_fd(), events: POLLPRI, revents: 0 }, PollFd { fd: crit.as_raw_fd(), events: POLLPRI, revents: 0 }];
            let mut level = MemoryPressure::Normal;
            loop {
                // After an event, two quiet windows (4 s) mean the pressure is gone.
                let timeout = if level == MemoryPressure::Normal { -1 } else { 4000 };
                // SAFETY: two valid pollfds on descriptors the closure keeps open; blocks until a trigger, an error or the timeout.
                let n = unsafe { poll(fds.as_mut_ptr(), 2, timeout) };
                if n < 0 {
                    continue;
                }
                if n == 0 {
                    level = MemoryPressure::Normal;
                    notify_pressure(level);
                    continue;
                }
                if fds.iter().any(|p| p.revents & POLLERR != 0) {
                    return;
                }
                level = if fds[1].revents & POLLPRI != 0 {
                    MemoryPressure::Critical
                } else if fds[0].revents & POLLPRI != 0 {
                    MemoryPressure::Warning
                } else {
                    continue;
                };
                notify_pressure(level);
            }
        });
    });
}

#[cfg(not(target_os = "linux"))]
fn start_watcher() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_limit_evicts_least_recently_used() {
        let c: Cache<u32, u32> = Cache::new(3, 0);
        for i in 0..3 {
            c.set(i, i, 1);
        }
        assert_eq!(c.get(&0), Some(0)); // 0 is now newest; 1 is oldest
        c.set(3, 3, 1);
        assert_eq!(c.len(), 3);
        assert_eq!(c.get(&1), None);
        assert_eq!(c.get(&0), Some(0));
    }

    #[test]
    fn cost_limit_and_replace_keep_the_total_exact() {
        let c: Cache<&'static str, u32> = Cache::new(0, 10);
        c.set("a", 1, 4);
        c.set("b", 2, 4);
        c.set("a", 3, 2); // replace: 4 + 2
        assert_eq!(c.total_cost(), 6);
        c.set("c", 4, 6); // 12 > 10: oldest ("b") goes
        assert_eq!(c.get(&"b"), None);
        assert_eq!(c.total_cost(), 8);
        c.set("huge", 5, 11);
        assert_eq!(c.get(&"huge"), None, "a value above the whole limit is not kept");
        assert!(c.total_cost() <= 10);
    }

    #[test]
    fn remove_and_remove_all() {
        let c: Cache<u32, u32> = Cache::new(0, 0);
        c.set(1, 1, 5);
        c.set(2, 2, 5);
        c.remove(&1);
        assert_eq!((c.len(), c.total_cost()), (1, 5));
        c.remove_all();
        assert!(c.is_empty());
        assert_eq!(c.total_cost(), 0);
    }

    #[test]
    fn purge_drops_the_oldest_fraction_of_the_cost() {
        let c: Cache<u32, u32> = Cache::new(0, 0);
        for i in 0..10 {
            c.set(i, i, 10);
        }
        assert_eq!(c.purge(0.5), 50);
        assert_eq!(c.total_cost(), 50);
        assert_eq!(c.get(&0), None);
        assert_eq!(c.get(&9), Some(9));
        assert_eq!(c.purge(1.0), 50);
        assert!(c.is_empty());
        assert_eq!(c.purge(0.5), 0);
    }

    #[test]
    fn purge_all_reaches_registered_caches_and_skips_dropped_ones() {
        let c: Cache<u32, u32> = Cache::new(0, 0);
        c.set(1, 1, 8);
        drop(Cache::<u32, u32>::new(0, 0));
        assert!(purge_all(1.0) >= 8);
        assert!(c.is_empty());
    }
}
