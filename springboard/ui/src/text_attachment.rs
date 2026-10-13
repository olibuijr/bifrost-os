//! Text attachments (`NSTextAttachment`): an inline object, such as an image or a file icon, that takes the place of
//! one attachment character in a text and sizes its glyph run like a character would. A pure model: the caller draws
//! the content.

/// The character that stands for an attachment in the text (`NSAttachmentCharacter`, U+FFFC).
pub const ATTACHMENT_CHARACTER: char = '\u{FFFC}';

/// An inline object in a text (`NSTextAttachment`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextAttachment {
    /// `fileType`: a type identifier of the contents, for example "public.png".
    pub file_type: String,
    /// `contents`: the data, empty for an attachment that only reserves room.
    pub contents: Vec<u8>,
    /// `bounds`: the glyph run size in points, `(width, height)`. The image size when unset.
    pub bounds: Option<(f32, f32)>,
    /// The natural size of the contents, used when `bounds` is unset.
    pub image_size: (f32, f32),
    /// The vertical offset of the bottom edge from the baseline; negative sinks the object below the baseline.
    pub baseline_offset: f32,
}

impl TextAttachment {
    pub fn new(file_type: &str, contents: Vec<u8>, image_size: (f32, f32)) -> Self {
        TextAttachment { file_type: file_type.to_string(), contents, bounds: None, image_size, baseline_offset: 0.0 }
    }

    /// The size of the glyph run: `bounds` when set, otherwise the natural size.
    pub fn glyph_size(&self) -> (f32, f32) {
        self.bounds.unwrap_or(self.image_size)
    }

    /// `attachmentBoundsForTextContainer:`: the size, shrunk to fit `max_width` with the aspect ratio kept. A
    /// non-positive `max_width` means no limit.
    pub fn bounds_for_container(&self, max_width: f32) -> (f32, f32) {
        let (w, h) = self.glyph_size();
        if max_width > 0.0 && w > max_width {
            (max_width, h * max_width / w)
        } else {
            (w, h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_size_prefers_bounds_over_the_image() {
        let mut a = TextAttachment::new("public.png", vec![1], (40.0, 20.0));
        assert_eq!(a.glyph_size(), (40.0, 20.0));
        a.bounds = Some((10.0, 10.0));
        assert_eq!(a.glyph_size(), (10.0, 10.0));
    }

    #[test]
    fn wide_attachments_shrink_to_the_container_keeping_the_aspect() {
        let a = TextAttachment::new("public.png", vec![], (200.0, 100.0));
        assert_eq!(a.bounds_for_container(100.0), (100.0, 50.0));
        assert_eq!(a.bounds_for_container(0.0), (200.0, 100.0));
        assert_eq!(a.bounds_for_container(300.0), (200.0, 100.0));
    }
}
