//! Contact pictures kept as files in a folder beside the contacts file: `<id>.png` or `<id>.jpg`, one per contact. The
//! format is checked from the bytes by the picture model (`bifrost_ui::contact_photo`), and a refused picture leaves the
//! old file in place. Deleting a contact removes its picture.
use bifrost_ui::contact_photo::{Format, Photos};
use std::io;
use std::path::{Path, PathBuf};

/// The folder that holds the pictures, beside the contacts file.
pub fn dir_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contact-photos")
}

fn extension(format: Format) -> &'static str {
    match format {
        Format::Png => "png",
        Format::Jpeg => "jpg",
    }
}

/// The format name an app sees.
pub fn format_name(format: Format) -> &'static str {
    match format {
        Format::Png => "png",
        Format::Jpeg => "jpeg",
    }
}

fn file_for(dir: &Path, id: u64, format: Format) -> PathBuf {
    dir.join(format!("{id}.{}", extension(format)))
}

/// The picture of a contact and its format, or `None` when it has none.
pub fn read(dir: &Path, id: u64) -> Option<(Format, Vec<u8>)> {
    [Format::Png, Format::Jpeg].into_iter().find_map(|format| std::fs::read(file_for(dir, id, format)).ok().map(|bytes| (format, bytes)))
}

/// Stores a picture for a contact, replacing any earlier one. The data is checked first (PNG or JPEG, within the size
/// cap); a refused picture leaves the old one in place. Returns the format stored.
pub fn write(dir: &Path, id: u64, data: &[u8]) -> Result<Format, String> {
    let format = Photos::new().set(id, data).map_err(|e| format!("{e:?}"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    remove(dir, id).map_err(|e| e.to_string())?;
    bifrost_core::fsutil::write_atomic(&file_for(dir, id, format), data).map_err(|e| e.to_string())?;
    Ok(format)
}

/// Removes a contact's picture. Returns whether there was one.
pub fn remove(dir: &Path, id: u64) -> io::Result<bool> {
    let mut removed = false;
    for format in [Format::Png, Format::Jpeg] {
        match std::fs::remove_file(file_for(dir, id, format)) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("contact-photos-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const PNG: [u8; 11] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];

    #[test]
    fn a_picture_is_stored_read_back_and_replaced() {
        let dir = scratch("replace");
        assert_eq!(write(&dir, 5, &PNG), Ok(Format::Png));
        assert_eq!(read(&dir, 5), Some((Format::Png, PNG.to_vec())));
        let jpeg = [0xFF, 0xD8, 0xFF, 9];
        assert_eq!(write(&dir, 5, &jpeg), Ok(Format::Jpeg));
        assert_eq!(read(&dir, 5), Some((Format::Jpeg, jpeg.to_vec())));
        assert!(!file_for(&dir, 5, Format::Png).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_picture_keeps_the_old_one_and_remove_clears_it() {
        let dir = scratch("refuse");
        write(&dir, 7, &PNG).unwrap();
        assert!(write(&dir, 7, b"not a picture").is_err());
        assert_eq!(read(&dir, 7), Some((Format::Png, PNG.to_vec())));
        assert!(matches!(remove(&dir, 7), Ok(true)));
        assert!(matches!(remove(&dir, 7), Ok(false)));
        assert_eq!(read(&dir, 7), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_folder_sits_beside_the_contacts_file() {
        assert_eq!(dir_for(Path::new("/d/Contacts/contacts.tsv")), PathBuf::from("/d/Contacts/contact-photos"));
    }
}
