//! Disk image model: RAW creation configuration, block size and format values, open mode, block count
//! truncation and the error values. Pure model: no file is written or read here. A storage layer creates the
//! file from `RawCreation::size`, reads its length, and calls `DiskImage::open_raw`. Sizes are whole blocks of
//! 512 bytes or 4 KB; a block count must be positive.
use std::fmt;

/// Bytes in a 512-byte block.
pub const BLOCK_512_BYTES: u64 = 512;
/// Bytes in a 4 KB block.
pub const BLOCK_4K_BYTES: u64 = 4096;

/// The block size of an image: 512 bytes or 4 KB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSize {
    B512,
    K4,
}

impl BlockSize {
    /// Size of one block in bytes.
    pub fn bytes(self) -> u64 {
        match self {
            BlockSize::B512 => BLOCK_512_BYTES,
            BlockSize::K4 => BLOCK_4K_BYTES,
        }
    }

    /// The block size whose byte count is `n`, or `None` for any other size.
    pub fn from_bytes(n: u64) -> Option<BlockSize> {
        match n {
            BLOCK_512_BYTES => Some(BlockSize::B512),
            BLOCK_4K_BYTES => Some(BlockSize::K4),
            _ => None,
        }
    }
}

/// The image formats this model can describe. Only the RAW layout (the bytes of the image as they are) is
/// supported. A sparse format would need its own encoder and is not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Raw,
}

impl Format {
    /// Parses a format name, ignoring ASCII case. `"raw"` is the only name this model accepts. Any other name, such as a
    /// sparse or layered format, is an `UnsupportedFormat` error that keeps the name for the caller to show.
    pub fn parse(name: &str) -> Result<Format, ImageError> {
        if name.eq_ignore_ascii_case("raw") {
            Ok(Format::Raw)
        } else {
            Err(ImageError::UnsupportedFormat { name: name.to_string() })
        }
    }
}

/// Whether an open image may be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// The block count is zero or negative.
    InvalidBlockCount(i64),
    /// The block count times the block size does not fit in 64 bits.
    SizeOverflow,
    /// The image is open read-only and cannot be resized.
    ReadOnly,
    /// The file length is zero or is not a whole number of blocks.
    CorruptedImage { length: u64, block_size: u64 },
    /// The format name is not one this model supports (only RAW is).
    UnsupportedFormat { name: String },
}

impl ImageError {
    /// Name of the official error type this value stands for, or `None` when the framework has no such type. A zero or
    /// negative block count is `InvalidBlockCountError`, a damaged file is `CorruptedImageError`, and an unknown format is
    /// `UnsupportedFormatError`. The size overflow and read-only errors have no official type, so they return `None`.
    /// Stacking errors are not modelled, because layered images are not built.
    pub fn type_name(&self) -> Option<&'static str> {
        match self {
            ImageError::InvalidBlockCount(_) => Some("InvalidBlockCountError"),
            ImageError::CorruptedImage { .. } => Some("CorruptedImageError"),
            ImageError::UnsupportedFormat { .. } => Some("UnsupportedFormatError"),
            ImageError::SizeOverflow | ImageError::ReadOnly => None,
        }
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::InvalidBlockCount(n) => write!(f, "invalid block count {}", n),
            ImageError::SizeOverflow => write!(f, "image size overflows 64 bits"),
            ImageError::ReadOnly => write!(f, "image is read-only"),
            ImageError::CorruptedImage { length, block_size } => {
                write!(f, "image length {} is not a whole number of {}-byte blocks", length, block_size)
            }
            ImageError::UnsupportedFormat { name } => write!(f, "image format {} is not supported", name),
        }
    }
}

/// Parameters for a new, empty RAW image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawCreation {
    block_count: u64,
    block_size: BlockSize,
}

impl RawCreation {
    /// A configuration for `block_count` blocks. Rejects a zero or negative count, and a size that overflows.
    pub fn new(block_count: i64, block_size: BlockSize) -> Result<RawCreation, ImageError> {
        if block_count <= 0 {
            return Err(ImageError::InvalidBlockCount(block_count));
        }
        let count = block_count as u64;
        if count.checked_mul(block_size.bytes()).is_none() {
            return Err(ImageError::SizeOverflow);
        }
        Ok(RawCreation { block_count: count, block_size })
    }

    /// Number of blocks.
    pub fn block_count(&self) -> u64 {
        self.block_count
    }

    /// Block size.
    pub fn block_size(&self) -> BlockSize {
        self.block_size
    }

    /// Logical size of the image in bytes.
    pub fn size(&self) -> u64 {
        self.block_count * self.block_size.bytes()
    }
}

/// An open disk image: its path, format, block size, block count and open mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskImage {
    path: String,
    format: Format,
    block_size: BlockSize,
    block_count: u64,
    open_mode: OpenMode,
}

