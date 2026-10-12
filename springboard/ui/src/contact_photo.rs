//! Contact pictures: the picture of each contact, kept beside its record. The shell asks this model before it stores a
//! picture. Only PNG and JPEG are accepted, detected from the first bytes of the data and not from a file name, and a
//! picture is capped at `MAX_BYTES`. Setting a picture replaces the old one, and removing it leaves the contact with
//! none. Pure: nothing is decoded here, and `fit_size` only works out the size of a thumbnail.

use std::collections::BTreeMap;

/// The largest picture a contact can hold, in bytes.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// The picture formats a contact can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The data is empty. Use `remove` to clear a picture.
    Empty,
    /// The data is longer than `MAX_BYTES`; the value is its length.
    TooLarge(usize),
    /// The first bytes are not a PNG or JPEG signature.
    UnknownFormat,
}

/// The format of picture data, from its signature (the first bytes of the file).
pub fn format_of(data: &[u8]) -> Option<Format> {
    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if data.starts_with(&PNG_SIGNATURE) {
        Some(Format::Png)
    } else if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        Some(Format::Jpeg)
    } else {
        None
    }
}

/// The picture of each contact, by contact identifier.
#[derive(Debug, Default)]
pub struct Photos {
    pictures: BTreeMap<u64, (Format, Vec<u8>)>,
}

impl Photos {
    pub fn new() -> Self {
        Photos { pictures: BTreeMap::new() }
    }

    /// Sets the picture of a contact, replacing any earlier one. Returns its format. A refused picture leaves the
    /// old one in place.
    pub fn set(&mut self, contact: u64, data: &[u8]) -> Result<Format, Error> {
        if data.is_empty() {
            return Err(Error::Empty);
        }
        if data.len() > MAX_BYTES {
            return Err(Error::TooLarge(data.len()));
        }
        let format = format_of(data).ok_or(Error::UnknownFormat)?;
        self.pictures.insert(contact, (format, data.to_vec()));
        Ok(format)
    }

    /// The picture bytes of a contact, if it has one.
    pub fn data(&self, contact: u64) -> Option<&[u8]> {
        self.pictures.get(&contact).map(|(_, bytes)| bytes.as_slice())
    }

    /// The format of a contact's picture, if it has one.
    pub fn format(&self, contact: u64) -> Option<Format> {
        self.pictures.get(&contact).map(|(format, _)| *format)
    }

    /// Clears the picture of a contact. `false` when it had none.
    pub fn remove(&mut self, contact: u64) -> bool {
        self.pictures.remove(&contact).is_some()
    }

    /// The number of contacts that have a picture.
    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty()
    }
}

/// The size of a thumbnail that fits a `width` by `height` picture inside a `max` by `max` square, keeping the aspect
/// ratio with integer arithmetic. A picture that already fits keeps its size. A non-zero side stays at least 1 pixel.
/// A zero source size, or a zero `max`, gives (0, 0).
pub fn fit_size(width: u32, height: u32, max: u32) -> (u32, u32) {
    let long = width.max(height);
    if long == 0 || max == 0 {
        return (0, 0);
    }
    if long <= max {
        return (width, height);
    }
    let scale = |side: u32| -> u32 {
        if side == 0 {
            0
        } else {
            ((side as u64 * max as u64 / long as u64) as u32).max(1)
        }
    };
    (scale(width), scale(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_HEAD: [u8; 10] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 1];
    const JPEG_HEAD: [u8; 4] = [0xFF, 0xD8, 0xFF, 0xE0];

    #[test]
    fn the_format_comes_from_the_signature_not_a_name() {
        assert_eq!(format_of(&PNG_HEAD), Some(Format::Png));
        assert_eq!(format_of(&JPEG_HEAD), Some(Format::Jpeg));
        assert_eq!(format_of(b"GIF89a"), None, "a format that is not held here");
        assert_eq!(format_of(&[0xFF, 0xD8]), None, "two bytes are not enough for a JPEG signature");
        assert_eq!(format_of(&[0x89, b'P', b'N', b'G']), None, "a truncated PNG signature");
    }

    #[test]
    fn a_picture_is_stored_and_read_back_with_its_format() {
        let mut photos = Photos::new();
        assert_eq!(photos.set(7, &PNG_HEAD), Ok(Format::Png));
        assert_eq!(photos.data(7), Some(&PNG_HEAD[..]));
        assert_eq!(photos.format(7), Some(Format::Png));
        assert_eq!(photos.len(), 1);
        assert_eq!(photos.data(8), None, "another contact has no picture");
    }

    #[test]
    fn a_new_picture_replaces_the_old_one() {
        let mut photos = Photos::new();
        photos.set(7, &PNG_HEAD).unwrap();
        assert_eq!(photos.set(7, &JPEG_HEAD), Ok(Format::Jpeg));
        assert_eq!(photos.format(7), Some(Format::Jpeg));
        assert_eq!(photos.data(7), Some(&JPEG_HEAD[..]));
        assert_eq!(photos.len(), 1, "still one picture for the contact");
    }

    #[test]
    fn empty_oversized_and_unknown_pictures_are_refused_and_keep_the_old_one() {
        let mut photos = Photos::new();
        photos.set(3, &JPEG_HEAD).unwrap();
        assert_eq!(photos.set(3, &[]), Err(Error::Empty));
        assert_eq!(photos.set(3, b"not a picture"), Err(Error::UnknownFormat));
        let big = vec![0xFFu8; MAX_BYTES + 1];
        assert_eq!(photos.set(3, &big), Err(Error::TooLarge(MAX_BYTES + 1)));
        assert_eq!(photos.data(3), Some(&JPEG_HEAD[..]), "the refused pictures changed nothing");
    }

    #[test]
    fn removing_a_picture_clears_it_and_reports_whether_there_was_one() {
        let mut photos = Photos::new();
        photos.set(5, &PNG_HEAD).unwrap();
        assert!(photos.remove(5));
        assert!(!photos.remove(5), "nothing left to remove");
        assert_eq!(photos.data(5), None);
        assert!(photos.is_empty());
    }

    #[test]
    fn a_thumbnail_keeps_the_aspect_ratio_inside_the_square() {
        assert_eq!(fit_size(400, 200, 100), (100, 50), "wide picture, width is the long side");
        assert_eq!(fit_size(300, 1000, 100), (30, 100), "tall picture, height is the long side");
        assert_eq!(fit_size(50, 20, 100), (50, 20), "a picture that fits keeps its size");
        assert_eq!(fit_size(1000, 1, 100), (100, 1), "a thin side stays at one pixel");
        assert_eq!(fit_size(0, 0, 100), (0, 0));
        assert_eq!(fit_size(10, 10, 0), (0, 0));
    }
}
