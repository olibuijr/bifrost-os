//! Notification attachments: an image, audio or video file shown with a notification, and the thumbnail
//! options that choose what part of it the card shows. The file is named by a document grant the app already
//! holds, never by a path. This model checks the identifier, the file type and the options, and works out the
//! thumbnail rectangle for a given image size. It does not decode media; the shell reads the file.

use crate::Rect;

/// The kind of media an attachment carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Audio,
    Video,
}

impl Kind {
    /// The kind of a file name from its extension (case-insensitive); `None` for a type this model does not accept.
    pub fn from_extension(name: &str) -> Option<Kind> {
        let (_, ext) = name.rsplit_once('.')?;
        match ext.to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "gif" => Some(Kind::Image),
            "mp3" | "m4a" | "wav" | "aiff" | "caf" => Some(Kind::Audio),
            "mp4" | "mov" | "m4v" => Some(Kind::Video),
            _ => None,
        }
    }

    /// The kind named by a type hint such as `public.png` (`UNNotificationAttachmentOptionsTypeHintKey`).
    pub fn from_hint(hint: &str) -> Option<Kind> {
        match hint {
            "public.png" | "public.jpeg" | "public.gif" => Some(Kind::Image),
            "public.mp3" | "public.wav" => Some(Kind::Audio),
            "public.mpeg-4" | "public.movie" => Some(Kind::Video),
            _ => None,
        }
    }
}

/// `UNNotificationAttachmentOptions`: all optional, the defaults show the whole file's thumbnail.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    /// `UNNotificationAttachmentOptionsTypeHintKey`: decides the kind when the file name has no usable extension.
    pub type_hint: Option<String>,
    /// `UNNotificationAttachmentOptionsThumbnailHiddenKey`: the card shows no thumbnail.
    pub thumbnail_hidden: bool,
    /// `UNNotificationAttachmentOptionsThumbnailClippingRectKey`: the part of the image shown, as fractions of its
    /// width and height (0 to 1). Must lie inside the unit square with a positive size.
    pub clip: Option<Rect>,
    /// `UNNotificationAttachmentOptionsThumbnailTimeKey`: the frame of an animated image or a movie used as thumbnail.
    pub thumbnail_frame: Option<u32>,
}

/// Why an attachment was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyId,
    EmptyGrant,
    /// the extension and the type hint both fail to name a supported kind
    UnsupportedType,
    /// the clipping rectangle is empty or leaves the unit square
    BadClip,
}

/// One attachment of a notification request (`UNNotificationAttachment`).
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub id: String,
    /// the document grant naming the file
    pub grant: String,
    pub kind: Kind,
    pub options: Options,
}

impl Attachment {
    /// `init(identifier:url:options:)`. A type hint that names a kind wins over the extension.
    pub fn new(id: &str, grant: &str, options: Options) -> Result<Attachment, Error> {
        if id.trim().is_empty() {
            return Err(Error::EmptyId);
        }
        if grant.trim().is_empty() {
            return Err(Error::EmptyGrant);
        }
        if let Some(clip) = options.clip {
            let inside = clip.x >= 0.0 && clip.y >= 0.0 && clip.w > 0.0 && clip.h > 0.0 && clip.right() <= 1.0 && clip.bottom() <= 1.0;
            if !inside {
                return Err(Error::BadClip);
            }
        }
        let kind = options
            .type_hint
            .as_deref()
            .and_then(Kind::from_hint)
            .or_else(|| Kind::from_extension(grant))
            .ok_or(Error::UnsupportedType)?;
        Ok(Attachment { id: id.into(), grant: grant.into(), kind, options })
    }

    /// The rectangle of the thumbnail inside an image of `width` by `height` points, or `None` when the thumbnail
    /// is hidden. Without a clip the whole image is the thumbnail.
    pub fn thumbnail_rect(&self, width: f32, height: f32) -> Option<Rect> {
        if self.options.thumbnail_hidden {
            return None;
        }
        Some(match self.options.clip {
            None => Rect { x: 0.0, y: 0.0, w: width, h: height },
            Some(c) => Rect { x: c.x * width, y: c.y * height, w: c.w * width, h: c.h * height },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(x: f32, y: f32, w: f32, h: f32) -> Options {
        Options { clip: Some(Rect { x, y, w, h }), ..Default::default() }
    }

    #[test]
    fn extension_decides_the_kind_case_insensitively() {
        assert_eq!(Kind::from_extension("cover.PNG"), Some(Kind::Image));
        assert_eq!(Kind::from_extension("song.m4a"), Some(Kind::Audio));
        assert_eq!(Kind::from_extension("clip.mov"), Some(Kind::Video));
        assert_eq!(Kind::from_extension("notes.txt"), None);
        assert_eq!(Kind::from_extension("noextension"), None);
    }

    #[test]
    fn type_hint_overrides_the_extension() {
        let opts = Options { type_hint: Some("public.mp3".into()), ..Default::default() };
        let a = Attachment::new("a", "grant-1/cover.png", opts).unwrap();
        assert_eq!(a.kind, Kind::Audio);
        // an unknown hint falls back to the extension
        let opts = Options { type_hint: Some("public.unknown".into()), ..Default::default() };
        assert_eq!(Attachment::new("a", "grant-1/cover.png", opts).unwrap().kind, Kind::Image);
    }

    #[test]
    fn empty_identifier_and_grant_are_refused() {
        assert_eq!(Attachment::new("  ", "g/x.png", Options::default()), Err(Error::EmptyId));
        assert_eq!(Attachment::new("a", "", Options::default()), Err(Error::EmptyGrant));
    }

    #[test]
    fn unsupported_file_is_refused() {
        assert_eq!(Attachment::new("a", "g/readme.txt", Options::default()), Err(Error::UnsupportedType));
    }

    #[test]
    fn clip_must_stay_inside_the_unit_square_with_positive_size() {
        assert!(Attachment::new("a", "g/x.png", clip(0.0, 0.0, 1.0, 1.0)).is_ok());
        assert_eq!(Attachment::new("a", "g/x.png", clip(0.5, 0.0, 0.6, 1.0)), Err(Error::BadClip));
        assert_eq!(Attachment::new("a", "g/x.png", clip(0.0, 0.0, 0.0, 1.0)), Err(Error::BadClip));
        assert_eq!(Attachment::new("a", "g/x.png", clip(-0.1, 0.0, 0.5, 0.5)), Err(Error::BadClip));
    }

    #[test]
    fn thumbnail_rect_scales_the_clip_and_respects_hiding() {
        let a = Attachment::new("a", "g/x.png", clip(0.25, 0.5, 0.5, 0.5)).unwrap();
        assert_eq!(a.thumbnail_rect(400.0, 200.0), Some(Rect { x: 100.0, y: 100.0, w: 200.0, h: 100.0 }));
        let whole = Attachment::new("a", "g/x.png", Options::default()).unwrap();
        assert_eq!(whole.thumbnail_rect(400.0, 200.0), Some(Rect { x: 0.0, y: 0.0, w: 400.0, h: 200.0 }));
        let hidden = Attachment::new("a", "g/x.png", Options { thumbnail_hidden: true, ..Default::default() }).unwrap();
        assert_eq!(hidden.thumbnail_rect(400.0, 200.0), None);
    }

    #[test]
    fn thumbnail_frame_is_kept_for_animated_images_and_movies() {
        let opts = Options { thumbnail_frame: Some(12), ..Default::default() };
        let a = Attachment::new("a", "g/clip.mp4", opts).unwrap();
        assert_eq!(a.kind, Kind::Video);
        assert_eq!(a.options.thumbnail_frame, Some(12));
    }
}
