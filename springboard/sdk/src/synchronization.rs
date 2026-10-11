//! Synchronization parity: `Mutex` with closure-scoped locking, `Atomic` integers with an explicit memory ordering
//! per operation, `AtomicLazyReference` and `atomic_memory_fence`.
//!
//! Pure `std`: the orderings map onto `std::sync::atomic::Ordering` and nothing here uses `unsafe`. `WordPair`
//! is not provided, because a lock-free double-word atomic needs `unsafe` or a platform instruction (see
//! `docs/parity/synchronization.md`). Apps reach these types as `bifrost_app::synchronization`.

use std::sync::atomic::{self, AtomicI32, AtomicI64, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, TryLockError};

/// Memory ordering of an atomic load (`AtomicLoadOrdering`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadOrdering {
    Relaxed,
    Acquiring,
    SequentiallyConsistent,
}

/// Memory ordering of an atomic store (`AtomicStoreOrdering`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoreOrdering {
    Relaxed,
    Releasing,
    SequentiallyConsistent,
}

/// Memory ordering of an atomic read-modify-write (`AtomicUpdateOrdering`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpdateOrdering {
    Relaxed,
    Acquiring,
    Releasing,
    AcquiringAndReleasing,
    SequentiallyConsistent,
}

impl LoadOrdering {
    fn to_std(self) -> Ordering {
        match self {
            Self::Relaxed => Ordering::Relaxed,
            Self::Acquiring => Ordering::Acquire,
            Self::SequentiallyConsistent => Ordering::SeqCst,
        }
    }
}

impl StoreOrdering {
    fn to_std(self) -> Ordering {
        match self {
            Self::Relaxed => Ordering::Relaxed,
            Self::Releasing => Ordering::Release,
            Self::SequentiallyConsistent => Ordering::SeqCst,
        }
    }
}

impl UpdateOrdering {
    fn to_std(self) -> Ordering {
        match self {
            Self::Relaxed => Ordering::Relaxed,
            Self::Acquiring => Ordering::Acquire,
            Self::Releasing => Ordering::Release,
            Self::AcquiringAndReleasing => Ordering::AcqRel,
            Self::SequentiallyConsistent => Ordering::SeqCst,
        }
    }
}

/// Orders memory accesses around the call without touching a value (`atomicMemoryFence(ordering:)`).
/// `Relaxed` is a no-op: std's fence rejects it, and a relaxed fence orders nothing.
pub fn atomic_memory_fence(ordering: UpdateOrdering) {
    if ordering != UpdateOrdering::Relaxed {
        atomic::fence(ordering.to_std());
    }
}

/// A value protected by mutual exclusion. The lock is held only while a closure runs, so the guard cannot escape
/// (`Mutex.withLock`). A lock poisoned by a panic is recovered, as the rest of the SDK does.
pub struct Mutex<T> {
    inner: StdMutex<T>,
}

impl<T> Mutex<T> {
    pub fn new(initial: T) -> Self {
        Self { inner: StdMutex::new(initial) }
    }

    /// Waits for the lock, runs `body` on the protected value and returns its result (`withLock(_:)`).
    pub fn with_lock<R>(&self, body: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        body(&mut *guard)
    }

