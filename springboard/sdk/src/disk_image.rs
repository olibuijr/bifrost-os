//! Disk images for apps: create, open, resize and inspect RAW image files, the part of DiskImageKit an app can use
//! without a mount daemon.
//!
//! The rules (legal sizes, block sizes, the ISO 9660 descriptor check) live in [`bifrost_core::disk_image`]. This module
//! does the file work on top of them: a new image is a file of the exact size, created with `create_new` so an existing
//! file is never overwritten, and a resize sets the file length. Nothing here mounts an image or talks to a volume
//! service: mounting needs a system daemon, which BifrOSt does not have yet, so the mount and eject parts of the
//! framework are still gaps.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

pub use bifrost_core::disk_image::{BlockSize, ImageContents, ImageError, ImageFormat, ImageProperties, Inspection};

/// Bytes read from the head of an image to look for its ISO 9660 descriptor: the system area plus the primary volume
/// descriptor, the 34 KB the core `inspect` needs.
const HEAD_BYTES: u64 = 34 * 1024;

fn invalid(error: ImageError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

/// Creates a new writable RAW image file of `byte_size` bytes, sparse where the filesystem allows it. Fails with
/// `AlreadyExists` when the path is taken, and with `InvalidInput` when the size is not a whole number of blocks.
pub fn create_raw_file(path: &Path, byte_size: u64, block_size: BlockSize) -> io::Result<ImageProperties> {
    let props = bifrost_core::disk_image::create_raw(byte_size, block_size).map_err(invalid)?;
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.set_len(props.byte_size())?;
    Ok(props)
}

/// Describes an existing RAW image file. A read-only open never changes the file; a read-write open needs write
/// permission on it.
pub fn open_raw_file(path: &Path, block_size: BlockSize, read_only: bool) -> io::Result<ImageProperties> {
    let file = OpenOptions::new().read(true).write(!read_only).open(path)?;
    let length = file.metadata()?.len();
    bifrost_core::disk_image::open_raw(length, block_size, read_only).map_err(invalid)
}

/// Grows or shrinks a writable RAW image file to `new_byte_size` bytes and returns its new properties. The filesystem
/// inside the image is not touched, so the caller grows that separately.
pub fn resize_raw_file(path: &Path, new_byte_size: u64, block_size: BlockSize) -> io::Result<ImageProperties> {
    let mut props = open_raw_file(path, block_size, false)?;
    props.truncate(new_byte_size).map_err(invalid)?;
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(props.byte_size())?;
    Ok(props)
}

/// Reads what an image file is from its first bytes and its length: RAW, or an ISO 9660 volume with the blocks and
/// block size its descriptor declares. Reads at most the first 34 KB of the file.
pub fn inspect_file(path: &Path) -> io::Result<Inspection> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut head = Vec::new();
    file.by_ref().take(HEAD_BYTES).read_to_end(&mut head)?;
    Ok(bifrost_core::disk_image::inspect(&head, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-disk-image-{}-{}", name, std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("image.raw")
    }

    #[test]
    fn create_writes_a_file_of_the_exact_size_and_refuses_to_overwrite() {
        let path = scratch("create");
        let _ = std::fs::remove_file(&path);
        let props = create_raw_file(&path, 10 * 512, BlockSize::Bytes512).unwrap();
        assert_eq!(props.block_count, 10);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 5120);
        let again = create_raw_file(&path, 5120, BlockSize::Bytes512).unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn create_refuses_a_size_that_is_not_whole_blocks() {
        let path = scratch("partial");
        let _ = std::fs::remove_file(&path);
        let error = create_raw_file(&path, 1000, BlockSize::Bytes512).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!path.exists());
    }

    #[test]
    fn resize_grows_and_shrinks_the_file() {
        let path = scratch("resize");
        let _ = std::fs::remove_file(&path);
        create_raw_file(&path, 4096, BlockSize::Bytes4096).unwrap();
        let grown = resize_raw_file(&path, 3 * 4096, BlockSize::Bytes4096).unwrap();
        assert_eq!(grown.block_count, 3);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 12288);
        resize_raw_file(&path, 4096, BlockSize::Bytes4096).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 4096);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_reports_the_block_count_and_read_only_flag() {
        let path = scratch("open");
        let _ = std::fs::remove_file(&path);
        create_raw_file(&path, 8192, BlockSize::Bytes512).unwrap();
        let props = open_raw_file(&path, BlockSize::Bytes512, true).unwrap();
        assert_eq!(props.block_count, 16);
        assert!(props.read_only);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn inspect_calls_a_plain_file_raw_and_reads_an_iso_descriptor() {
        let path = scratch("inspect");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, vec![0u8; 40 * 1024]).unwrap();
        let raw = inspect_file(&path).unwrap();
        assert_eq!(raw.contents, ImageContents::Raw);
        assert_eq!(raw.length, 40 * 1024);

        // A primary volume descriptor at sector 16: type 1, "CD001", version 1, block count and block size.
        let mut bytes = vec![0u8; 40 * 1024];
        let pvd = 16 * 2048;
        bytes[pvd] = 1;
        bytes[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        bytes[pvd + 6] = 1;
        bytes[pvd + 40..pvd + 45].copy_from_slice(b"BIFRO");
        bytes[pvd + 80..pvd + 84].copy_from_slice(&100u32.to_le_bytes());
        bytes[pvd + 128..pvd + 130].copy_from_slice(&2048u16.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        let iso = inspect_file(&path).unwrap();
        assert_eq!(iso.contents, ImageContents::Iso9660);
        assert_eq!(iso.volume_name.as_deref(), Some("BIFRO"));
        assert_eq!(iso.declared_size(), Some(100 * 2048));
        assert!(!iso.is_complete(), "a 40 KB file is shorter than the 200 KB the volume declares");
        let _ = std::fs::remove_file(&path);
    }
}
