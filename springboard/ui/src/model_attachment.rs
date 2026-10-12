//! Image attachments for a prompt. The format is read from the first bytes of the data, not from a file name, and a size
//! limit is checked before the attachment is kept. An attachment becomes a transcript segment that names it and gives
//! its size, the same segment the transcript already holds for a sent file.
//!
//! This is a pure model. It reads bytes the caller already holds and opens no file. Decoding a picture is the renderer's
//! job (`render/src/imgdec.rs`), and nothing here checks that the picture decodes.

use crate::transcript::Segment;
use std::fmt;

/// The largest attachment kept when the caller gives no limit: 20 MiB. This is a BifrOSt limit, not a model limit.
pub const DEFAULT_MAX_BYTES: usize = 20 * 1024 * 1024;

/// An image format an attachment can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Media {
    /// The media type of the format, as it is named in a request.
    pub fn mime(self) -> &'static str {
        match self {
            Media::Png => "image/png",
            Media::Jpeg => "image/jpeg",
            Media::Gif => "image/gif",
            Media::Webp => "image/webp",
        }
    }

    /// The format of `data`, read from its leading bytes. `None` for anything else, including data too short to tell.
    pub fn sniff(data: &[u8]) -> Option<Media> {
        if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(Media::Png)
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Media::Jpeg)
        } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            Some(Media::Gif)
        } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == &b"WEBP"[..] {
            Some(Media::Webp)
        } else {
            None
        }
    }
}

/// Why data could not become an attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentError {
    Empty,
    /// The leading bytes are not one of the image formats.
    NotAnImage,
    TooLarge { bytes: usize, max: usize },
}

impl fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachmentError::Empty => write!(f, "the attachment has no data"),
            AttachmentError::NotAnImage => write!(f, "the attachment is not a PNG, JPEG, GIF or WebP image"),
            AttachmentError::TooLarge { bytes, max } => {
                write!(f, "attachment of {bytes} bytes is over the {max} byte limit")
            }
        }
    }
}

/// An image sent with a prompt. It keeps its format, its size and an optional label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    label: String,
    media: Media,
    bytes: usize,
}

impl Attachment {
    /// An attachment made from `data`, which must be an image of at most `max_bytes`.
    pub fn from_data(data: &[u8], max_bytes: usize) -> Result<Self, AttachmentError> {
        if data.is_empty() {
            return Err(AttachmentError::Empty);
        }
        let media = Media::sniff(data).ok_or(AttachmentError::NotAnImage)?;
        if data.len() > max_bytes {
            return Err(AttachmentError::TooLarge { bytes: data.len(), max: max_bytes });
        }
        Ok(Attachment { label: String::new(), media, bytes: data.len() })
    }

    /// Names the attachment, so the model can refer to it when several are sent with one prompt.
    pub fn labelled(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn media(&self) -> Media {
        self.media
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The transcript segment for this attachment: its label, or its media type when it has none, and its size.
    pub fn segment(&self) -> Segment {
        let name = if self.label.is_empty() { self.media.mime().to_string() } else { self.label.clone() };
        Segment::Attachment { name, bytes: self.bytes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_HEADER: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn each_format_is_read_from_its_leading_bytes() {
        assert_eq!(Media::sniff(&PNG_HEADER), Some(Media::Png));
        assert_eq!(Media::sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 16]), Some(Media::Jpeg));
        assert_eq!(Media::sniff(b"GIF89a\x01\x00"), Some(Media::Gif));
        assert_eq!(Media::sniff(b"GIF87a"), Some(Media::Gif));
        assert_eq!(Media::sniff(b"RIFF\x24\x00\x00\x00WEBPVP8 "), Some(Media::Webp));
    }

    #[test]
    fn data_that_is_not_an_image_or_is_too_short_to_tell_is_refused() {
        assert_eq!(Media::sniff(&[]), None);
        assert_eq!(Media::sniff(&[0x89, b'P', b'N', b'G']), None, "a truncated PNG signature is not a PNG");
        assert_eq!(Media::sniff(b"RIFF1234WAVE"), None, "a RIFF file of another kind is not WebP");
        assert_eq!(Media::sniff(b"RIFF"), None);
        assert_eq!(Media::sniff(b"%PDF-1.7"), None, "documents are not image attachments");
    }

    #[test]
    fn from_data_refuses_empty_data_a_non_image_and_an_oversized_image() {
        assert_eq!(Attachment::from_data(&[], DEFAULT_MAX_BYTES), Err(AttachmentError::Empty));
        assert_eq!(Attachment::from_data(b"hello", DEFAULT_MAX_BYTES), Err(AttachmentError::NotAnImage));
        assert_eq!(
            Attachment::from_data(&PNG_HEADER, 7),
            Err(AttachmentError::TooLarge { bytes: 8, max: 7 }),
        );
        let exact = Attachment::from_data(&PNG_HEADER, 8).unwrap();
        assert_eq!(exact.bytes(), 8, "a file of exactly the limit is kept");
        assert_eq!(exact.media(), Media::Png);
    }

    #[test]
    fn the_segment_names_the_attachment_by_its_label_or_its_media_type() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        let labelled = Attachment::from_data(&jpeg, DEFAULT_MAX_BYTES).unwrap().labelled("map");
        assert_eq!(labelled.label(), "map");
        assert_eq!(labelled.segment(), Segment::Attachment { name: "map".into(), bytes: 4 });
        let plain = Attachment::from_data(&PNG_HEADER, DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(plain.label(), "");
        assert_eq!(plain.segment(), Segment::Attachment { name: "image/png".into(), bytes: 8 });
    }

    #[test]
    fn mime_types_and_error_messages_are_fixed_text() {
        assert_eq!(Media::Gif.mime(), "image/gif");
        assert_eq!(Media::Webp.mime(), "image/webp");
        assert_eq!(
            AttachmentError::TooLarge { bytes: 9, max: 4 }.to_string(),
            "attachment of 9 bytes is over the 4 byte limit",
        );
    }
}