    /// Runs `body` only when the lock is free right now; `None` when another holder has it
    /// (`withLockIfAvailable(_:)`). Never blocks.
    pub fn with_lock_if_available<R>(&self, body: impl FnOnce(&mut T) -> R) -> Option<R> {
        match self.inner.try_lock() {
            Ok(mut guard) => Some(body(&mut *guard)),
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut guard = poisoned.into_inner();
                Some(body(&mut *guard))
            }
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// An integer type `Atomic` can hold. Implemented for the fixed-size std atomic integers only.
pub trait AtomicInteger: Copy + Eq + Send + Sync + 'static {
    type Cell: Send + Sync;
    fn new_cell(v: Self) -> Self::Cell;
    fn load_cell(c: &Self::Cell, o: Ordering) -> Self;
    fn store_cell(c: &Self::Cell, v: Self, o: Ordering);
    fn swap_cell(c: &Self::Cell, v: Self, o: Ordering) -> Self;
    fn cas_cell(c: &Self::Cell, expected: Self, desired: Self, success: Ordering, failure: Ordering) -> Result<Self, Self>;
    fn fetch_add_cell(c: &Self::Cell, v: Self, o: Ordering) -> Self;
    fn fetch_sub_cell(c: &Self::Cell, v: Self, o: Ordering) -> Self;
    fn fetch_and_cell(c: &Self::Cell, v: Self, o: Ordering) -> Self;
    fn fetch_or_cell(c: &Self::Cell, v: Self, o: Ordering) -> Self;
    fn fetch_xor_cell(c: &Self::Cell, v: Self, o: Ordering) -> Self;
    /// The value `fetch_add` leaves behind (wrapping), so `add` can report the new value too.
    fn add_wrapping(self, rhs: Self) -> Self;
    fn sub_wrapping(self, rhs: Self) -> Self;
    fn and_bits(self, rhs: Self) -> Self;
    fn or_bits(self, rhs: Self) -> Self;
    fn xor_bits(self, rhs: Self) -> Self;
}

macro_rules! atomic_integer {
    ($int:ty, $cell:ty) => {
        impl AtomicInteger for $int {
            type Cell = $cell;
            fn new_cell(v: Self) -> $cell {
                <$cell>::new(v)
            }
            fn load_cell(c: &$cell, o: Ordering) -> Self {
                c.load(o)
            }
            fn store_cell(c: &$cell, v: Self, o: Ordering) {
                c.store(v, o)
            }
            fn swap_cell(c: &$cell, v: Self, o: Ordering) -> Self {
                c.swap(v, o)
            }
            fn cas_cell(c: &$cell, expected: Self, desired: Self, success: Ordering, failure: Ordering) -> Result<Self, Self> {
                c.compare_exchange(expected, desired, success, failure)
            }
            fn fetch_add_cell(c: &$cell, v: Self, o: Ordering) -> Self {
                c.fetch_add(v, o)
            }
            fn fetch_sub_cell(c: &$cell, v: Self, o: Ordering) -> Self {
                c.fetch_sub(v, o)
            }
            fn fetch_and_cell(c: &$cell, v: Self, o: Ordering) -> Self {
                c.fetch_and(v, o)
            }
            fn fetch_or_cell(c: &$cell, v: Self, o: Ordering) -> Self {
                c.fetch_or(v, o)
            }
            fn fetch_xor_cell(c: &$cell, v: Self, o: Ordering) -> Self {
                c.fetch_xor(v, o)
            }
            fn add_wrapping(self, rhs: Self) -> Self {
                self.wrapping_add(rhs)
            }
            fn sub_wrapping(self, rhs: Self) -> Self {
                self.wrapping_sub(rhs)
            }
            fn and_bits(self, rhs: Self) -> Self {
                self & rhs
            }
            fn or_bits(self, rhs: Self) -> Self {
                self | rhs
            }
            fn xor_bits(self, rhs: Self) -> Self {
                self ^ rhs
            }
        }
    };
}

atomic_integer!(u32, AtomicU32);
atomic_integer!(u64, AtomicU64);
atomic_integer!(usize, AtomicUsize);
atomic_integer!(i32, AtomicI32);
atomic_integer!(i64, AtomicI64);
atomic_integer!(isize, AtomicIsize);

/// An integer that can be read and changed from several threads without a lock (`Atomic`). Every operation names its
/// memory ordering. Read-modify-write operations return the value before and after, as the framework does.
pub struct Atomic<T: AtomicInteger> {
    cell: T::Cell,
}

impl<T: AtomicInteger> Atomic<T> {
    pub fn new(initial: T) -> Self {
        Self { cell: T::new_cell(initial) }
    }

    pub fn load(&self, ordering: LoadOrdering) -> T {
        T::load_cell(&self.cell, ordering.to_std())
    }

    pub fn store(&self, value: T, ordering: StoreOrdering) {
        T::store_cell(&self.cell, value, ordering.to_std())
    }

    /// Stores `value` and returns the value it replaced (`exchange(_:ordering:)`).
    pub fn exchange(&self, value: T, ordering: UpdateOrdering) -> T {
        T::swap_cell(&self.cell, value, ordering.to_std())
    }

