//! Block buffers and a bounded memory pool for media samples. A block buffer is a list of owned byte blocks that
//! reads as one run of bytes; slices copy a range of the source, and appending another buffer copies its blocks
//! (this model has no shared reference counts). A memory pool keeps freed blocks so that a steady stream of
//! same-sized frames reuses allocations. This is the pure model behind sample storage; no media is decoded here.

use std::ops::Range;

/// Failures of the block and pool model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockError {
    /// An offset or a range does not lie inside the source.
    OutOfRange,
}

/// A run of bytes held as a list of blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockBuffer {
    blocks: Vec<Vec<u8>>,
}

impl BlockBuffer {
    /// An empty buffer with no blocks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a block that copies `bytes`. An empty slice adds nothing.
    pub fn append(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.blocks.push(bytes.to_vec());
        }
    }

    /// Adds a block that copies `range` of `bytes`. A range that is reversed or runs past the end is refused.
    pub fn append_slice(&mut self, bytes: &[u8], range: Range<usize>) -> Result<(), BlockError> {
        if range.start > range.end || range.end > bytes.len() {
            return Err(BlockError::OutOfRange);
        }
        self.append(&bytes[range]);
        Ok(())
    }

    /// Adds copies of every block of `other`, in order.
    pub fn append_buffer(&mut self, other: &BlockBuffer) {
        self.blocks.extend(other.blocks.iter().cloned());
    }

    /// Total number of bytes across all blocks.
    pub fn len(&self) -> usize {
        self.blocks.iter().map(|b| b.len()).sum()
    }

    /// True when the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Copies bytes starting at `offset` into `out` and returns how many were copied. An offset equal to the
    /// length copies nothing; an offset past the length is refused.
    pub fn read(&self, offset: usize, out: &mut [u8]) -> Result<usize, BlockError> {
        if offset > self.len() {
            return Err(BlockError::OutOfRange);
        }
        let mut skip = offset;
        let mut written = 0;
        for block in &self.blocks {
            if written == out.len() {
                break;
            }
            if skip >= block.len() {
                skip -= block.len();
                continue;
            }
            let avail = &block[skip..];
            skip = 0;
            let n = avail.len().min(out.len() - written);
            out[written..written + n].copy_from_slice(&avail[..n]);
            written += n;
        }
        Ok(written)
    }

    /// The whole buffer as one contiguous vector.
    pub fn contiguous(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len());
        for block in &self.blocks {
            out.extend_from_slice(block);
        }
        out
    }
}

/// A bounded free list of byte vectors. `take` returns the smallest free block that is large enough (zeroed to
/// the requested size), and `give` returns a block to the list or drops it when the list is full.
#[derive(Debug)]
pub struct MemoryPool {
    free: Vec<Vec<u8>>,
    max_free: usize,
    created: u64,
    reused: u64,
}

impl MemoryPool {
    /// A pool that keeps at most `max_free` blocks idle.
    pub fn new(max_free: usize) -> Self {
        Self { free: Vec::new(), max_free, created: 0, reused: 0 }
    }

    /// A zeroed block of `size` bytes, reused from the free list when a large enough one is idle.
    pub fn take(&mut self, size: usize) -> Vec<u8> {
        let mut best: Option<usize> = None;
        for (i, block) in self.free.iter().enumerate() {
            if block.capacity() >= size
                && best.map_or(true, |j| self.free[j].capacity() > block.capacity())
            {
                best = Some(i);
            }
        }
        match best {
            Some(i) => {
                let mut block = self.free.swap_remove(i);
                block.clear();
                block.resize(size, 0);
                self.reused += 1;
                block
            }
            None => {
                self.created += 1;
                vec![0; size]
            }
        }
    }

    /// Returns a block to the pool. When the pool already holds `max_free` idle blocks, the block is dropped.
    pub fn give(&mut self, block: Vec<u8>) {
        if self.free.len() < self.max_free {
            self.free.push(block);
        }
    }

    /// Blocks currently idle in the pool.
    pub fn idle_count(&self) -> usize {
        self.free.len()
    }

    /// Blocks allocated fresh because no idle block fit.
    pub fn created(&self) -> u64 {
        self.created
    }

    /// Blocks handed out from the idle list.
    pub fn reused(&self) -> u64 {
        self.reused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_buffer_reads_across_blocks() {
        let mut b = BlockBuffer::new();
        b.append(b"abc");
        b.append(b"de");
        assert_eq!(b.len(), 5);
        assert_eq!(b.block_count(), 2);
        let mut out = [0u8; 3];
        assert_eq!(b.read(1, &mut out), Ok(3));
        assert_eq!(&out, b"bcd");
        assert_eq!(b.contiguous(), b"abcde".to_vec());
    }

    #[test]
    fn block_buffer_read_at_end_and_past_end() {
        let mut b = BlockBuffer::new();
        b.append(b"xy");
        let mut out = [0u8; 4];
        assert_eq!(b.read(2, &mut out), Ok(0));
        assert_eq!(b.read(3, &mut out), Err(BlockError::OutOfRange));
    }

    #[test]
    fn append_slice_copies_only_the_range() {
        let mut b = BlockBuffer::new();
        b.append_slice(b"wxyz", 1..3).unwrap();
        assert_eq!(b.contiguous(), b"xy".to_vec());
        assert_eq!(b.append_slice(b"wxyz", 3..9), Err(BlockError::OutOfRange));
        assert_eq!(b.append_slice(b"wxyz", 2..1), Err(BlockError::OutOfRange));
        assert_eq!(b.block_count(), 1);
    }

    #[test]
    fn empty_append_adds_no_block() {
        let mut b = BlockBuffer::new();
        b.append(&[]);
        assert!(b.is_empty());
        assert_eq!(b.block_count(), 0);
        let mut other = BlockBuffer::new();
        other.append(b"q");
        b.append_buffer(&other);
        assert_eq!(b.contiguous(), b"q".to_vec());
    }

    #[test]
    fn pool_reuses_a_block_that_is_large_enough() {
        let mut pool = MemoryPool::new(4);
        let first = pool.take(10);
        assert_eq!(first.len(), 10);
        assert_eq!((pool.created(), pool.reused()), (1, 0));
        pool.give(first);
        assert_eq!(pool.idle_count(), 1);
        let second = pool.take(4);
        assert_eq!(second, vec![0u8; 4]);
        assert_eq!((pool.created(), pool.reused()), (1, 1));
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn pool_takes_the_smallest_fitting_block() {
        let mut pool = MemoryPool::new(4);
        let mut big = Vec::with_capacity(16);
        big.resize(16, 9);
        let mut small = Vec::with_capacity(8);
        small.resize(8, 9);
        pool.give(big);
        pool.give(small);
        let got = pool.take(6);
        assert_eq!(got.capacity(), 8);
        assert_eq!(pool.idle_count(), 1);
        assert_eq!(pool.reused(), 1);
    }

    #[test]
    fn pool_drops_blocks_beyond_its_limit() {
        let mut pool = MemoryPool::new(1);
        pool.give(vec![0; 4]);
        pool.give(vec![0; 4]);
        assert_eq!(pool.idle_count(), 1);
        let mut none = MemoryPool::new(0);
        none.give(vec![0; 4]);
        assert_eq!(none.idle_count(), 0);
        let _ = none.take(4);
        assert_eq!(none.created(), 1);
    }
}
