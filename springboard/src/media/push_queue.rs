//! Push buffers: an app pushes blocks of interleaved samples into a bounded queue, and the output side pulls a block of
//! an exact length. Blocks play in priority order, and blocks of one priority play in the order they were pushed.
//! When the queue runs dry the pull is filled with silence and the shortfall is reported, so the caller can count
//! underruns. Pure state: the queue holds samples, not a device, so it is tested without audio hardware.

use std::cmp::Reverse;
use std::collections::VecDeque;
use std::fmt;

/// The order in which queued blocks play. Higher priorities play first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Normal,
    High,
}

/// Why a block was not queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushError {
    /// The block has no samples.
    EmptyBlock,
    /// The block does not fit; `free` samples are left.
    Full { free: usize },
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushError::EmptyBlock => f.write_str("the block has no samples"),
            PushError::Full { free } => write!(f, "the queue has room for {free} more samples"),
        }
    }
}

/// The samples pulled for one output block and how many of them are silence because the queue ran dry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pulled {
    pub samples: Vec<i16>,
    pub underrun: usize,
}

#[derive(Clone, Debug)]
struct Block {
    priority: Priority,
    seq: u64,
    samples: VecDeque<i16>,
}

/// A bounded queue of sample blocks. Its capacity is fixed when it is made, so an app that pushes too fast is told so.
#[derive(Clone, Debug)]
pub struct PushQueue {
    capacity: usize,
    queued: usize,
    next_seq: u64,
    blocks: Vec<Block>,
}

impl PushQueue {
    /// A queue that holds up to `capacity` samples. Returns `None` for a zero capacity.
    pub fn new(capacity: usize) -> Option<Self> {
        (capacity > 0).then(|| PushQueue { capacity, queued: 0, next_seq: 0, blocks: Vec::new() })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The samples waiting to play.
    pub fn queued(&self) -> usize {
        self.queued
    }

    /// The samples that still fit.
    pub fn free(&self) -> usize {
        self.capacity - self.queued
    }

    /// Queues a block at `priority`. A block that does not fit is refused whole, so a block never plays partly.
    pub fn push(&mut self, priority: Priority, samples: &[i16]) -> Result<(), PushError> {
        if samples.is_empty() {
            return Err(PushError::EmptyBlock);
        }
        if samples.len() > self.free() {
            return Err(PushError::Full { free: self.free() });
        }
        self.blocks.push(Block { priority, seq: self.next_seq, samples: samples.iter().copied().collect() });
        self.next_seq += 1;
        self.queued += samples.len();
        Ok(())
    }

    /// Takes exactly `samples` samples for the output. Missing samples are silence and are counted as underrun.
    pub fn pull(&mut self, samples: usize) -> Pulled {
        let mut out = Vec::with_capacity(samples);
        while out.len() < samples {
            let Some(index) = self.next_block() else { break };
            let (taken, drained) = {
                let block = &mut self.blocks[index];
                let take = (samples - out.len()).min(block.samples.len());
                out.extend(block.samples.drain(..take));
                (take, block.samples.is_empty())
            };
            self.queued -= taken;
            if drained {
                self.blocks.remove(index);
            }
        }
        let underrun = samples - out.len();
        out.resize(samples, 0);
        Pulled { samples: out, underrun }
    }

    /// The index of the block that plays next: the highest priority, and the earliest push among equals.
    fn next_block(&self) -> Option<usize> {
        self.blocks.iter().enumerate().min_by_key(|(_, b)| (Reverse(b.priority), b.seq)).map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queue_needs_room_for_at_least_one_sample() {
        assert!(PushQueue::new(0).is_none());
        assert_eq!(PushQueue::new(4).map(|q| q.free()), Some(4));
    }

    #[test]
    fn a_block_that_does_not_fit_is_refused_whole() {
        let mut q = PushQueue::new(4).unwrap();
        q.push(Priority::Normal, &[1, 2, 3]).unwrap();
        assert_eq!(q.push(Priority::Normal, &[4, 5]), Err(PushError::Full { free: 1 }));
        assert_eq!(q.queued(), 3, "nothing from the refused block was queued");
        assert_eq!(q.push(Priority::Normal, &[]), Err(PushError::EmptyBlock));
    }

    #[test]
    fn equal_priority_blocks_play_in_the_order_they_were_pushed() {
        let mut q = PushQueue::new(16).unwrap();
        q.push(Priority::Normal, &[1, 2]).unwrap();
        q.push(Priority::Normal, &[3, 4]).unwrap();
        assert_eq!(q.pull(4).samples, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_higher_priority_block_plays_before_an_earlier_lower_one() {
        let mut q = PushQueue::new(16).unwrap();
        q.push(Priority::Low, &[10, 11]).unwrap();
        q.push(Priority::High, &[90, 91]).unwrap();
        assert_eq!(q.pull(2).samples, vec![90, 91]);
        assert_eq!(q.pull(2).samples, vec![10, 11], "the low block resumes afterwards");
    }

    #[test]
    fn a_partly_played_block_keeps_its_place_across_pulls() {
        let mut q = PushQueue::new(16).unwrap();
        q.push(Priority::Normal, &[1, 2, 3, 4, 5]).unwrap();
        assert_eq!(q.pull(2).samples, vec![1, 2]);
        assert_eq!(q.queued(), 3);
        assert_eq!(q.pull(3).samples, vec![3, 4, 5]);
        assert_eq!(q.queued(), 0);
    }

    #[test]
    fn a_dry_queue_pads_with_silence_and_reports_the_shortfall() {
        let mut q = PushQueue::new(8).unwrap();
        q.push(Priority::Normal, &[7]).unwrap();
        let pulled = q.pull(4);
        assert_eq!(pulled, Pulled { samples: vec![7, 0, 0, 0], underrun: 3 });
        assert_eq!(q.pull(2), Pulled { samples: vec![0, 0], underrun: 2 });
    }

    #[test]
    fn freed_room_can_be_filled_again_after_a_pull() {
        let mut q = PushQueue::new(3).unwrap();
        q.push(Priority::Normal, &[1, 2, 3]).unwrap();
        assert_eq!(q.push(Priority::Normal, &[4]), Err(PushError::Full { free: 0 }));
        q.pull(2);
        assert!(q.push(Priority::Normal, &[4, 5]).is_ok());
        assert_eq!(q.pull(3).samples, vec![3, 4, 5]);
    }
}
