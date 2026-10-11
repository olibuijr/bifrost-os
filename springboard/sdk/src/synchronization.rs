//! Synchronization parity (`Atomic`, `Mutex`, `atomicMemoryFence`); see `docs/bifrost-ui.md`, "Synchronization".
//!
//! Std atomics and mutexes do the work. The wrappers keep each operation to the orderings that are valid for it, as
//! the framework's ordering types do, and `Mutex` ignores poisoning: a panic inside `with_lock` does not make later
//! locks fail. Not covered yet: `AtomicLazyReference`, `WordPair` and the optional-representable atomics.

use std::sync::atomic::{
    self, AtomicBool, AtomicI32, AtomicI64, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};
use std::sync::{Mutex as StdMutex, TryLockError};

/// `AtomicLoadOrdering`: the orderings a load may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadOrdering {
    Relaxed,
    Acquiring,
    SequentiallyConsistent,
}

/// `AtomicStoreOrdering`: the orderings a store may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOrdering {
    Relaxed,
    Releasing,
    SequentiallyConsistent,
}

/// `AtomicUpdateOrdering`: the orderings a read-modify-write operation may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    /// The failure ordering of a compare-exchange that succeeds with `self`. Std rejects Release and AcqRel there.
    fn failure(self) -> Ordering {
        match self {
            Self::Acquiring | Self::AcquiringAndReleasing => Ordering::Acquire,
            Self::SequentiallyConsistent => Ordering::SeqCst,
            Self::Relaxed | Self::Releasing => Ordering::Relaxed,
        }
    }
}

/// `AtomicRepresentable`: the value types [`Atomic`] supports (`bool` and the primitive integers used by apps).
pub trait AtomicRepresentable: Copy + Send + Sync + 'static {
    #[doc(hidden)]
    type Storage: Send + Sync;
    #[doc(hidden)]
    fn new_storage(value: Self) -> Self::Storage;
    #[doc(hidden)]
    fn load_from(storage: &Self::Storage, ordering: Ordering) -> Self;
    #[doc(hidden)]
    fn store_to(storage: &Self::Storage, value: Self, ordering: Ordering);
    #[doc(hidden)]
    fn swap_in(storage: &Self::Storage, value: Self, ordering: Ordering) -> Self;
    #[doc(hidden)]
    fn compare_in(storage: &Self::Storage, expected: Self, desired: Self, success: Ordering, failure: Ordering) -> Result<Self, Self>;
}

macro_rules! atomic_representable {
    ($($value:ty => $storage:ty),* $(,)?) => {$(
        impl AtomicRepresentable for $value {
            type Storage = $storage;
            fn new_storage(value: Self) -> $storage {
                <$storage>::new(value)
            }
            fn load_from(storage: &$storage, ordering: Ordering) -> Self {
                storage.load(ordering)
            }
            fn store_to(storage: &$storage, value: Self, ordering: Ordering) {
                storage.store(value, ordering)
            }
            fn swap_in(storage: &$storage, value: Self, ordering: Ordering) -> Self {
                storage.swap(value, ordering)
            }
            fn compare_in(storage: &$storage, expected: Self, desired: Self, success: Ordering, failure: Ordering) -> Result<Self, Self> {
                storage.compare_exchange(expected, desired, success, failure)
            }
        }
    )*};
}

atomic_representable!(
    bool => AtomicBool,
    i32 => AtomicI32,
    i64 => AtomicI64,
    isize => AtomicIsize,
    u32 => AtomicU32,
    u64 => AtomicU64,
    usize => AtomicUsize,
);

/// `Atomic<Value>`: a value that several threads read and write without a lock. Each operation takes the ordering
/// that applies to it, so a load cannot ask for a release ordering.
pub struct Atomic<T: AtomicRepresentable> {
    storage: T::Storage,
}

impl<T: AtomicRepresentable> Atomic<T> {
    pub fn new(value: T) -> Self {
        Self { storage: T::new_storage(value) }
    }

    pub fn load(&self, ordering: LoadOrdering) -> T {
        T::load_from(&self.storage, ordering.to_std())
    }

    pub fn store(&self, value: T, ordering: StoreOrdering) {
        T::store_to(&self.storage, value, ordering.to_std())
    }

    /// Replaces the value and returns the old one.
    pub fn exchange(&self, value: T, ordering: UpdateOrdering) -> T {
        T::swap_in(&self.storage, value, ordering.to_std())
    }

    /// Replaces the value only when it equals `expected`. Returns `Ok(expected)` on success and `Err(current)` otherwise.
    pub fn compare_exchange(&self, expected: T, desired: T, ordering: UpdateOrdering) -> Result<T, T> {
        T::compare_in(&self.storage, expected, desired, ordering.to_std(), ordering.failure())
    }
}

macro_rules! atomic_integer {
    ($($value:ty),* $(,)?) => {$(
        impl Atomic<$value> {
            /// Adds with wrapping arithmetic and returns `(old, new)`.
            pub fn add(&self, operand: $value, ordering: UpdateOrdering) -> ($value, $value) {
                let old = self.storage.fetch_add(operand, ordering.to_std());
                (old, old.wrapping_add(operand))
            }

            /// Subtracts with wrapping arithmetic and returns `(old, new)`.
            pub fn subtract(&self, operand: $value, ordering: UpdateOrdering) -> ($value, $value) {
                let old = self.storage.fetch_sub(operand, ordering.to_std());
                (old, old.wrapping_sub(operand))
            }

            /// Bitwise AND and returns `(old, new)`.
            pub fn bitwise_and(&self, operand: $value, ordering: UpdateOrdering) -> ($value, $value) {
                let old = self.storage.fetch_and(operand, ordering.to_std());
                (old, old & operand)
            }

            /// Bitwise OR and returns `(old, new)`.
            pub fn bitwise_or(&self, operand: $value, ordering: UpdateOrdering) -> ($value, $value) {
                let old = self.storage.fetch_or(operand, ordering.to_std());
                (old, old | operand)
            }

            /// Bitwise XOR and returns `(old, new)`.
            pub fn bitwise_xor(&self, operand: $value, ordering: UpdateOrdering) -> ($value, $value) {
                let old = self.storage.fetch_xor(operand, ordering.to_std());
                (old, old ^ operand)
            }
        }
    )*};
}

