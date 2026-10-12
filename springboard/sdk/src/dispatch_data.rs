//! Dispatch data (`DispatchData`): an immutable byte buffer that may be made of several discontiguous regions, and
//! that presents them as one sequence of bytes.
//!
//! A value is a list of pieces. Each piece is a window onto shared bytes (`Arc<Vec<u8>>`), so appending another
//! `DispatchData` or taking a subrange copies no bytes: it adds or narrows pieces that point at the same storage.
//! The bytes are copied only when they are built from a slice (`from_slice`, `append_slice`) or read out (`to_vec`,
//! `copy_bytes`). `region` returns the one piece that holds a location, and `enumerate_bytes` walks the pieces in
//! order, as the framework's own enumeration does.
//!
//! Indices are byte offsets from the start of the whole sequence. Where the framework traps on a range that is out of
//! bounds, this model returns `None` (`subdata`, `get`, `region`) or clamps (`copy_bytes`) so that the caller decides.

use std::ops::Range;
use std::sync::Arc;

/// One window onto shared storage: `bytes[start..end]`.
#[derive(Clone, Debug)]
struct Piece {
    bytes: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl Piece {
    fn len(&self) -> usize {
        self.end - self.start
    }

    fn slice(&self) -> &[u8] {
        &self.bytes[self.start..self.end]
    }
}

/// `DispatchData`: an immutable, possibly discontiguous byte sequence. Cloning is cheap; it shares the storage.
#[derive(Clone, Debug, Default)]
pub struct DispatchData {
    pieces: Vec<Piece>,
    len: usize,
}

impl DispatchData {
    /// `DispatchData.empty`: a sequence with no bytes and no regions.
    pub fn empty() -> DispatchData {
        DispatchData::default()
    }

    /// `init(bytes:)`: a sequence that holds a copy of the slice.
    pub fn from_slice(bytes: &[u8]) -> DispatchData {
        let mut data = DispatchData::empty();
        data.append_slice(bytes);
        data
    }

    /// `init(bytes:)` for owned storage: takes the vector without copying it.
    pub fn from_vec(bytes: Vec<u8>) -> DispatchData {
        let len = bytes.len();
        if len == 0 {
            return DispatchData::empty();
        }
        DispatchData {
            pieces: vec![Piece { bytes: Arc::new(bytes), start: 0, end: len }],
            len,
        }
    }

    /// The number of bytes in the whole sequence.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the sequence holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of discontiguous regions. An empty sequence has none.
    pub fn region_count(&self) -> usize {
        self.pieces.len()
    }

    /// `append(_:)` with another sequence: shares its regions, copies no bytes. Appending an empty sequence changes
    /// nothing.
    pub fn append(&mut self, other: &DispatchData) {
        for piece in &other.pieces {
            self.pieces.push(piece.clone());
        }
        self.len += other.len;
    }

    /// `append(_:)` with raw bytes: copies the slice into a new region. An empty slice adds no region.
    pub fn append_slice(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let len = bytes.len();
        self.pieces.push(Piece { bytes: Arc::new(bytes.to_vec()), start: 0, end: len });
        self.len += len;
    }

    /// The byte at `index`, or `None` when the index is past the end.
    pub fn get(&self, index: usize) -> Option<u8> {
        if index >= self.len {
            return None;
        }
        let mut pos = 0;
        for piece in &self.pieces {
            let next = pos + piece.len();
            if index < next {
                return Some(piece.slice()[index - pos]);
            }
            pos = next;
        }
        None
    }

    /// `subdata(in:)`: a new sequence over `range`, sharing the storage. `None` when the range starts after it ends or
    /// ends past the length. An empty range gives an empty sequence.
    pub fn subdata(&self, range: Range<usize>) -> Option<DispatchData> {
        if range.start > range.end || range.end > self.len {
            return None;
        }
        let mut out = DispatchData::empty();
        let mut pos = 0;
        for piece in &self.pieces {
            let plen = piece.len();
            let a = range.start.max(pos);
            let b = range.end.min(pos + plen);
            if a < b {
                out.pieces.push(Piece {
                    bytes: Arc::clone(&piece.bytes),
                    start: piece.start + (a - pos),
                    end: piece.start + (b - pos),
                });
                out.len += b - a;
            }
            pos += plen;
            if pos >= range.end {
                break;
            }
        }
        Some(out)
    }

    /// `region(location:)`: the region that holds `location`, and the offset of `location` inside that region. The
    /// region is returned as a sequence of its own. `None` when the location is past the end.
    pub fn region(&self, location: usize) -> Option<(DispatchData, usize)> {
        if location >= self.len {
            return None;
        }
        let mut pos = 0;
        for piece in &self.pieces {
            let plen = piece.len();
            if location < pos + plen {
                let region = DispatchData { pieces: vec![piece.clone()], len: plen };
                return Some((region, location - pos));
            }
            pos += plen;
        }
        None
    }

