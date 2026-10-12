//! Disk image model: RAW image geometry, create and open checks, and truncate or extend.
//!
//! Pure: no file is created, opened, resized or mounted here. The caller reads and writes the bytes itself, and this
//! module decides which sizes are legal. The operations mirror the create, open, truncate and extend steps described in
//! the official disk image framework documentation. Only the RAW format is modelled. Layered and sparse formats are not
//! built, and the parser says so instead of guessing.

/// Image formats this build can describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Raw,
}

/// Size of one block in bytes. The image is always a whole number of blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSize {
    Bytes512,
    Bytes4096,
}

impl BlockSize {
    pub fn bytes(self) -> u64 {
        match self {
            BlockSize::Bytes512 => 512,
            BlockSize::Bytes4096 => 4096,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// The byte count is zero, or it is not a whole number of blocks.
    InvalidBlockCount,
    /// The format name is not one this build supports.
    UnsupportedFormat,
    /// The image was opened read-only and cannot be resized.
    ReadOnly,
}

/// The properties a caller can read from an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageProperties {
    pub format: ImageFormat,
    pub block_size: BlockSize,
    pub block_count: u64,
    pub read_only: bool,
}

impl ImageProperties {
    /// The image size in bytes. It cannot overflow, because `block_count` was derived from a byte count that fit in u64.
    pub fn byte_size(&self) -> u64 {
        self.block_count * self.block_size.bytes()
    }
}

/// Parses a format name, ignoring ASCII case. `"raw"` is the only supported name.
pub fn parse_format(name: &str) -> Result<ImageFormat, ImageError> {
    if name.eq_ignore_ascii_case("raw") {
        Ok(ImageFormat::Raw)
    } else {
        Err(ImageError::UnsupportedFormat)
    }
}

/// Checks a byte count against a block size and returns the block count. Zero bytes and partial blocks are rejected.
fn block_count_for(byte_size: u64, block_size: BlockSize) -> Result<u64, ImageError> {
    let block = block_size.bytes();
    if byte_size == 0 || byte_size % block != 0 {
        return Err(ImageError::InvalidBlockCount);
    }
    Ok(byte_size / block)
}

/// Describes a new writable RAW image of `byte_size` bytes.
pub fn create_raw(byte_size: u64, block_size: BlockSize) -> Result<ImageProperties, ImageError> {
    let block_count = block_count_for(byte_size, block_size)?;
    Ok(ImageProperties {
        format: ImageFormat::Raw,
        block_size,
        block_count,
        read_only: false,
    })
}

/// Describes an existing RAW image whose file is `byte_len` bytes long.
pub fn open_raw(byte_len: u64, block_size: BlockSize, read_only: bool) -> Result<ImageProperties, ImageError> {
    let block_count = block_count_for(byte_len, block_size)?;
    Ok(ImageProperties {
        format: ImageFormat::Raw,
        block_size,
        block_count,
        read_only,
    })
}

impl ImageProperties {
    /// Resizes the image to `new_byte_size` bytes, growing or shrinking it. A read-only image is refused before the size
    /// is checked. The filesystem inside the image is not touched, so the caller must grow it separately.
    pub fn truncate(&mut self, new_byte_size: u64) -> Result<(), ImageError> {
        if self.read_only {
            return Err(ImageError::ReadOnly);
        }
        self.block_count = block_count_for(new_byte_size, self.block_size)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_counts_whole_blocks() {
        let img = create_raw(10 * 512, BlockSize::Bytes512).unwrap();
        assert_eq!(img.block_count, 10);
        assert_eq!(img.byte_size(), 5120);
        assert!(!img.read_only);

        let big = create_raw(8192, BlockSize::Bytes4096).unwrap();
        assert_eq!(big.block_count, 2);
        assert_eq!(big.byte_size(), 8192);
    }

    #[test]
    fn create_rejects_zero_and_partial_blocks() {
        assert_eq!(create_raw(0, BlockSize::Bytes512), Err(ImageError::InvalidBlockCount));
        // 1000 = 512 + 488, so the last block would be partial.
        assert_eq!(create_raw(1000, BlockSize::Bytes512), Err(ImageError::InvalidBlockCount));
        // 4096 is a whole 4 KiB block, but 2048 is not.
        assert_eq!(create_raw(2048, BlockSize::Bytes4096), Err(ImageError::InvalidBlockCount));
    }

    #[test]
    fn open_keeps_the_read_only_flag() {
        let ro = open_raw(1024, BlockSize::Bytes512, true).unwrap();
        assert_eq!(ro.block_count, 2);
        assert!(ro.read_only);
        assert_eq!(open_raw(1000, BlockSize::Bytes512, false), Err(ImageError::InvalidBlockCount));
    }

    #[test]
    fn parse_format_accepts_only_raw() {
        assert_eq!(parse_format("RAW"), Ok(ImageFormat::Raw));
        assert_eq!(parse_format("raw"), Ok(ImageFormat::Raw));
        assert_eq!(parse_format("qcow2"), Err(ImageError::UnsupportedFormat));
        assert_eq!(parse_format("asif"), Err(ImageError::UnsupportedFormat));
    }

    #[test]
    fn truncate_grows_and_shrinks_by_blocks() {
        let mut img = create_raw(4 * 512, BlockSize::Bytes512).unwrap();
        img.truncate(8 * 512).unwrap();
        assert_eq!(img.block_count, 8);
        assert_eq!(img.byte_size(), 4096);

        img.truncate(2 * 512).unwrap();
        assert_eq!(img.block_count, 2);
        assert_eq!(img.byte_size(), 1024);
    }

    #[test]
    fn truncate_refuses_read_only_and_bad_sizes_without_changing_the_image() {
        let mut ro = open_raw(2048, BlockSize::Bytes512, true).unwrap();
        assert_eq!(ro.truncate(4096), Err(ImageError::ReadOnly));
        assert_eq!(ro.block_count, 4);

        let mut rw = create_raw(2048, BlockSize::Bytes512).unwrap();
        assert_eq!(rw.truncate(100), Err(ImageError::InvalidBlockCount));
        assert_eq!(rw.truncate(0), Err(ImageError::InvalidBlockCount));
        assert_eq!(rw.block_count, 4);
    }
}
