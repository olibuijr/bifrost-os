//! Disk image model, shared by the shell, the `hdiutil` tool, Files and Disk Utility.
//!
//! Pure: no file is created, opened, resized or mounted here. The caller reads and writes the bytes itself, and this
//! module decides which sizes are legal, what an image's first bytes say, and how a size string reads. Only the RAW
//! layout (the bytes as they are) is modelled. Sparse and layered formats are not built, and the parser says so instead
//! of guessing. Sizes are whole blocks of 512 bytes or 4 KB; a block count must be positive.

use std::fmt;

/// Size of one block in bytes, as the image is laid out.
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

    /// The block size whose byte count is `n`, or `None` for any other size.
    pub fn from_bytes(n: u64) -> Option<BlockSize> {
        match n {
            512 => Some(BlockSize::Bytes512),
            4096 => Some(BlockSize::Bytes4096),
            _ => None,
        }
    }
}

/// Image formats this build can describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Raw,
}

/// Parses a format name, ignoring ASCII case. `"raw"` is the only supported name.
pub fn parse_format(name: &str) -> Result<ImageFormat, ImageError> {
    if name.eq_ignore_ascii_case("raw") {
        Ok(ImageFormat::Raw)
    } else {
        Err(ImageError::UnsupportedFormat { name: name.to_string() })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// The byte count is zero, or it is not a whole number of blocks.
    InvalidBlockCount,
    /// The format name is not one this build supports (only RAW is).
    UnsupportedFormat { name: String },
    /// The image was opened read-only and cannot be resized.
    ReadOnly,
    /// The file is shorter than the volume its descriptor declares.
    Truncated { length: u64, declared: u64 },
    /// A size string is not a number with an optional k, m, g or t suffix.
    BadSize(String),
}

impl ImageError {
    /// The name of the official error type this value stands for, or `None` when there is none.
    pub fn type_name(&self) -> Option<&'static str> {
        match self {
            ImageError::InvalidBlockCount => Some("InvalidBlockCountError"),
            ImageError::UnsupportedFormat { .. } => Some("UnsupportedFormatError"),
            ImageError::Truncated { .. } => Some("CorruptedImageError"),
            ImageError::ReadOnly | ImageError::BadSize(_) => None,
        }
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::InvalidBlockCount => write!(f, "the size is zero or not a whole number of blocks"),
            ImageError::UnsupportedFormat { name } => write!(f, "image format {} is not supported", name),
            ImageError::ReadOnly => write!(f, "image is read-only"),
            ImageError::Truncated { length, declared } => {
                write!(f, "image is {} bytes but its volume declares {} bytes", length, declared)
            }
            ImageError::BadSize(text) => write!(f, "size {} is not a number with an optional k, m, g or t", text),
        }
    }
}

/// Checks a byte count against a block size and returns the block count. Zero bytes and partial blocks are rejected.
fn block_count_for(byte_size: u64, block_size: BlockSize) -> Result<u64, ImageError> {
    let block = block_size.bytes();
    if byte_size == 0 || !byte_size.is_multiple_of(block) {
        return Err(ImageError::InvalidBlockCount);
    }
    Ok(byte_size / block)
}

/// The properties of a RAW image: its format, block size, block count and whether it may be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageProperties {
    pub format: ImageFormat,
    pub block_size: BlockSize,
    pub block_count: u64,
    pub read_only: bool,
}

impl ImageProperties {
    /// The image size in bytes. It cannot overflow, because `block_count` came from a byte count that fit in u64.
    pub fn byte_size(&self) -> u64 {
        self.block_count * self.block_size.bytes()
    }

    /// Resizes the image to `new_byte_size` bytes, growing or shrinking it. A read-only image is refused before the size
    /// is checked. The filesystem inside the image is not touched, so the caller must grow it separately.
    pub fn truncate(&mut self, new_byte_size: u64) -> Result<(), ImageError> {
        if self.read_only {
            return Err(ImageError::ReadOnly);
        }
        self.block_count = block_count_for(new_byte_size, self.block_size)?;
        Ok(())
    }