    /// Stores `desired` only when the value is `expected`. Returns whether it was stored and the value found
    /// (`compareExchange(expected:desired:ordering:)`). The failure ordering follows from `ordering`.
    pub fn compare_exchange(&self, expected: T, desired: T, ordering: UpdateOrdering) -> (bool, T) {
        let failure = match ordering {
            UpdateOrdering::Relaxed => LoadOrdering::Relaxed,
            UpdateOrdering::SequentiallyConsistent => LoadOrdering::SequentiallyConsistent,
            _ => LoadOrdering::Acquiring,
        };
        self.compare_exchange_with(expected, desired, ordering, failure)
    }

    /// Compare-and-exchange with separate success and failure orderings
    /// (`compareExchange(expected:desired:successOrdering:failureOrdering:)`).
    pub fn compare_exchange_with(&self, expected: T, desired: T, success: UpdateOrdering, failure: LoadOrdering) -> (bool, T) {
        match T::cas_cell(&self.cell, expected, desired, success.to_std(), failure.to_std()) {
            Ok(found) => (true, found),
            Err(found) => (false, found),
        }
    }

    /// Adds `value` with wrapping arithmetic; returns `(old, new)` (`add(_:ordering:)`).
    pub fn add(&self, value: T, ordering: UpdateOrdering) -> (T, T) {
        let old = T::fetch_add_cell(&self.cell, value, ordering.to_std());
        (old, old.add_wrapping(value))
    }

    /// Subtracts `value` with wrapping arithmetic; returns `(old, new)` (`subtract(_:ordering:)`).
    pub fn subtract(&self, value: T, ordering: UpdateOrdering) -> (T, T) {
        let old = T::fetch_sub_cell(&self.cell, value, ordering.to_std());
        (old, old.sub_wrapping(value))
    }

    /// Bitwise AND with `value`; returns `(old, new)` (`bitwiseAnd(_:ordering:)`).
    pub fn bitwise_and(&self, value: T, ordering: UpdateOrdering) -> (T, T) {
        let old = T::fetch_and_cell(&self.cell, value, ordering.to_std());
        (old, old.and_bits(value))
    }

    /// Bitwise OR with `value`; returns `(old, new)` (`bitwiseOr(_:ordering:)`).
    pub fn bitwise_or(&self, value: T, ordering: UpdateOrdering) -> (T, T) {
        let old = T::fetch_or_cell(&self.cell, value, ordering.to_std());
        (old, old.or_bits(value))
    }

    /// Bitwise XOR with `value`; returns `(old, new)` (`bitwiseXor(_:ordering:)`).
    pub fn bitwise_xor(&self, value: T, ordering: UpdateOrdering) -> (T, T) {
        let old = T::fetch_xor_cell(&self.cell, value, ordering.to_std());
        (old, old.xor_bits(value))
    }
}

/// A shared reference that is set at most once, lazily (`AtomicLazyReference`). Readers see either nil or the
/// stored reference; the first `store_if_nil` wins.
pub struct AtomicLazyReference<T> {
    cell: OnceLock<Arc<T>>,
}

impl<T> AtomicLazyReference<T> {
    pub fn new() -> Self {
        Self { cell: OnceLock::new() }
    }

    /// The stored reference, or `None` while it is nil (`load()`).
    pub fn load(&self) -> Option<Arc<T>> {
        self.cell.get().cloned()
    }

    /// Stores `value` when the reference is still nil and returns the stored reference. When it is already set,
    /// `value` is dropped and the existing reference is returned (`storeIfNil(_:)`).
    pub fn store_if_nil(&self, value: Arc<T>) -> Arc<T> {
        Arc::clone(self.cell.get_or_init(|| value))
    }
}

impl<T> Default for AtomicLazyReference<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn mutex_with_lock_mutates_and_returns_the_result() {
        let m = Mutex::new(vec![1]);
        let len = m.with_lock(|v| {
            v.push(2);
            v.len()
        });
        assert_eq!(len, 2);
        assert_eq!(m.with_lock(|v| v.clone()), vec![1, 2]);
    }

