//! Export Photos: writes the chosen photos, with their edits applied, as JPEG, PNG or TIFF at a chosen size and
//! quality, named after the original or numbered. The options are kept in the settings file.

use crate::gfx::Image;
use crate::imgdec;
use crate::shell::camera::jpeg::{self, Exif};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Tiff,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Jpeg, Kind::Png, Kind::Tiff];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Jpeg => "JPEG",
            Kind::Png => "PNG",
            Kind::Tiff => "TIFF",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
            Kind::Tiff => "tiff",
        }
    }

    pub fn next(self) -> Kind {
        match self {
            Kind::Jpeg => Kind::Png,
            Kind::Png => Kind::Tiff,
            Kind::Tiff => Kind::Jpeg,
        }
    }
}

/// JPEG quality choices, as the export sheet's Quality row steps through them.
pub const QUALITIES: [u8; 4] = [40, 70, 90, 100];

pub fn quality_label(q: u8) -> &'static str {
    match q {
        0..=50 => "Low",
        51..=80 => "Medium",
        81..=95 => "High",
        _ => "Maximum",
    }
}

/// Longest edge choices; 0 is the full size.
pub const SIZES: [u32; 4] = [0, 4096, 2048, 1024];

pub fn size_label(edge: u32) -> String {
    if edge == 0 { "Full Size".to_string() } else { format!("{edge} px") }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Naming {
    /// the original file name with the new extension
    Original,
    /// "Photo 1", "Photo 2", ... in the order exported
    Sequential,
}

impl Naming {
    pub fn label(self) -> &'static str {
        match self {
            Naming::Original => "Original Names",
            Naming::Sequential => "Numbered",
        }
    }

    pub fn next(self) -> Naming {
        match self {
            Naming::Original => Naming::Sequential,
            Naming::Sequential => Naming::Original,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub kind: Kind,
    pub quality: u8,
    /// longest edge in px, 0 for the full size
    pub edge: u32,
    /// write the capture date into the file (JPEG)
    pub metadata: bool,
    pub naming: Naming,
}

impl Default for Options {
    fn default() -> Self {
        Options { kind: Kind::Jpeg, quality: 90, edge: 0, metadata: true, naming: Naming::Original }
    }
}

impl Options {
    /// The settings line: `jpeg,90,0,1,orig`.
    pub fn encode(&self) -> String {
        format!("{},{},{},{},{}", self.kind.ext(), self.quality, self.edge, u8::from(self.metadata), if self.naming == Naming::Original { "orig" } else { "seq" })
    }

    /// Reads `encode`'s line; anything unreadable leaves the default for that part.
    pub fn decode(text: &str) -> Options {
        let mut o = Options::default();
        let f: Vec<&str> = text.split(',').collect();
        if let Some(k) = f.first().and_then(|k| Kind::ALL.into_iter().find(|c| c.ext() == *k)) {
            o.kind = k;
        }
        if let Some(q) = f.get(1).and_then(|q| q.parse::<u8>().ok()).filter(|q| QUALITIES.contains(q)) {
            o.quality = q;
        }
        if let Some(e) = f.get(2).and_then(|e| e.parse::<u32>().ok()).filter(|e| SIZES.contains(e)) {
            o.edge = e;
        }
        if let Some(m) = f.get(3) {
            o.metadata = *m == "1";
        }
        if let Some(n) = f.get(4) {
            o.naming = if *n == "seq" { Naming::Sequential } else { Naming::Original };
        }
        o
    }

    pub fn next_quality(&mut self) {
        let i = QUALITIES.iter().position(|&q| q == self.quality).unwrap_or(2);
        self.quality = QUALITIES[(i + 1) % QUALITIES.len()];
    }

    pub fn next_edge(&mut self) {
        let i = SIZES.iter().position(|&e| e == self.edge).unwrap_or(0);
        self.edge = SIZES[(i + 1) % SIZES.len()];
    }
}

/// The file name of the `index`-th photo (from 0) exported from the file `original`.
pub fn file_name(opts: &Options, original: &str, index: usize) -> String {
    match opts.naming {
        Naming::Original => {
            let stem = original.rsplit_once('.').map_or(original, |(s, _)| s);
            format!("{stem}.{}", opts.kind.ext())
        }
        Naming::Sequential => format!("Photo {}.{}", index + 1, opts.kind.ext()),
    }
}

/// The bytes of `img` as the export asks. `taken` is "YYYY:MM:DD HH:MM:SS" for the metadata option.
pub fn encode(img: &Image, opts: &Options, taken: Option<&str>) -> Vec<u8> {
    let img = if opts.edge > 0 { imgdec::contain(img, opts.edge as usize, opts.edge as usize) } else { img.clone() };
    match opts.kind {
        Kind::Jpeg => {
            let exif = (opts.metadata).then(|| Exif { orientation: 1, date: taken.unwrap_or("").to_string() }).filter(|e| !e.date.is_empty());
            jpeg::encode_pixels(img.w, img.h, &img.px, opts.quality, exif.as_ref())
        }
        Kind::Png => crate::pngenc::encode_rgb(img.w, img.h, &img.px),
        Kind::Tiff => tiff(&img),
    }
}

/// A baseline little-endian TIFF: 8-bit RGB, one strip, no compression.
pub fn tiff(img: &Image) -> Vec<u8> {
    let (w, h) = (img.w as u32, img.h as u32);
    let data_len = w * h * 3;
    let mut out = Vec::with_capacity(data_len as usize + 200);
    out.extend(b"II");
    out.extend(42u16.to_le_bytes());
    out.extend(8u32.to_le_bytes());
    // the directory at offset 8: 9 entries, then the next-directory word; extra values follow it
    let entries = 9u16;
    let dir_len = 2 + entries as u32 * 12 + 4;
    let bits_at = 8 + dir_len;
    let data_at = bits_at + 6;
    out.extend(entries.to_le_bytes());
    let mut entry = |tag: u16, typ: u16, count: u32, value: u32| {
        out.extend(tag.to_le_bytes());
        out.extend(typ.to_le_bytes());
        out.extend(count.to_le_bytes());
        out.extend(value.to_le_bytes());
    };
    entry(256, 4, 1, w); // ImageWidth
    entry(257, 4, 1, h); // ImageLength
    entry(258, 3, 3, bits_at); // BitsPerSample
    entry(259, 3, 1, 1); // Compression: none
    entry(262, 3, 1, 2); // Photometric: RGB
    entry(273, 4, 1, data_at); // StripOffsets
    entry(277, 3, 1, 3); // SamplesPerPixel
    entry(278, 4, 1, h); // RowsPerStrip
    entry(279, 4, 1, data_len); // StripByteCounts
    out.extend(0u32.to_le_bytes());
    for _ in 0..3 {
        out.extend(8u16.to_le_bytes());
    }
    for &p in &img.px {
        out.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for (i, p) in img.px.iter_mut().enumerate() {
            *p = crate::gfx::pack((i % 256) as u8, (i / 3 % 256) as u8, 90, 255);
        }
        img
    }

    #[test]
    fn options_round_trip_and_bad_parts_fall_back() {
        let o = Options { kind: Kind::Tiff, quality: 40, edge: 2048, metadata: false, naming: Naming::Sequential };
        assert_eq!(Options::decode(&o.encode()), o);
        assert_eq!(Options::decode(""), Options::default());
        let odd = Options::decode("bmp,77,13,1,seq");
        assert_eq!((odd.kind, odd.quality, odd.edge, odd.naming), (Kind::Jpeg, 90, 0, Naming::Sequential));
    }

    #[test]
    fn rows_step_through_their_choices() {
        let mut o = Options::default();
        o.next_quality();
        assert_eq!(o.quality, 100);
        o.next_quality();
        assert_eq!(o.quality, 40);
        o.next_edge();
        assert_eq!(o.edge, 4096);
        for _ in 0..3 {
            o.next_edge();
        }
        assert_eq!(o.edge, 0);
        assert_eq!(Kind::Jpeg.next().next().next(), Kind::Jpeg);
        assert_eq!(quality_label(90), "High");
        assert_eq!(size_label(0), "Full Size");
    }

    #[test]
    fn names_follow_the_original_or_a_count() {
        let mut o = Options::default();
        assert_eq!(file_name(&o, "IMG_0001.heic", 3), "IMG_0001.jpg");
        assert_eq!(file_name(&o, "noext", 0), "noext.jpg");
        o.naming = Naming::Sequential;
        o.kind = Kind::Png;
        assert_eq!(file_name(&o, "IMG_0001.jpg", 3), "Photo 4.png");
    }

    #[test]
    fn each_kind_writes_a_file_its_reader_recognises() {
        let img = gradient(40, 30);
        let mut o = Options::default();
        let jpg = encode(&img, &o, Some("2026:10:04 12:00:00"));
        assert_eq!(&jpg[..2], &[0xFF, 0xD8]);
        assert_eq!(crate::cg::imageio::decode(&jpg).map(|i| (i.w, i.h)), Some((40, 30)));
        o.kind = Kind::Png;
        let png = encode(&img, &o, None);
        assert_eq!(&png[..4], b"\x89PNG");
        assert_eq!(crate::cg::imageio::decode(&png).unwrap().px, img.px.iter().map(|p| p | 0xFF00_0000).collect::<Vec<_>>());
        o.kind = Kind::Tiff;
        let t = encode(&img, &o, None);
        assert_eq!(&t[..4], b"II*\0");
        assert_eq!(t.len(), 8 + 2 + 9 * 12 + 4 + 6 + 40 * 30 * 3);
    }

    #[test]
    fn a_size_choice_shrinks_but_never_enlarges() {
        let img = gradient(400, 200);
        let mut o = Options { kind: Kind::Png, edge: 1024, ..Options::default() };
        assert_eq!(crate::cg::imageio::decode(&encode(&img, &o, None)).map(|i| (i.w, i.h)), Some((400, 200)));
        o.edge = 100;
        assert_eq!(crate::cg::imageio::decode(&encode(&img, &o, None)).map(|i| (i.w, i.h)), Some((100, 50)));
    }
}