    /// The same bytes described with another block size (a block-size conversion). The byte size must be a whole
    /// number of the new blocks.
    pub fn reblock(&self, to: BlockSize) -> Result<ImageProperties, ImageError> {
        let block_count = block_count_for(self.byte_size(), to)?;
        Ok(ImageProperties { format: self.format, block_size: to, block_count, read_only: self.read_only })
    }
}

/// Describes a new writable RAW image of `byte_size` bytes.
pub fn create_raw(byte_size: u64, block_size: BlockSize) -> Result<ImageProperties, ImageError> {
    let block_count = block_count_for(byte_size, block_size)?;
    Ok(ImageProperties { format: ImageFormat::Raw, block_size, block_count, read_only: false })
}

/// Describes an existing RAW image whose file is `byte_len` bytes long.
pub fn open_raw(byte_len: u64, block_size: BlockSize, read_only: bool) -> Result<ImageProperties, ImageError> {
    let block_count = block_count_for(byte_len, block_size)?;
    Ok(ImageProperties { format: ImageFormat::Raw, block_size, block_count, read_only })
}

/// Parses a size such as `512`, `10m`, `2G` or `1t` into bytes. A bare number is bytes; k, m, g and t are binary
/// multiples (1024 bytes, 1024 KB, and so on). Zero, an overflow and anything else are refused.
pub fn parse_size(text: &str) -> Result<u64, ImageError> {
    let bad = || ImageError::BadSize(text.to_string());
    let t = text.trim();
    let (digits, multiplier) = match t.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') => (&t[..t.len() - 1], 1u64 << 10),
        Some('m') => (&t[..t.len() - 1], 1u64 << 20),
        Some('g') => (&t[..t.len() - 1], 1u64 << 30),
        Some('t') => (&t[..t.len() - 1], 1u64 << 40),
        _ => (t, 1),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let n: u64 = digits.parse().map_err(|_| bad())?;
    if n == 0 {
        return Err(ImageError::InvalidBlockCount);
    }
    n.checked_mul(multiplier).ok_or_else(bad)
}

/// The contents recognised from an image's first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageContents {
    /// No volume descriptor found: the bytes are taken as they are.
    Raw,
    /// An ISO 9660 volume (the optical disc layout).
    Iso9660,
}

/// Byte offset of the ISO 9660 primary volume descriptor: the system area is 16 sectors of 2048 bytes.
const ISO_PVD_OFFSET: usize = 16 * 2048;
/// Bytes of the primary volume descriptor that are read: up to the logical block size field at offset 128.
const ISO_PVD_READ: usize = 132;

/// What `inspect` reads from an image's first bytes and its length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
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

impl Inspection {
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

fn raw_inspection(length: u64) -> Inspection {
    Inspection { length, contents: ImageContents::Raw, volume_name: None, declared_blocks: None, declared_block_size: None }
}

/// Reads the properties of an image from its first bytes and its total length. Pure: the caller reads the head of
/// the file (at least 34 KB, the system area plus the descriptor) and passes the file length. A head that is too short,
/// or has no ISO 9660 descriptor, is reported as raw.
pub fn inspect(head: &[u8], length: u64) -> Inspection {
    let pvd = match head.get(ISO_PVD_OFFSET..ISO_PVD_OFFSET + ISO_PVD_READ) {
        Some(p) => p,
        None => return raw_inspection(length),
    };
    // Descriptor type 1 (primary), identifier "CD001", version 1.
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" || pvd[6] != 1 {
        return raw_inspection(length);
    }
    let blocks = u32::from_le_bytes([pvd[80], pvd[81], pvd[82], pvd[83]]);
    let block_size = u16::from_le_bytes([pvd[128], pvd[129]]);
    let name = String::from_utf8_lossy(&pvd[40..72]).trim_end_matches([' ', '\0']).to_string();
    Inspection {
        length,
        contents: ImageContents::Iso9660,
        volume_name: if name.is_empty() { None } else { Some(name) },
        declared_blocks: Some(blocks),
        declared_block_size: Some(block_size),
    }
}