impl DiskImage {
    /// A new RAW image at `path` from a creation configuration. It opens read-write.
    pub fn create(path: &str, config: RawCreation) -> DiskImage {
        DiskImage {
            path: path.to_string(),
            format: Format::Raw,
            block_size: config.block_size(),
            block_count: config.block_count(),
            open_mode: OpenMode::ReadWrite,
        }
    }

    /// Opens an existing RAW image of `length` bytes. The length must be non-zero and a whole number of blocks.
    pub fn open_raw(path: &str, length: u64, block_size: BlockSize, mode: OpenMode) -> Result<DiskImage, ImageError> {
        let unit = block_size.bytes();
        if length == 0 || length % unit != 0 {
            return Err(ImageError::CorruptedImage { length, block_size: unit });
        }
        Ok(DiskImage {
            path: path.to_string(),
            format: Format::Raw,
            block_size,
            block_count: length / unit,
            open_mode: mode,
        })
    }

    /// Path of the image file.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Format of the image.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Block size of the image.
    pub fn block_size(&self) -> BlockSize {
        self.block_size
    }

    /// Number of blocks.
    pub fn block_count(&self) -> u64 {
        self.block_count
    }

    /// Logical size of the image in bytes.
    pub fn size(&self) -> u64 {
        self.block_count * self.block_size.bytes()
    }

    /// Whether the image is read-only or read-write.
    pub fn open_mode(&self) -> OpenMode {
        self.open_mode
    }

    /// Sets the block count to `new_count`, growing or shrinking the image. The filesystem inside the image is
    /// not changed. Rejects a zero or negative count, a size overflow, and any change to a read-only image.
    pub fn truncate(&mut self, new_count: i64) -> Result<(), ImageError> {
        if self.open_mode == OpenMode::ReadOnly {
            return Err(ImageError::ReadOnly);
        }
        let config = RawCreation::new(new_count, self.block_size)?;
        self.block_count = config.block_count();
        Ok(())
    }
}

/// Byte offset of the ISO 9660 primary volume descriptor: the system area is 16 sectors of 2048 bytes.
const ISO_PVD_OFFSET: usize = 16 * 2048;
/// Bytes of the primary volume descriptor that are read: up to the logical block size field at offset 128.
const ISO_PVD_READ: usize = 132;

/// The contents recognised from an image's first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageContents {
    /// No volume descriptor found: the bytes are taken as they are.
    Raw,
    /// An ISO 9660 volume (the optical disc layout).
    Iso9660,
}

/// Properties read from an image file by `inspect`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageProperties {
    /// Length of the image file in bytes.
    pub length: u64,
    /// The recognised contents.
    pub contents: ImageContents,
    /// Volume identifier from the descriptor, trailing spaces removed. `None` for raw images or an empty name.
    pub volume_name: Option<String>,
    /// Block count the descriptor declares.
    pub declared_blocks: Option<u32>,
    /// Logical block size the descriptor declares, in bytes.
    pub declared_block_size: Option<u16>,
}

impl ImageProperties {
    /// The size the descriptor declares (blocks times block size), when it declares both.
    pub fn declared_size(&self) -> Option<u64> {
        Some(u64::from(self.declared_blocks?) * u64::from(self.declared_block_size?))
    }

    /// False when the file is shorter than the descriptor says the volume is. True when there is no declared size.
    pub fn is_complete(&self) -> bool {
        match self.declared_size() {
            Some(size) => self.length >= size,
            None => true,
        }
    }
}

fn raw_properties(length: u64) -> ImageProperties {
    ImageProperties {
        length,
        contents: ImageContents::Raw,
        volume_name: None,
        declared_blocks: None,
        declared_block_size: None,
    }
}

/// Reads the properties of an image from its first bytes and its total length. Pure: the caller reads the head of
/// the file (at least 2 KB past the 32 KB system area to see the whole descriptor) and passes the file length.
/// A head that is too short, or has no ISO 9660 descriptor, is reported as raw.
pub fn inspect(head: &[u8], length: u64) -> ImageProperties {
    let pvd = match head.get(ISO_PVD_OFFSET..ISO_PVD_OFFSET + ISO_PVD_READ) {
        Some(p) => p,
        None => return raw_properties(length),
    };
    // Descriptor type 1 (primary), identifier "CD001", version 1.
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" || pvd[6] != 1 {
        return raw_properties(length);
    }
    let blocks = u32::from_le_bytes([pvd[80], pvd[81], pvd[82], pvd[83]]);
    let block_size = u16::from_le_bytes([pvd[128], pvd[129]]);
    let name = String::from_utf8_lossy(&pvd[40..72]).trim_end_matches(|c: char| c == ' ' || c == '\0').to_string();
    ImageProperties {
        length,
        contents: ImageContents::Iso9660,
        volume_name: if name.is_empty() { None } else { Some(name) },
        declared_blocks: Some(blocks),
        declared_block_size: Some(block_size),
    }
}

