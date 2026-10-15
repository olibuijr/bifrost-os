//! Extent maps: which byte ranges of a file are stored where on the volume.
//!
//! The kernel file systems allocate and read the blocks. This model describes the mapping an application can inspect:
//! a list of extents, each a run of file bytes that is either data at a disk address, unwritten space reserved at a disk
//! address, or a hole with no storage. It follows the packing rules of the reference extent packer. Extents are kept in
//! file order, extents of one kind that are contiguous in the file and on disk merge into one, and overlapping extents
//! are refused. A lookup maps a file offset to the disk offset that holds it.

/// What an extent holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtentKind {
    /// Written data stored at a disk address.
    Data,
    /// Space reserved at a disk address that reads as zeros until it is written.
    Unwritten,
    /// A run of zeros with no storage behind it.
    Hole,
}

/// A run of file bytes and where it is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// The first byte of the run in the file.
    pub file_offset: u64,
    /// The number of bytes in the run.
    pub length: u64,
    pub kind: ExtentKind,
    /// The disk address of the first byte. Data and unwritten extents need one; a hole has none.
    pub disk_offset: Option<u64>,
}

/// Where a file offset is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub kind: ExtentKind,
    /// The disk address of the byte, or None when it lies in a hole.
    pub disk_offset: Option<u64>,
}

/// Why an extent list was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtentError {
    /// An extent has no bytes.
    ZeroLength,
    /// A hole has a disk address, or a data or unwritten extent has none.
    BadDiskAddress,
    /// Two extents share bytes. The value is the file offset where the second one starts.
    Overlap(u64),
    /// An extent runs past the largest offset.
    Overflow,
}

impl std::fmt::Display for ExtentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtentError::ZeroLength => write!(f, "an extent must cover at least one byte"),
            ExtentError::BadDiskAddress => write!(f, "a hole has no disk address and a data extent needs one"),
            ExtentError::Overlap(at) => write!(f, "extents overlap at file offset {at}"),
            ExtentError::Overflow => write!(f, "an extent runs past the largest offset"),
        }
    }
}

impl std::error::Error for ExtentError {}

/// A checked, packed and sorted list of extents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtentMap {
    extents: Vec<Extent>,
}

impl ExtentMap {
    /// Checks the extents, sorts them by file offset and merges the runs that continue each other. The input may be in
    /// any order.
    pub fn new(mut extents: Vec<Extent>) -> Result<ExtentMap, ExtentError> {
        extents.sort_by_key(|e| e.file_offset);
        let mut packed: Vec<Extent> = Vec::with_capacity(extents.len());
        for extent in extents {
            check(&extent)?;
            if let Some(last) = packed.last_mut() {
                if extent.file_offset < last.file_offset + last.length {
                    return Err(ExtentError::Overlap(extent.file_offset));
                }
                if continues(last, &extent) {
                    last.length += extent.length;
                    continue;
                }
            }
            packed.push(extent);
        }
        Ok(ExtentMap { extents: packed })
    }

    /// The packed extents, in file order.
    pub fn extents(&self) -> &[Extent] {
        &self.extents
    }

    /// The file offset just past the last extent, or zero for an empty map.
    pub fn end(&self) -> u64 {
        self.extents.last().map_or(0, |e| e.file_offset + e.length)
    }

    /// Where the byte at `offset` is stored, or None when `offset` is at or past the end of the map.
    pub fn locate(&self, offset: u64) -> Option<Location> {
        let index = self.extents.partition_point(|e| e.file_offset <= offset);
        let extent = self.extents.get(index.checked_sub(1)?)?;
        if offset >= extent.file_offset + extent.length {
            return None;
        }
        let within = offset - extent.file_offset;
        Some(Location { kind: extent.kind, disk_offset: extent.disk_offset.map(|disk| disk + within) })
    }
}

fn check(extent: &Extent) -> Result<(), ExtentError> {
    if extent.length == 0 {
        return Err(ExtentError::ZeroLength);
    }
    extent.file_offset.checked_add(extent.length).ok_or(ExtentError::Overflow)?;
    match (extent.kind, extent.disk_offset) {
        (ExtentKind::Hole, None) => Ok(()),
        (ExtentKind::Hole, Some(_)) | (_, None) => Err(ExtentError::BadDiskAddress),
        (_, Some(disk)) => {
            disk.checked_add(extent.length).ok_or(ExtentError::Overflow)?;
            Ok(())
        }
    }
}

/// True when `next` starts where `last` ends, in the file and, for storage, on the disk, and has the same kind.
fn continues(last: &Extent, next: &Extent) -> bool {
    if last.kind != next.kind || last.file_offset + last.length != next.file_offset {
        return false;
    }
    match (last.disk_offset, next.disk_offset) {
        (Some(disk), Some(next_disk)) => disk + last.length == next_disk,
        (None, None) => true,
        _ => false,
    }
}