atomic_integer!(i32, i64, isize, u32, u64, usize);

impl Atomic<bool> {
    /// Logical AND and returns `(old, new)`.
    pub fn bitwise_and(&self, operand: bool, ordering: UpdateOrdering) -> (bool, bool) {
        let old = self.storage.fetch_and(operand, ordering.to_std());
        (old, old & operand)
    }

    /// Logical OR and returns `(old, new)`.
    pub fn bitwise_or(&self, operand: bool, ordering: UpdateOrdering) -> (bool, bool) {
        let old = self.storage.fetch_or(operand, ordering.to_std());
        (old, old | operand)
    }

    /// Logical XOR and returns `(old, new)`.
    pub fn bitwise_xor(&self, operand: bool, ordering: UpdateOrdering) -> (bool, bool) {
        let old = self.storage.fetch_xor(operand, ordering.to_std());
        (old, old ^ operand)
    }
}

/// `Mutex<Value>`: mutual exclusion over one value, used through closures as the framework's `withLock` is.
pub struct Mutex<T> {
    inner: StdMutex<T>,
}

impl<T> Mutex<T> {
    pub fn new(value: T) -> Self {
        Self { inner: StdMutex::new(value) }
    }

    /// Runs `body` with the lock held and returns its result. Waits while another thread holds the lock.
    pub fn with_lock<R>(&self, body: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        body(&mut *guard)
    }

    /// Runs `body` only when the lock is free right now, and returns `None` otherwise. Never waits.
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

    pub fn into_inner(self) -> T {
        self.inner.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// `atomicMemoryFence(ordering:)`: orders memory accesses without an atomic operation. A relaxed fence does nothing.
pub fn atomic_memory_fence(ordering: UpdateOrdering) {
    match ordering {
        UpdateOrdering::Relaxed => {}
        other => atomic::fence(other.to_std()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_store_and_exchange_round_trip() {
        let a = Atomic::new(7_i64);
        assert_eq!(a.load(LoadOrdering::Acquiring), 7);
        a.store(9, StoreOrdering::Releasing);
        assert_eq!(a.exchange(11, UpdateOrdering::AcquiringAndReleasing), 9);
        assert_eq!(a.load(LoadOrdering::SequentiallyConsistent), 11);
    }

    #[test]
    fn compare_exchange_reports_the_current_value_on_failure() {
        let a = Atomic::new(1_u32);
        assert_eq!(a.compare_exchange(1, 2, UpdateOrdering::SequentiallyConsistent), Ok(1));
        assert_eq!(a.compare_exchange(1, 3, UpdateOrdering::Releasing), Err(2));
        assert_eq!(a.load(LoadOrdering::Relaxed), 2);
    }

    #[test]
    fn integer_add_wraps_and_returns_old_and_new() {
        let a = Atomic::new(i32::MAX);
        assert_eq!(a.add(1, UpdateOrdering::Relaxed), (i32::MAX, i32::MIN));
        assert_eq!(a.subtract(1, UpdateOrdering::Relaxed), (i32::MIN, i32::MAX));
    }

    #[test]
    fn bitwise_operations_on_integers_and_bools() {
        let a = Atomic::new(0b1100_u64);
        assert_eq!(a.bitwise_and(0b1010, UpdateOrdering::Relaxed), (0b1100, 0b1000));
        assert_eq!(a.bitwise_or(0b0001, UpdateOrdering::Relaxed), (0b1000, 0b1001));
        assert_eq!(a.bitwise_xor(0b1111, UpdateOrdering::Relaxed), (0b1001, 0b0110));
        let flag = Atomic::new(true);
        assert_eq!(flag.bitwise_and(false, UpdateOrdering::Relaxed), (true, false));
        assert!(!flag.load(LoadOrdering::Relaxed));
    }

    #[test]
    fn with_lock_mutates_the_protected_value() {
        let m = Mutex::new(vec![1]);
        let len = m.with_lock(|v| {
            v.push(2);
            v.len()
        });
        assert_eq!(len, 2);
        assert_eq!(m.into_inner(), vec![1, 2]);
    }

    #[test]
    fn with_lock_if_available_declines_while_the_lock_is_held() {
        let m = Mutex::new(0);
        m.with_lock(|v| {
            *v = 1;
            assert_eq!(m.with_lock_if_available(|w| *w), None);
        });
        assert_eq!(m.with_lock_if_available(|w| *w), Some(1));
    }

    #[test]
    fn a_panic_under_the_lock_does_not_poison_later_locks() {
        let m = std::sync::Arc::new(Mutex::new(5));
        let shared = m.clone();
        let _ = std::thread::spawn(move || shared.with_lock(|_: &mut i32| -> () { panic!("boom") })).join();
        assert_eq!(m.with_lock(|v| *v), 5);
    }

    #[test]
    fn memory_fences_accept_every_ordering() {
        atomic_memory_fence(UpdateOrdering::Relaxed);
        atomic_memory_fence(UpdateOrdering::Acquiring);
        atomic_memory_fence(UpdateOrdering::SequentiallyConsistent);
    }
}
