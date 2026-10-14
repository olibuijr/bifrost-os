//! Export: a copy of a photo, with its saved edit applied, written as a JPEG or a PNG at a chosen quality and size,
//! optionally carrying the title, caption, keywords and place a person gave it. The original file is never touched.
//! The choices are kept in `photos.ini` and cycle on tap in the export sheet.

use super::meta::Details;
use crate::cg::imageio::encode::{encode_jpeg, encode_png, JpegOptions, PngOptions};
use crate::gfx::Image;
use crate::imgdec;
use crate::shell::prefs::Prefs;

/// The file format of the copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
}

/// How hard a JPEG is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Maximum,
    High,
    Medium,
    Low,
}

/// The longest side of the copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Full,
    Large,
    Medium,
    Small,
}

/// Everything the export sheet chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub kind: Kind,
    pub quality: Quality,
    pub size: Size,
    /// write the title, caption, keywords and place into the file
    pub details: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { kind: Kind::Jpeg, quality: Quality::High, size: Size::Full, details: true }
    }
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Jpeg => "JPEG",
            Kind::Png => "PNG",
        }
    }

    pub fn next(self) -> Kind {
        match self {
            Kind::Jpeg => Kind::Png,
            Kind::Png => Kind::Jpeg,
        }
    }

    /// The extension of files of this kind.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
        }
    }
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Quality::Maximum => "Maximum",
            Quality::High => "High",
            Quality::Medium => "Medium",
            Quality::Low => "Low",
        }
    }

    pub fn next(self) -> Quality {
        match self {
            Quality::Maximum => Quality::High,
            Quality::High => Quality::Medium,
            Quality::Medium => Quality::Low,
            Quality::Low => Quality::Maximum,
        }
    }

    /// The JPEG quality setting, 1 to 100.
    pub fn jpeg(self) -> u8 {
        match self {
            Quality::Maximum => 98,
            Quality::High => 90,
            Quality::Medium => 75,
            Quality::Low => 50,
        }
    }
}

impl Size {
    pub fn label(self) -> &'static str {
        match self {
            Size::Full => "Full Size",
            Size::Large => "Large",
            Size::Medium => "Medium",
            Size::Small => "Small",
        }
    }

    pub fn next(self) -> Size {
        match self {
            Size::Full => Size::Large,
            Size::Large => Size::Medium,
            Size::Medium => Size::Small,
            Size::Small => Size::Full,
        }
    }

    /// The longest side in pixels; None keeps the size of the picture.
    pub fn longest(self) -> Option<usize> {
        match self {
            Size::Full => None,
            Size::Large => Some(2048),
            Size::Medium => Some(1024),
            Size::Small => Some(512),
        }
    }
}

impl Options {
    /// The choices kept in `prefs`; anything missing or unknown is the default.
    pub fn load(prefs: &Prefs) -> Options {
        let d = Options::default();
        Options {
            kind: match prefs.get("export.kind") {
                Some("png") => Kind::Png,
                Some("jpeg") => Kind::Jpeg,
                _ => d.kind,
            },
            quality: match prefs.get("export.quality") {
                Some("maximum") => Quality::Maximum,
                Some("high") => Quality::High,
                Some("medium") => Quality::Medium,
                Some("low") => Quality::Low,
                _ => d.quality,
            },
            size: match prefs.get("export.size") {
                Some("full") => Size::Full,
                Some("large") => Size::Large,
                Some("medium") => Size::Medium,
                Some("small") => Size::Small,
                _ => d.size,
            },
            details: prefs.get("export.details").map_or(d.details, |v| v == "1"),
        }
    }

    /// The prefs writes that keep these choices.
    pub fn writes(&self) -> Vec<(String, String)> {
        let kind = if self.kind == Kind::Png { "png" } else { "jpeg" };
        let quality = self.quality.label().to_lowercase();
        let size = match self.size {
            Size::Full => "full",
            Size::Large => "large",
            Size::Medium => "medium",
            Size::Small => "small",
        };
        vec![
            ("export.kind".to_string(), kind.to_string()),
            ("export.quality".to_string(), quality),
            ("export.size".to_string(), size.to_string()),
            ("export.details".to_string(), if self.details { "1" } else { "0" }.to_string()),
        ]
    }
}

