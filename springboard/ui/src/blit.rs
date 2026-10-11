//! Buffer fill and copy passes: a recorded list of byte-range commands over numbered buffers.
//! A pass is checked as a whole before any byte is written, so a bad command never leaves a buffer half updated.
//! This is the CPU reference for a GPU blit pass; a renderer can replay the same commands on its own queue.

use std::ops::Range;

/// One recorded command. Buffers are referred to by their index in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlitCommand {
    /// Sets every byte of `range` in `buffer` to `value`.
    Fill { buffer: usize, range: Range<usize>, value: u8 },
    /// Copies `size` bytes from `from` at `source_offset` into `to` at `destination_offset`.
    Copy { from: usize, source_offset: usize, to: usize, destination_offset: usize, size: usize },
}

/// Why a pass cannot run against the given buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlitError {
    /// A command names a buffer the store does not hold.
    NoSuchBuffer { buffer: usize },
    /// A range runs past the end of its buffer.
    OutOfBounds { buffer: usize, end: usize, length: usize },
    /// A fill range has its start after its end.
    ReversedRange { buffer: usize },
    /// A copy inside one buffer has source and destination ranges that overlap. The model refuses these rather
    /// than guess an order of the bytes.
    OverlappingCopy { buffer: usize },
}

/// An ordered list of fill and copy commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlitPass {
    commands: Vec<BlitCommand>,
}

impl BlitPass {
    /// An empty pass.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a fill of `range` in `buffer` with `value`.
    pub fn fill(&mut self, buffer: usize, range: Range<usize>, value: u8) {
        self.commands.push(BlitCommand::Fill { buffer, range, value });
    }

    /// Records a copy of `size` bytes between two buffers (or two places in one buffer).
    pub fn copy(&mut self, from: usize, source_offset: usize, to: usize, destination_offset: usize, size: usize) {
        self.commands.push(BlitCommand::Copy { from, source_offset, to, destination_offset, size });
    }

    /// The recorded commands in order.
    pub fn commands(&self) -> &[BlitCommand] {
        &self.commands
    }

    /// Number of recorded commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// True when nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Checks every command against the buffer lengths (`lengths[i]` is the size of buffer `i`) without writing.
    pub fn validate(&self, lengths: &[usize]) -> Result<(), BlitError> {
        for command in &self.commands {
            match command {
                BlitCommand::Fill { buffer, range, .. } => {
                    let length = buffer_length(lengths, *buffer)?;
                    if range.start > range.end {
                        return Err(BlitError::ReversedRange { buffer: *buffer });
                    }
                    check_span(*buffer, range.start, range.end - range.start, length)?;
                }
                BlitCommand::Copy { from, source_offset, to, destination_offset, size } => {
                    let from_length = buffer_length(lengths, *from)?;
                    let to_length = buffer_length(lengths, *to)?;
                    let source = check_span(*from, *source_offset, *size, from_length)?;
                    let destination = check_span(*to, *destination_offset, *size, to_length)?;
                    let overlap = source.start < destination.end && destination.start < source.end;
                    if from == to && *size > 0 && overlap {
                        return Err(BlitError::OverlappingCopy { buffer: *from });
                    }
                }
            }
        }
        Ok(())
    }

    /// Validates the whole pass, then applies its commands in order. Returns how many commands ran.
    /// On error no buffer is changed.
    pub fn execute(&self, store: &mut [Vec<u8>]) -> Result<usize, BlitError> {
        let lengths: Vec<usize> = store.iter().map(|b| b.len()).collect();
        self.validate(&lengths)?;
        for command in &self.commands {
            match command {
                BlitCommand::Fill { buffer, range, value } => {
                    store[*buffer][range.clone()].fill(*value);
                }
                BlitCommand::Copy { from, source_offset, to, destination_offset, size } => {
                    if from == to {
                        store[*from].copy_within(*source_offset..*source_offset + *size, *destination_offset);
                    } else {
                        let bytes: Vec<u8> = store[*from][*source_offset..*source_offset + *size].to_vec();
                        store[*to][*destination_offset..*destination_offset + *size].copy_from_slice(&bytes);
                    }
                }
            }
        }
        Ok(self.commands.len())
    }
}

