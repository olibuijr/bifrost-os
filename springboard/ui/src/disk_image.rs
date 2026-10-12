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