/// The name of the exported file: the stem of `name` with the extension of `kind`.
pub fn file_name(name: &str, kind: Kind) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = if stem.is_empty() { "Photo" } else { stem };
    format!("{stem}.{}", kind.extension())
}

/// The text a person wrote about the photo, as (name, text) pairs for the file; fields left empty are left out.
pub fn notes(details: &Details) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if !details.title.is_empty() {
        out.push(("Title", details.title.clone()));
    }
    if !details.caption.is_empty() {
        out.push(("Description", details.caption.clone()));
    }
    if !details.keywords.is_empty() {
        out.push(("Keywords", details.keywords.join(", ")));
    }
    if !details.place.is_empty() {
        out.push(("Location", details.place.clone()));
    }
    out
}

/// The pixels as straight (not premultiplied) RGBA bytes, which is what the encoders take.
pub fn straight_rgba(img: &Image) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.px.len() * 4);
    for &p in &img.px {
        let [r, g, b, a] = p.to_le_bytes();
        match a {
            255 => out.extend([r, g, b, a]),
            0 => out.extend([0, 0, 0, 0]),
            _ => {
                let straight = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                out.extend([straight(r), straight(g), straight(b), a]);
            }
        }
    }
    out
}

/// The file contents for `img` under `options`; None when the picture is empty or too large to write.
pub fn encode(img: &Image, options: &Options, details: &Details) -> Option<Vec<u8>> {
    let scaled = match options.size.longest() {
        Some(side) => imgdec::contain(img, side, side),
        None => img.clone(),
    };
    let rgba = straight_rgba(&scaled);
    let notes = if options.details { notes(details) } else { Vec::new() };
    match options.kind {
        Kind::Png => {
            let text = notes.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            encode_png(scaled.w, scaled.h, &rgba, &PngOptions { text, ..PngOptions::default() })
        }
        Kind::Jpeg => {
            let comment = (!notes.is_empty()).then(|| notes.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"));
            encode_jpeg(scaled.w, scaled.h, &rgba, &JpegOptions { quality: options.quality.jpeg(), comment, ..JpegOptions::default() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    fn details() -> Details {
        Details { title: "Harbour".into(), caption: "Evening".into(), keywords: vec!["sea".into(), "boats".into()], place: "Reykjavik".into(), taken: None }
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn choices_round_trip_through_the_prefs_file() {
        let mut prefs = Prefs::memory();
        assert_eq!(Options::load(&prefs), Options::default());
        let chosen = Options { kind: Kind::Png, quality: Quality::Low, size: Size::Medium, details: false };
        let writes = chosen.writes();
        let pairs: Vec<(&str, &str)> = writes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        prefs.set_many(&pairs).unwrap();
        assert_eq!(Options::load(&prefs), chosen);
        prefs.set("export.kind", "tiff").unwrap();
        assert_eq!(Options::load(&prefs).kind, Kind::Jpeg, "an unknown value is the default");
    }

    #[test]
    fn each_choice_cycles_through_all_its_values() {
        let (mut k, mut q, mut s) = (Kind::Jpeg, Quality::Maximum, Size::Full);
        let (mut kinds, mut qualities, mut sizes) = (vec![k], vec![q], vec![s]);
        for _ in 0..4 {
            k = k.next();
            q = q.next();
            s = s.next();
            kinds.push(k);
            qualities.push(q);
            sizes.push(s);
        }
        assert_eq!(kinds[..3], [Kind::Jpeg, Kind::Png, Kind::Jpeg]);
        assert_eq!(qualities[..4], [Quality::Maximum, Quality::High, Quality::Medium, Quality::Low]);
        assert_eq!(qualities[4], Quality::Maximum);
        assert_eq!(sizes[..4], [Size::Full, Size::Large, Size::Medium, Size::Small]);
        assert_eq!(sizes[4], Size::Full);
        assert!(Quality::Maximum.jpeg() > Quality::High.jpeg() && Quality::High.jpeg() > Quality::Medium.jpeg() && Quality::Medium.jpeg() > Quality::Low.jpeg());
        assert_eq!((Size::Full.longest(), Size::Small.longest()), (None, Some(512)));
    }

    #[test]
    fn the_file_keeps_its_stem_and_takes_the_extension_of_the_kind() {
        assert_eq!(file_name("IMG_0042.PNG", Kind::Jpeg), "IMG_0042.jpg");
        assert_eq!(file_name("beach.holiday.jpeg", Kind::Png), "beach.holiday.png");
        assert_eq!(file_name("noext", Kind::Png), "noext.png");
        assert_eq!(file_name(".png", Kind::Jpeg), "Photo.jpg");
    }

    #[test]
    fn straight_pixels_undo_the_premultiplication() {
        let img = Image { w: 3, h: 1, px: vec![pack(10, 20, 30, 255), pack(0, 0, 0, 0), pack(50, 100, 25, 128)] };
        let rgba = straight_rgba(&img);
        assert_eq!(&rgba[..4], [10, 20, 30, 255]);
        assert_eq!(&rgba[4..8], [0, 0, 0, 0]);
        assert_eq!(rgba[11], 128);
        assert!((rgba[8] as i32 - 99).abs() <= 1 && (rgba[9] as i32 - 199).abs() <= 1 && (rgba[10] as i32 - 50).abs() <= 1, "{:?}", &rgba[8..12]);
    }

    #[test]
    fn a_png_export_reads_back_the_same_pixels_and_carries_the_details() {
        let img = solid(6, 4, pack(10, 20, 30, 255));
        let options = Options { kind: Kind::Png, ..Options::default() };
        let bytes = encode(&img, &options, &details()).expect("encoded");
        let back = crate::cg::imageio::decode(&bytes).expect("a PNG");
        assert_eq!(back, img);
        assert!(contains(&bytes, "Harbour") && contains(&bytes, "Reykjavik") && contains(&bytes, "sea, boats"));
        let plain = encode(&img, &Options { details: false, ..options }, &details()).unwrap();
        assert!(!contains(&plain, "Harbour"), "the details stay out when asked");
    }

    #[test]
    fn a_jpeg_export_is_a_jpeg_close_to_the_picture_with_a_comment() {
        let img = solid(16, 16, pack(200, 100, 50, 255));
        let bytes = encode(&img, &Options::default(), &details()).expect("encoded");
        assert_eq!(&bytes[..2], [0xFF, 0xD8], "a JPEG starts with its marker");
        let back = crate::cg::imageio::decode(&bytes).expect("a JPEG");
        assert_eq!((back.w, back.h), (16, 16));
        let [r, g, b, _] = back.px[0].to_le_bytes();
        assert!((r as i32 - 200).abs() < 8 && (g as i32 - 100).abs() < 8 && (b as i32 - 50).abs() < 8, "{r} {g} {b}");
        assert!(contains(&bytes, "Title: Harbour"));
        let small = encode(&img, &Options { quality: Quality::Low, details: false, ..Options::default() }, &details()).unwrap();
        assert!(!contains(&small, "Harbour"));
    }

    #[test]
    fn the_size_limits_the_longest_side_and_never_enlarges() {
        let img = solid(3000, 1500, pack(1, 2, 3, 255));
        let png = |size| encode(&img, &Options { kind: Kind::Png, size, details: false, ..Options::default() }, &Details::default()).unwrap();
        let dims = |bytes: &[u8]| crate::cg::imageio::pixel_size(bytes).unwrap();
        assert_eq!(dims(&png(Size::Full)), (3000, 1500));
        assert_eq!(dims(&png(Size::Large)), (2048, 1024));
        assert_eq!(dims(&png(Size::Small)), (512, 256));
        let tiny = solid(100, 50, pack(1, 2, 3, 255));
        let bytes = encode(&tiny, &Options { kind: Kind::Png, size: Size::Large, ..Options::default() }, &Details::default()).unwrap();
        assert_eq!(dims(&bytes), (100, 50));
    }

    #[test]
    fn an_empty_picture_does_not_encode() {
        assert!(encode(&Image { w: 0, h: 0, px: vec![] }, &Options::default(), &Details::default()).is_none());
    }

    #[test]
    fn notes_list_only_what_was_written() {
        assert!(notes(&Details::default()).is_empty());
        let n = notes(&details());
        assert_eq!(n.iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["Title", "Description", "Keywords", "Location"]);
        assert_eq!(n[2].1, "sea, boats");
    }
}