#[cfg(test)]
mod inspect_tests {
    use super::*;

    /// A head of one system area plus one descriptor sector, with the given name, block count and block size.
    fn iso_head(name: &[u8], blocks: u32, block_size: u16) -> Vec<u8> {
        let mut h = vec![0u8; ISO_PVD_OFFSET + 2048];
        h[ISO_PVD_OFFSET] = 1;
        h[ISO_PVD_OFFSET + 1..ISO_PVD_OFFSET + 6].copy_from_slice(b"CD001");
        h[ISO_PVD_OFFSET + 6] = 1;
        for b in &mut h[ISO_PVD_OFFSET + 40..ISO_PVD_OFFSET + 72] {
            *b = b' ';
        }
        h[ISO_PVD_OFFSET + 40..ISO_PVD_OFFSET + 40 + name.len()].copy_from_slice(name);
        h[ISO_PVD_OFFSET + 80..ISO_PVD_OFFSET + 84].copy_from_slice(&blocks.to_le_bytes());
        h[ISO_PVD_OFFSET + 128..ISO_PVD_OFFSET + 130].copy_from_slice(&block_size.to_le_bytes());
        h
    }

    #[test]
    fn iso_descriptor_gives_name_and_declared_size() {
        // 1000 blocks of 2048 bytes declare 2_048_000 bytes.
        let head = iso_head(b"BIFROST", 1000, 2048);
        let p = inspect(&head, 2_048_000);
        assert_eq!(p.contents, ImageContents::Iso9660);
        assert_eq!(p.volume_name.as_deref(), Some("BIFROST"));
        assert_eq!(p.declared_blocks, Some(1000));
        assert_eq!(p.declared_block_size, Some(2048));
        assert_eq!(p.declared_size(), Some(2_048_000));
        assert!(p.is_complete());
    }

    #[test]
    fn a_file_shorter_than_the_declared_volume_is_incomplete() {
        // Half of the declared 2_048_000 bytes is present.
        let head = iso_head(b"BIFROST", 1000, 2048);
        let p = inspect(&head, 1_024_000);
        assert!(!p.is_complete());
        assert_eq!(p.length, 1_024_000);
    }

    #[test]
    fn a_blank_name_is_none() {
        // The name field is all spaces, so the trimmed name is empty.
        let head = iso_head(b"", 10, 2048);
        assert_eq!(inspect(&head, 20480).volume_name, None);
    }

    #[test]
    fn a_head_without_the_identifier_is_raw() {
        let head = vec![0u8; ISO_PVD_OFFSET + 2048];
        let p = inspect(&head, 500);
        assert_eq!(p.contents, ImageContents::Raw);
        assert_eq!(p.volume_name, None);
        assert_eq!(p.declared_size(), None);
        assert!(p.is_complete());
    }