fn buffer_length(lengths: &[usize], buffer: usize) -> Result<usize, BlitError> {
    lengths.get(buffer).copied().ok_or(BlitError::NoSuchBuffer { buffer })
}

/// The byte range `start..start+length`, or `OutOfBounds` when it does not fit in `buffer_length`.
fn check_span(buffer: usize, start: usize, length: usize, buffer_length: usize) -> Result<Range<usize>, BlitError> {
    let end = start.checked_add(length).unwrap_or(usize::MAX);
    if end > buffer_length {
        return Err(BlitError::OutOfBounds { buffer, end, length: buffer_length });
    }
    Ok(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Vec<Vec<u8>> {
        vec![vec![0u8; 8], vec![0u8; 4]]
    }

    #[test]
    fn fill_sets_only_the_requested_range() {
        let mut pass = BlitPass::new();
        pass.fill(0, 2..5, 7);
        let mut s = store();
        assert_eq!(pass.execute(&mut s), Ok(1));
        assert_eq!(s[0], vec![0, 0, 7, 7, 7, 0, 0, 0]);
        assert_eq!(s[1], vec![0, 0, 0, 0]);
    }

    #[test]
    fn copy_moves_bytes_between_buffers() {
        let mut pass = BlitPass::new();
        pass.fill(1, 0..4, 9);
        pass.copy(1, 1, 0, 4, 3);
        let mut s = store();
        assert_eq!(pass.execute(&mut s), Ok(2));
        assert_eq!(s[0], vec![0, 0, 0, 0, 9, 9, 9, 0]);
    }

    #[test]
    fn copy_inside_one_buffer_works_when_the_ranges_are_disjoint() {
        let mut s = vec![vec![1, 2, 3, 4, 0, 0, 0, 0]];
        let mut pass = BlitPass::new();
        pass.copy(0, 0, 0, 4, 4);
        assert_eq!(pass.execute(&mut s), Ok(1));
        assert_eq!(s[0], vec![1, 2, 3, 4, 1, 2, 3, 4]);
    }

    #[test]
    fn overlapping_copy_in_one_buffer_is_refused() {
        let mut s = vec![vec![1, 2, 3, 4, 0, 0, 0, 0]];
        let mut pass = BlitPass::new();
        pass.copy(0, 0, 0, 2, 4);
        assert_eq!(pass.execute(&mut s), Err(BlitError::OverlappingCopy { buffer: 0 }));
        assert_eq!(s[0], vec![1, 2, 3, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn a_bad_command_leaves_every_buffer_untouched() {
        let mut pass = BlitPass::new();
        pass.fill(0, 0..8, 5);
        pass.copy(0, 6, 1, 0, 4);
        let mut s = store();
        assert_eq!(
            pass.execute(&mut s),
            Err(BlitError::OutOfBounds { buffer: 0, end: 10, length: 8 })
        );
        assert_eq!(s, store());
    }

    #[test]
    fn missing_buffers_and_reversed_ranges_are_rejected() {
        let mut pass = BlitPass::new();
        pass.fill(3, 0..1, 1);
        assert_eq!(pass.validate(&[8, 4]), Err(BlitError::NoSuchBuffer { buffer: 3 }));

        let mut reversed = BlitPass::new();
        reversed.fill(0, 5..2, 1);
        assert_eq!(reversed.validate(&[8]), Err(BlitError::ReversedRange { buffer: 0 }));
    }

    #[test]
    fn zero_sized_copies_are_allowed_anywhere_in_bounds() {
        let mut pass = BlitPass::new();
        pass.copy(0, 8, 0, 8, 0);
        assert!(pass.validate(&[8]).is_ok());
        assert_eq!(pass.len(), 1);
        assert!(!pass.is_empty());
        assert!(BlitPass::new().is_empty());
    }
}