/// Checks an image's first bytes and length: `Ok` with what was read when the file holds the whole volume its
/// descriptor declares (or when it has no descriptor), `Truncated` otherwise.
pub fn verify(head: &[u8], length: u64) -> Result<Inspection, ImageError> {
    let info = inspect(head, length);
    match info.declared_size() {
        Some(declared) if !info.is_complete() => Err(ImageError::Truncated { length, declared }),
        _ => Ok(info),
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
    fn a_file_shorter_than_the_declared_volume_is_incomplete_and_fails_verify() {
        let head = iso_head(b"BIFROST", 1000, 2048);
        let p = inspect(&head, 1_024_000);
        assert!(!p.is_complete());
        assert_eq!(p.length, 1_024_000);
        assert_eq!(verify(&head, 1_024_000), Err(ImageError::Truncated { length: 1_024_000, declared: 2_048_000 }));
        assert_eq!(ImageError::Truncated { length: 1, declared: 2 }.type_name(), Some("CorruptedImageError"));
    }

    #[test]
    fn verify_passes_a_complete_iso_and_a_raw_image() {
        let head = iso_head(b"X", 10, 2048);
        assert!(verify(&head, 20_480).is_ok());
        let raw = vec![0u8; ISO_PVD_OFFSET + 2048];
        assert_eq!(verify(&raw, 500).unwrap().contents, ImageContents::Raw);
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
        assert_eq!(BlockSize::Bytes512.bytes(), 512);
        assert_eq!(BlockSize::Bytes4096.bytes(), 4096);
        assert_eq!(BlockSize::from_bytes(4096), Some(BlockSize::Bytes4096));
        assert_eq!(BlockSize::from_bytes(1024), None);
    }

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
        assert_eq!(parse_format("qcow2"), Err(ImageError::UnsupportedFormat { name: "qcow2".into() }));
        assert_eq!(parse_format("asif").unwrap_err().type_name(), Some("UnsupportedFormatError"));
        assert_eq!(ImageError::UnsupportedFormat { name: "asif".into() }.to_string(), "image format asif is not supported");
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

    #[test]
    fn reblock_keeps_the_bytes_and_needs_whole_new_blocks() {
        let img = create_raw(8192, BlockSize::Bytes512).unwrap();
        let big = img.reblock(BlockSize::Bytes4096).unwrap();
        assert_eq!(big.block_count, 2);
        assert_eq!(big.byte_size(), 8192);
        let odd = create_raw(512 * 3, BlockSize::Bytes512).unwrap();
        assert_eq!(odd.reblock(BlockSize::Bytes4096), Err(ImageError::InvalidBlockCount));
    }

    #[test]
    fn parse_size_reads_plain_bytes_and_binary_suffixes() {
        assert_eq!(parse_size("512"), Ok(512));
        assert_eq!(parse_size("10m"), Ok(10 << 20));
        assert_eq!(parse_size("2G"), Ok(2 << 30));
        assert_eq!(parse_size("1t"), Ok(1 << 40));
        assert_eq!(parse_size("4k"), Ok(4096));
    }

    #[test]
    fn parse_size_refuses_zero_garbage_and_overflow() {
        assert_eq!(parse_size("0"), Err(ImageError::InvalidBlockCount));
        assert_eq!(parse_size(""), Err(ImageError::BadSize(String::new())));
        assert_eq!(parse_size("m"), Err(ImageError::BadSize("m".into())));
        assert_eq!(parse_size("1.5g"), Err(ImageError::BadSize("1.5g".into())));
        assert!(parse_size("99999999999t").is_err());
        assert!(parse_size("18446744073709551615k").is_err());
    }

    #[test]
    fn error_messages_name_the_value() {
        assert_eq!(
            ImageError::Truncated { length: 10, declared: 20 }.to_string(),
            "image is 10 bytes but its volume declares 20 bytes"
        );
        assert_eq!(ImageError::InvalidBlockCount.type_name(), Some("InvalidBlockCountError"));
        assert_eq!(ImageError::ReadOnly.type_name(), None);
    }
}