    #[test]
    fn with_lock_if_available_is_none_while_the_lock_is_held() {
        let m = Mutex::new(0);
        let seen_inside = m.with_lock(|v| {
            *v = 5;
            m.with_lock_if_available(|v| *v)
        });
        assert_eq!(seen_inside, None);
        assert_eq!(m.with_lock_if_available(|v| *v), Some(5));
    }

    #[test]
    fn mutex_recovers_a_lock_poisoned_by_a_panic() {
        let m = Arc::new(Mutex::new(7));
        let m2 = Arc::clone(&m);
        let joined = std::thread::spawn(move || m2.with_lock(|_| -> i32 { panic!("expected test panic") })).join();
        assert!(joined.is_err());
        assert_eq!(m.with_lock(|v| *v), 7);
        assert_eq!(m.with_lock_if_available(|v| *v), Some(7));
    }

    #[test]
    fn mutex_serialises_increments_from_threads() {
        let m = Mutex::new(0usize);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..500 {
                        m.with_lock(|v| *v += 1);
                    }
                });
            }
        });
        assert_eq!(m.with_lock(|v| *v), 2000);
    }

    #[test]
    fn atomic_add_wraps_and_reports_old_and_new() {
        let a = Atomic::new(u32::MAX);
        assert_eq!(a.add(2, UpdateOrdering::SequentiallyConsistent), (u32::MAX, 1));
        assert_eq!(a.load(LoadOrdering::Acquiring), 1);
        assert_eq!(a.subtract(3, UpdateOrdering::Relaxed), (1, u32::MAX - 1));
    }

    #[test]
    fn atomic_bitwise_operations_report_old_and_new() {
        let a = Atomic::new(0b1100u64);
        assert_eq!(a.bitwise_and(0b0100, UpdateOrdering::Relaxed), (0b1100, 0b0100));
        assert_eq!(a.bitwise_or(0b0011, UpdateOrdering::AcquiringAndReleasing), (0b0100, 0b0111));
        assert_eq!(a.bitwise_xor(0b0110, UpdateOrdering::Releasing), (0b0111, 0b0001));
        assert_eq!(a.load(LoadOrdering::SequentiallyConsistent), 0b0001);
    }

    #[test]
    fn atomic_exchange_and_compare_exchange_follow_the_stored_value() {
        let a = Atomic::new(7i64);
        assert_eq!(a.exchange(9, UpdateOrdering::Acquiring), 7);
        assert_eq!(a.compare_exchange(9, 11, UpdateOrdering::AcquiringAndReleasing), (true, 9));
        assert_eq!(a.compare_exchange(9, 13, UpdateOrdering::SequentiallyConsistent), (false, 11));
        assert_eq!(a.compare_exchange_with(11, 12, UpdateOrdering::Releasing, LoadOrdering::Acquiring), (true, 11));
        a.store(-3, StoreOrdering::Releasing);
        assert_eq!(a.load(LoadOrdering::Relaxed), -3);
    }

    #[test]
    fn atomic_counts_correctly_across_threads() {
        let counter = Atomic::new(0usize);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..1000 {
                        counter.add(1, UpdateOrdering::Relaxed);
                    }
                });
            }
        });
        assert_eq!(counter.load(LoadOrdering::SequentiallyConsistent), 4000);
    }

    #[test]
    fn memory_fence_accepts_every_ordering() {
        for o in [
            UpdateOrdering::Relaxed,
            UpdateOrdering::Acquiring,
            UpdateOrdering::Releasing,
            UpdateOrdering::AcquiringAndReleasing,
            UpdateOrdering::SequentiallyConsistent,
        ] {
            atomic_memory_fence(o);
        }
    }

    #[test]
    fn lazy_reference_keeps_the_first_stored_value() {
        let r: AtomicLazyReference<String> = AtomicLazyReference::new();
        assert!(r.load().is_none());
        let first = r.store_if_nil(Arc::new("first".to_string()));
        let second = r.store_if_nil(Arc::new("second".to_string()));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second.as_str(), "first");
        assert_eq!(r.load().unwrap().as_str(), "first");
    }
}
