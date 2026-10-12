//! Block pool: reusable byte blocks for large media buffers. Taking a block reuses a returned one when it is big
//! enough, so a steady stream of same-sized frames stops allocating. The pool keeps at most `max_idle` blocks and
//! drops the rest, and it counts what it did so the shell can report on it. This is the pure model: no I/O and no
//! system calls.

/// Counters for a pool. `allocated` counts fresh allocations, `reused` counts blocks handed out from the idle list,
/// and `dropped` counts returned blocks the pool refused to keep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    pub allocated: u64,
    pub reused: u64,
    pub dropped: u64,
    pub idle: usize,
}

/// A pool of blocks that each have at least `block_size` bytes of capacity.
#[derive(Clone, Debug)]
pub struct BlockPool {
    block_size: usize,
    max_idle: usize,
    idle: Vec<Vec<u8>>,
    allocated: u64,
    reused: u64,
    dropped: u64,
}

impl BlockPool {
    /// A pool for blocks of `block_size` bytes that keeps at most `max_idle` returned blocks.
    pub fn new(block_size: usize, max_idle: usize) -> Self {
        BlockPool {
            block_size,
            max_idle,
            idle: Vec::new(),
            allocated: 0,
            reused: 0,
            dropped: 0,
        }
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Hands out an empty block with at least `block_size` capacity. A returned block is reused when one is idle;
    /// otherwise a new one is allocated.
    pub fn take(&mut self) -> Vec<u8> {
        match self.idle.pop() {
            Some(mut block) => {
                self.reused += 1;
                block.clear();
                block
            }
            None => {
                self.allocated += 1;
                Vec::with_capacity(self.block_size)
            }
        }
    }

    /// Returns a block to the pool. A block with less capacity than `block_size`, or any block arriving while the pool
    /// is full, is dropped. Returns whether the block was kept.
    pub fn give(&mut self, block: Vec<u8>) -> bool {
        if block.capacity() < self.block_size || self.idle.len() >= self.max_idle {
            self.dropped += 1;
            return false;
        }
        self.idle.push(block);
        true
    }

    /// Drops every idle block and returns how many were released.
    pub fn trim(&mut self) -> usize {
        let released = self.idle.len();
        self.idle.clear();
        released
    }

    pub fn stats(&self) -> PoolStats {
        PoolStats {
            allocated: self.allocated,
            reused: self.reused,
            dropped: self.dropped,
            idle: self.idle.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_take_allocates_with_block_capacity() {
        let mut pool = BlockPool::new(4096, 2);
        let block = pool.take();
        assert_eq!(block.len(), 0);
        assert!(block.capacity() >= 4096);
        assert_eq!(pool.stats(), PoolStats { allocated: 1, reused: 0, dropped: 0, idle: 0 });
    }

    #[test]
    fn given_block_is_reused_by_the_next_take() {
        let mut pool = BlockPool::new(1024, 4);
        let block = pool.take();
        assert!(pool.give(block));
        assert_eq!(pool.stats().idle, 1);
        let again = pool.take();
        assert!(again.capacity() >= 1024);
        assert_eq!(pool.stats(), PoolStats { allocated: 1, reused: 1, dropped: 0, idle: 0 });
    }

    #[test]
    fn reused_block_comes_back_empty() {
        let mut pool = BlockPool::new(16, 1);
        let mut block = pool.take();
        block.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(block.len(), 5);
        assert!(pool.give(block));
        let reused = pool.take();
        assert_eq!(reused.len(), 0);
        assert!(reused.capacity() >= 16);
    }

    #[test]
    fn undersized_block_is_refused_and_counted() {
        let mut pool = BlockPool::new(100, 4);
        assert!(!pool.give(Vec::with_capacity(10)));
        assert_eq!(pool.stats(), PoolStats { allocated: 0, reused: 0, dropped: 1, idle: 0 });
    }

    #[test]
    fn full_idle_list_drops_extra_blocks() {
        let mut pool = BlockPool::new(8, 2);
        assert!(pool.give(Vec::with_capacity(8)));
        assert!(pool.give(Vec::with_capacity(8)));
        assert!(!pool.give(Vec::with_capacity(8)));
        assert_eq!(pool.stats().idle, 2);
        assert_eq!(pool.stats().dropped, 1);
    }

    #[test]
    fn trim_releases_every_idle_block() {
        let mut pool = BlockPool::new(8, 5);
        pool.give(Vec::with_capacity(8));
        pool.give(Vec::with_capacity(8));
        pool.give(Vec::with_capacity(8));
        assert_eq!(pool.trim(), 3);
        assert_eq!(pool.stats().idle, 0);
        assert_eq!(pool.trim(), 0);
    }
}