    #[test]
    fn a_short_head_is_raw_without_panicking() {
        // Only 100 bytes: the descriptor is not in the head, so the image is read as raw.
        let p = inspect(&[1u8; 100], 100);
        assert_eq!(p.contents, ImageContents::Raw);
        assert_eq!(p.length, 100);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_sizes_map_to_bytes_and_back() {
        assert_eq!(BlockSize::B512.bytes(), 512);
        assert_eq!(BlockSize::K4.bytes(), 4096);
        assert_eq!(BlockSize::from_bytes(4096), Some(BlockSize::K4));
        assert_eq!(BlockSize::from_bytes(1024), None);
    }

    #[test]
    fn creation_rejects_zero_and_negative_block_counts() {
        assert_eq!(RawCreation::new(0, BlockSize::B512), Err(ImageError::InvalidBlockCount(0)));
        assert_eq!(RawCreation::new(-3, BlockSize::K4), Err(ImageError::InvalidBlockCount(-3)));
    }

    #[test]
    fn creation_size_is_blocks_times_block_size() {
        let a = RawCreation::new(2048, BlockSize::B512).unwrap();
        assert_eq!(a.size(), 1_048_576);
        let b = RawCreation::new(100, BlockSize::K4).unwrap();
        assert_eq!(b.size(), 409_600);
        assert_eq!(b.block_count(), 100);
    }

    #[test]
    fn creation_rejects_size_overflow() {
        // i64::MAX blocks of 4 KB do not fit in 64 bits.
        assert_eq!(RawCreation::new(i64::MAX, BlockSize::K4), Err(ImageError::SizeOverflow));
    }

    #[test]
    fn create_gives_a_read_write_raw_image() {
        let cfg = RawCreation::new(8, BlockSize::B512).unwrap();
        let img = DiskImage::create("/tmp/x.img", cfg);
        assert_eq!(img.format(), Format::Raw);
        assert_eq!(img.open_mode(), OpenMode::ReadWrite);
        assert_eq!(img.size(), 4096);
        assert_eq!(img.path(), "/tmp/x.img");
    }

    #[test]
    fn open_counts_blocks_from_length_and_rejects_partial_blocks() {
        let img = DiskImage::open_raw("a.iso", 1_048_576, BlockSize::B512, OpenMode::ReadOnly).unwrap();
        assert_eq!(img.block_count(), 2048);
        assert_eq!(img.open_mode(), OpenMode::ReadOnly);
        assert_eq!(
            DiskImage::open_raw("a.iso", 1000, BlockSize::B512, OpenMode::ReadOnly),
            Err(ImageError::CorruptedImage { length: 1000, block_size: 512 })
        );
        assert!(DiskImage::open_raw("a.iso", 0, BlockSize::K4, OpenMode::ReadOnly).is_err());
    }

    #[test]
    fn truncate_grows_and_shrinks_block_count() {
        let mut img = DiskImage::open_raw("d.img", 1_048_576, BlockSize::B512, OpenMode::ReadWrite).unwrap();
        img.truncate(4096).unwrap();
        assert_eq!(img.size(), 2_097_152);
        img.truncate(1).unwrap();
        assert_eq!(img.block_count(), 1);
        assert_eq!(img.truncate(0), Err(ImageError::InvalidBlockCount(0)));
        assert_eq!(img.block_count(), 1);
    }

    #[test]
    fn truncate_refuses_read_only_images_and_leaves_them_unchanged() {
        let mut img = DiskImage::open_raw("r.iso", 4096, BlockSize::K4, OpenMode::ReadOnly).unwrap();
        assert_eq!(img.truncate(8), Err(ImageError::ReadOnly));
        assert_eq!(img.size(), 4096);
    }

    #[test]
    fn error_messages_name_the_value() {
        assert_eq!(ImageError::InvalidBlockCount(-3).to_string(), "invalid block count -3");
        assert_eq!(
            ImageError::CorruptedImage { length: 1000, block_size: 512 }.to_string(),
            "image length 1000 is not a whole number of 512-byte blocks"
        );
    }

    #[test]
    fn format_parse_accepts_raw_in_any_case_and_names_anything_else() {
        assert_eq!(Format::parse("RAW"), Ok(Format::Raw));
        assert_eq!(Format::parse("Raw"), Ok(Format::Raw));
        assert_eq!(
            Format::parse("asif"),
            Err(ImageError::UnsupportedFormat { name: "asif".to_string() })
        );
        assert_eq!(
            ImageError::UnsupportedFormat { name: "asif".to_string() }.to_string(),
            "image format asif is not supported"
        );
    }

    #[test]
    fn invalid_block_count_maps_to_its_official_error_type() {
        // Zero blocks at creation, and a negative count on truncate, are both InvalidBlockCountError.
        let zero = RawCreation::new(0, BlockSize::B512).unwrap_err();
        assert_eq!(zero.type_name(), Some("InvalidBlockCountError"));
        let mut img = DiskImage::create("x.img", RawCreation::new(4, BlockSize::B512).unwrap());
        let negative = img.truncate(-1).unwrap_err();
        assert_eq!(negative, ImageError::InvalidBlockCount(-1));
        assert_eq!(negative.type_name(), Some("InvalidBlockCountError"));
    }

    #[test]
    fn corrupted_and_unsupported_values_map_to_their_error_types() {
        // A 1000-byte file is not a whole number of 512-byte blocks: CorruptedImageError.
        let corrupt = DiskImage::open_raw("a.iso", 1000, BlockSize::B512, OpenMode::ReadOnly).unwrap_err();
        assert_eq!(corrupt.type_name(), Some("CorruptedImageError"));
        let unsupported = Format::parse("qcow2").unwrap_err();
        assert_eq!(unsupported.type_name(), Some("UnsupportedFormatError"));
    }

    #[test]
    fn errors_without_an_official_type_return_none() {
        // Overflow and read-only refusals are not in the framework's error list.
        assert_eq!(RawCreation::new(i64::MAX, BlockSize::K4).unwrap_err().type_name(), None);
        assert_eq!(ImageError::ReadOnly.type_name(), None);
    }

    #[test]
    fn corrupted_image_size_is_the_length_is_checked_against_each_block_size() {
        // 4096 is one whole 4 KB block. 4608 = 4096 + 512 leaves a 512-byte partial block, so it is corrupted.
        assert!(DiskImage::open_raw("c.img", 4096, BlockSize::K4, OpenMode::ReadOnly).is_ok());
        assert_eq!(
            DiskImage::open_raw("c.img", 4096 + 512, BlockSize::K4, OpenMode::ReadOnly),
            Err(ImageError::CorruptedImage { length: 4608, block_size: 4096 })
        );
    }
}