    /// `copyBytes(to:from:)`: copies the bytes of `range` into the front of `dest`. The range is clamped to the
    /// sequence and the copy stops when `dest` is full. Returns the number of bytes written (0 for an empty range).
    pub fn copy_bytes(&self, dest: &mut [u8], range: Range<usize>) -> usize {
        let room = dest.len();
        let end = range.end.min(self.len);
        let mut written = 0;
        let mut pos = 0;
        for piece in &self.pieces {
            if written >= room || pos >= end {
                break;
            }
            let plen = piece.len();
            let a = range.start.max(pos);
            let b = end.min(pos + plen);
            if a < b {
                let n = (b - a).min(room - written);
                let s = piece.start + (a - pos);
                dest[written..written + n].copy_from_slice(&piece.bytes[s..s + n]);
                written += n;
            }
            pos += plen;
        }
        written
    }

    /// `enumerateBytes`: calls `visit` with each region's bytes and the offset of its first byte, in order. The walk
    /// stops as soon as `visit` returns `false` (the framework's `inout Bool` stop flag).
    pub fn enumerate_bytes<F: FnMut(&[u8], usize) -> bool>(&self, mut visit: F) {
        let mut pos = 0;
        for piece in &self.pieces {
            if !visit(piece.slice(), pos) {
                return;
            }
            pos += piece.len();
        }
    }

    /// The whole sequence as one vector.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len);
        self.enumerate_bytes(|chunk, _| {
            out.extend_from_slice(chunk);
            true
        });
        out
    }
}

impl PartialEq for DispatchData {
    fn eq(&self, other: &DispatchData) -> bool {
        // Equal contents, whatever the region boundaries are.
        self.len == other.len && self.to_vec() == other.to_vec()
    }
}

impl Eq for DispatchData {}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_regions() -> DispatchData {
        let mut data = DispatchData::from_slice(&[1, 2]);
        data.append(&DispatchData::from_slice(&[3, 4, 5]));
        data
    }

    #[test]
    fn appended_regions_read_as_one_sequence() {
        let data = two_regions();
        assert_eq!(data.len(), 5);
        assert_eq!(data.region_count(), 2);
        assert_eq!(data.to_vec(), vec![1, 2, 3, 4, 5]);
        assert_eq!(data.get(0), Some(1));
        assert_eq!(data.get(2), Some(3));
        assert_eq!(data.get(5), None);
    }

    #[test]
    fn subdata_narrows_across_region_boundaries() {
        let data = two_regions();
        assert_eq!(data.subdata(1..4).unwrap().to_vec(), vec![2, 3, 4]);
        assert_eq!(data.subdata(3..3).unwrap().len(), 0);
        assert!(data.subdata(4..6).is_none());
        assert!(data.subdata(3..2).is_none());
    }

    #[test]
    fn region_reports_the_piece_and_the_offset_inside_it() {
        let data = two_regions();
        let (first, offset) = data.region(1).unwrap();
        assert_eq!((first.to_vec(), offset), (vec![1, 2], 1));
        let (second, offset) = data.region(2).unwrap();
        assert_eq!((second.to_vec(), offset), (vec![3, 4, 5], 0));
        assert!(data.region(5).is_none());
    }

    #[test]
    fn copy_bytes_clamps_the_range_and_stops_when_the_destination_is_full() {
        let data = two_regions();
        let mut four = [0u8; 4];
        assert_eq!(data.copy_bytes(&mut four, 1..5), 4);
        assert_eq!(four, [2, 3, 4, 5]);
        let mut six = [9u8; 6];
        assert_eq!(data.copy_bytes(&mut six, 3..99), 2);
        assert_eq!(six, [4, 5, 9, 9, 9, 9]);
        assert_eq!(data.copy_bytes(&mut six, 2..2), 0);
    }

    #[test]
    fn enumerate_bytes_visits_each_region_until_asked_to_stop() {
        let data = two_regions();
        let mut seen = Vec::new();
        data.enumerate_bytes(|chunk, offset| {
            seen.push((chunk.len(), offset));
            true
        });
        assert_eq!(seen, vec![(2, 0), (3, 2)]);

        let mut calls = 0;
        data.enumerate_bytes(|_, _| {
            calls += 1;
            false
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn empty_appends_add_no_regions_and_equality_ignores_boundaries() {
        let mut data = DispatchData::empty();
        data.append_slice(&[]);
        data.append(&DispatchData::empty());
        assert!(data.is_empty());
        assert_eq!(data.region_count(), 0);

        let joined = two_regions();
        let whole = DispatchData::from_vec(vec![1, 2, 3, 4, 5]);
        assert_eq!(joined, whole);
        assert_ne!(whole, DispatchData::from_slice(&[1, 2, 3, 4]));
    }
}
