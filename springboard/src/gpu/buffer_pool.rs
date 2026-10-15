//! A pool of render buffers that are reused across frames, with a fence per submission. A buffer moves from free, to
//! being written by the CPU, to in flight on the GPU, and back to free once its fence completes. Completion may come
//! in any order, because a frame can finish before an older one. Pure bookkeeping: the pool holds indices and tickets,
//! never the pixels, so the same rules apply to any backend.

/// The state of one buffer in the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Free,
    Writing,
    InFlight(u64),
}

/// A buffer the CPU may write. Hand it back with [`BufferPool::submit`] or [`BufferPool::abandon`].
#[derive(Debug, PartialEq, Eq)]
pub struct Lease {
    index: usize,
}

impl Lease {
    /// The buffer's position in the pool, for the backend that owns the memory.
    pub fn index(&self) -> usize {
        self.index
    }
}

/// The completion token of one submission. It is done once the pool has retired its buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fence {
    ticket: u64,
}

/// A fixed set of reusable buffers. A pool with no buffers would never hand one out, so it cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferPool {
    slots: Vec<Slot>,
    next_ticket: u64,
}

impl BufferPool {
    /// A pool of `count` buffers, all free. Returns `None` for zero buffers.
    pub fn new(count: usize) -> Option<Self> {
        (count > 0).then(|| BufferPool { slots: vec![Slot::Free; count], next_ticket: 1 })
    }

    /// The number of buffers in the pool.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many buffers are free to write right now.
    pub fn free_count(&self) -> usize {
        self.slots.iter().filter(|s| **s == Slot::Free).count()
    }

    /// The first free buffer, now owned by the CPU for writing. `None` when every buffer is busy, so the caller waits
    /// for a fence instead of overwriting a frame the GPU still reads.
    pub fn acquire(&mut self) -> Option<Lease> {
        let index = self.slots.iter().position(|s| *s == Slot::Free)?;
        self.slots[index] = Slot::Writing;
        Some(Lease { index })
    }

    /// Gives the buffer to the GPU and returns the fence that completes when the GPU is done with it.
    pub fn submit(&mut self, lease: Lease) -> Fence {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        self.slots[lease.index] = Slot::InFlight(ticket);
        Fence { ticket }
    }

    /// Returns a buffer that was written but not submitted, so it can be reused at once.
    pub fn abandon(&mut self, lease: Lease) {
        self.slots[lease.index] = Slot::Free;
    }

    /// True when the GPU has retired the submission behind `fence`, so its buffer is free again or has been reused.
    pub fn is_done(&self, fence: Fence) -> bool {
        !self.slots.contains(&Slot::InFlight(fence.ticket))
    }

    /// Retires the submission behind `fence`, freeing its buffer. Returns false when no buffer waits on that fence,
    /// for example because it was already retired.
    pub fn complete(&mut self, fence: Fence) -> bool {
        match self.slots.iter().position(|s| *s == Slot::InFlight(fence.ticket)) {
            Some(index) => {
                self.slots[index] = Slot::Free;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pool_needs_at_least_one_buffer() {
        assert!(BufferPool::new(0).is_none());
        assert_eq!(BufferPool::new(3).map(|p| p.free_count()), Some(3));
    }

    #[test]
    fn a_busy_pool_hands_out_no_buffer_until_a_fence_completes() {
        let mut pool = BufferPool::new(2).unwrap();
        let a = pool.acquire().unwrap();
        let b = pool.acquire().unwrap();
        let fa = pool.submit(a);
        let _fb = pool.submit(b);
        assert!(pool.acquire().is_none(), "both buffers are on the GPU");
        assert!(pool.complete(fa));
        assert_eq!(pool.acquire().map(|l| l.index()), Some(0), "the first buffer is reused");
    }

    #[test]
    fn fences_complete_in_any_order() {
        let mut pool = BufferPool::new(3).unwrap();
        let fences: Vec<Fence> = (0..3).map(|_| {
            let lease = pool.acquire().unwrap();
            pool.submit(lease)
        }).collect();
        assert!(pool.complete(fences[2]));
        assert!(pool.complete(fences[0]));
        assert!(!pool.is_done(fences[1]), "the middle frame is still on the GPU");
        assert!(pool.is_done(fences[2]));
        assert_eq!(pool.free_count(), 2);
    }

    #[test]
    fn a_fence_retires_its_buffer_only_once() {
        let mut pool = BufferPool::new(1).unwrap();
        let lease = pool.acquire().unwrap();
        let fence = pool.submit(lease);
        assert!(pool.complete(fence));
        assert!(!pool.complete(fence), "already retired");
        assert!(pool.is_done(fence));
    }

    #[test]
    fn an_abandoned_write_frees_its_buffer_without_a_fence() {
        let mut pool = BufferPool::new(1).unwrap();
        let lease = pool.acquire().unwrap();
        assert!(pool.acquire().is_none());
        pool.abandon(lease);
        assert_eq!(pool.free_count(), 1);
    }
}
