//! Image properties and the TIFF-structured metadata block (the container that holds EXIF, GPS and TIFF tags in
//! JPEG APP1 segments, PNG `eXIf` chunks and TIFF files themselves): a reader that fills property dictionaries
//! and a writer that builds the block back from them.

use std::collections::BTreeMap;

/// A metadata value.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Int(i64),
    Float(f64),
    Text(String),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
    Bytes(Vec<u8>),
    Dict(Dict),
}

impl PropValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            Self::Ints(v) => v.first().map(|&i| i as f64),
            Self::Floats(v) => v.first().copied(),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Ints(v) => v.first().copied(),
            Self::Float(f) => Some(*f as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// A named group of properties.
pub type Dict = BTreeMap<String, PropValue>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorModel {
    #[default]
    Rgb,
    Gray,
    Cmyk,
    Indexed,
}

/// What an image source knows about one image without decoding its pixels.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Properties {
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub dpi_width: Option<f64>,
    pub dpi_height: Option<f64>,
    /// Bits per colour component.
    pub depth: u32,
    /// EXIF orientation 1..=8 (1 = upright).
    pub orientation: u8,
    pub color_model: ColorModel,
    pub has_alpha: bool,
    pub is_indexed: bool,
    pub icc_profile: Option<Vec<u8>>,
    pub profile_name: Option<String>,
    /// The XMP packet (UTF-8 XML) when the file carries one: JPEG APP1, PNG `iTXt` `XML:com.adobe.xmp`, WebP `XMP `.
    pub xmp: Option<Vec<u8>>,
    /// The image resource blocks of a JPEG APP13 segment, header included (the IPTC datasets live in resource
    /// 0x0404; `bifrost_core::imgmeta::iptc_from_app13` reads them).
    pub image_resources: Option<Vec<u8>>,
    pub exif: Dict,
    pub tiff: Dict,
    pub gps: Dict,
    pub jfif: Dict,
    pub png: Dict,
    pub gif: Dict,
    pub webp: Dict,
}

impl Properties {
    pub fn new(w: usize, h: usize) -> Self {
        Properties { pixel_width: w, pixel_height: h, depth: 8, orientation: 1, has_alpha: true, ..Default::default() }
    }

    /// Fills `tiff`, `exif`, `gps`, the orientation and the resolution from a TIFF-structured block.
    pub fn absorb_exif(&mut self, block: &[u8]) {
        if let Some(p) = parse_block(block) {
            for (k, v) in p.tiff {
                self.tiff.insert(k, v);
            }
            self.exif.extend(p.exif);
            self.gps.extend(p.gps);
            if let Some(o) = self.tiff.get("Orientation").and_then(|v| v.as_i64()) {
                if (1..=8).contains(&o) {
                    self.orientation = o as u8;
                }
            }
            let unit = self.tiff.get("ResolutionUnit").and_then(|v| v.as_i64()).unwrap_or(2);
            let scale = if unit == 3 { 2.54 } else { 1.0 };
            if let Some(x) = self.tiff.get("XResolution").and_then(|v| v.as_f64()) {
                self.dpi_width.get_or_insert(x * scale);
            }
            if let Some(y) = self.tiff.get("YResolution").and_then(|v| v.as_f64()) {
                self.dpi_height.get_or_insert(y * scale);
            }
        }
    }
}

pub(super) const TIFF_TAGS: &[(u16, &str)] = &[
    (0x010E, "ImageDescription"),
    (0x010F, "Make"),
    (0x0110, "Model"),
    (0x0112, "Orientation"),
    (0x011A, "XResolution"),
    (0x011B, "YResolution"),
    (0x0128, "ResolutionUnit"),
    (0x0131, "Software"),
    (0x0132, "DateTime"),
    (0x013B, "Artist"),
    (0x8298, "Copyright"),
];
pub(super) const EXIF_TAGS: &[(u16, &str)] = &[
    (0x829A, "ExposureTime"),
    (0x829D, "FNumber"),
    (0x8822, "ExposureProgram"),
    (0x8827, "ISOSpeedRatings"),
    (0x9000, "ExifVersion"),
    (0x9003, "DateTimeOriginal"),
    (0x9004, "DateTimeDigitized"),
    (0x9201, "ShutterSpeedValue"),
    (0x9202, "ApertureValue"),
    (0x9204, "ExposureBiasValue"),
    (0x9207, "MeteringMode"),
    (0x9011, "OffsetTimeOriginal"),
    (0x9209, "Flash"),
    (0x920A, "FocalLength"),
    (0x9286, "UserComment"),
    (0xA001, "ColorSpace"),
    (0xA002, "PixelXDimension"),
    (0xA003, "PixelYDimension"),
    (0xA402, "ExposureMode"),
    (0xA403, "WhiteBalance"),
    (0xA405, "FocalLenIn35mmFilm"),
    (0xA433, "LensMake"),
    (0xA434, "LensModel"),
];
pub(super) const GPS_TAGS: &[(u16, &str)] = &[
    (0x00, "VersionID"),
    (0x01, "LatitudeRef"),
    (0x02, "Latitude"),
    (0x03, "LongitudeRef"),
    (0x04, "Longitude"),
    (0x05, "AltitudeRef"),
    (0x06, "Altitude"),
    (0x07, "TimeStamp"),
    (0x0C, "SpeedRef"),
    (0x0D, "Speed"),
    (0x0E, "TrackRef"),
    (0x0F, "Track"),
    (0x10, "ImgDirectionRef"),
    (0x11, "ImgDirection"),
    (0x1D, "DateStamp"),
];
const RATIONAL_TAGS: &[&str] = &["XResolution", "YResolution", "ExposureTime", "FNumber", "FocalLength", "ShutterSpeedValue", "ApertureValue", "ExposureBiasValue", "Altitude", "Latitude", "Longitude", "TimeStamp", "Speed", "Track", "ImgDirection"];
const BYTE_TAGS: &[&str] = &["VersionID", "AltitudeRef"];

fn tag_name(table: &[(u16, &str)], tag: u16) -> String {
    table.iter().find(|t| t.0 == tag).map(|t| t.1.to_string()).unwrap_or_else(|| format!("Tag_{tag:04X}"))
}

fn tag_id(table: &[(u16, &str)], name: &str) -> Option<u16> {
    table.iter().find(|t| t.1 == name).map(|t| t.0).or_else(|| name.strip_prefix("Tag_").and_then(|h| u16::from_str_radix(h, 16).ok()))
}

const TYPE_SIZE: [usize; 13] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8];

/// One directory entry; `at` is the absolute offset of its value bytes.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub tag: u16,
    pub typ: u16,
    pub count: usize,
    pub at: usize,
}

/// A TIFF-structured byte block with its byte order.
pub struct Block<'a> {
    pub d: &'a [u8],
    pub le: bool,
}

impl<'a> Block<'a> {
    /// Checks the header; offsets are relative to the start of `d`.
    pub fn new(d: &'a [u8]) -> Option<Self> {
        let le = match d.get(..4)? {
            [b'I', b'I', 42, 0] => true,
            [b'M', b'M', 0, 42] => false,
            _ => return None,
        };
        Some(Block { d, le })
    }
    pub fn u16(&self, o: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(o..o + 2)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }
    pub fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o + 4)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
    pub fn first_ifd(&self) -> Option<usize> {
        Some(self.u32(4)? as usize)
    }
    /// Entries of the directory at `off` and the offset of the next one (0 for none).
    pub fn ifd(&self, off: usize) -> Option<(Vec<Entry>, usize)> {
        let n = self.u16(off)? as usize;
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let e = off + 2 + i * 12;
            let tag = self.u16(e)?;
            let typ = self.u16(e + 2)?;
            let count = self.u32(e + 4)? as usize;
            let size = TYPE_SIZE.get(typ as usize).copied().unwrap_or(0).checked_mul(count)?;
            let at = if size <= 4 { e + 8 } else { self.u32(e + 8)? as usize };
            if size > 0 && at.checked_add(size)? > self.d.len() {
                continue;
            }
            v.push(Entry { tag, typ, count, at });
        }
        let next = self.u32(off + 2 + n * 12).unwrap_or(0) as usize;
        Some((v, next))
    }
    /// Integer values of a BYTE/SHORT/LONG (or signed) entry.
    pub fn uints(&self, e: &Entry) -> Vec<i64> {
        (0..e.count)
            .filter_map(|i| match e.typ {
                1 | 7 => self.d.get(e.at + i).map(|&b| b as i64),
                6 => self.d.get(e.at + i).map(|&b| b as i8 as i64),
                3 => self.u16(e.at + i * 2).map(|v| v as i64),
                8 => self.u16(e.at + i * 2).map(|v| v as i16 as i64),
                4 => self.u32(e.at + i * 4).map(|v| v as i64),
                9 => self.u32(e.at + i * 4).map(|v| v as i32 as i64),
                _ => None,
            })
            .collect()
    }
    /// Floating values of a RATIONAL/FLOAT/DOUBLE entry.
    pub fn floats(&self, e: &Entry) -> Vec<f64> {
        (0..e.count)
            .filter_map(|i| match e.typ {
                5 => {
                    let (n, d) = (self.u32(e.at + i * 8)? as f64, self.u32(e.at + i * 8 + 4)? as f64);
                    Some(if d == 0.0 { 0.0 } else { n / d })
                }
                10 => {
                    let (n, d) = (self.u32(e.at + i * 8)? as i32 as f64, self.u32(e.at + i * 8 + 4)? as i32 as f64);
                    Some(if d == 0.0 { 0.0 } else { n / d })
                }
                11 => self.u32(e.at + i * 4).map(|v| f32::from_bits(v) as f64),
                12 => {
                    let b: [u8; 8] = self.d.get(e.at + i * 8..e.at + i * 8 + 8)?.try_into().ok()?;
                    Some(f64::from_bits(if self.le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) }))
                }
                _ => None,
            })
            .collect()
    }
    pub fn value(&self, e: &Entry) -> PropValue {
        match e.typ {
            2 => {
                let b = self.d.get(e.at..e.at + e.count).unwrap_or(&[]);
                PropValue::Text(String::from_utf8_lossy(b).trim_end_matches('\0').to_string())
            }
            7 => PropValue::Bytes(self.d.get(e.at..e.at + e.count).unwrap_or(&[]).to_vec()),
            5 | 10 | 11 | 12 => {
                let f = self.floats(e);
                if f.len() == 1 {
                    PropValue::Float(f[0])
                } else {
                    PropValue::Floats(f)
                }
            }
            _ => {
                let u = self.uints(e);
                if u.len() == 1 {
                    PropValue::Int(u[0])
                } else {
                    PropValue::Ints(u)
                }
            }
        }
    }
}

/// The dictionaries read from a block.
#[derive(Default)]
pub struct ParsedBlock {
    pub tiff: Dict,
    pub exif: Dict,
    pub gps: Dict,
}

fn fill(b: &Block, off: usize, table: &[(u16, &str)], out: &mut Dict, subs: &mut Vec<(u16, usize)>) {
    if let Some((entries, _)) = b.ifd(off) {
        for e in entries {
            if e.tag == 0x8769 || e.tag == 0x8825 {
                if let Some(&o) = b.uints(&e).first() {
                    subs.push((e.tag, o as usize));
                }
                continue;
            }
            if e.tag == 0x927C || e.tag == 0x02BC {
                continue; // maker notes and XMP packets are opaque
            }
            // image structure tags of TIFF files are not metadata
            const STRUCTURE: &[u16] = &[254, 255, 256, 257, 258, 259, 262, 263, 264, 265, 266, 269, 273, 277, 278, 279, 284, 317, 320, 322, 323, 324, 325, 338, 339, 34675];
            if table.iter().all(|t| t.0 != e.tag) && STRUCTURE.contains(&e.tag) {
                continue;
            }
            out.insert(tag_name(table, e.tag), b.value(&e));
        }
    }
}

/// Reads IFD0 (TIFF tags) plus the EXIF and GPS directories it points at.
pub fn parse_block(d: &[u8]) -> Option<ParsedBlock> {
    let b = Block::new(d)?;
    let mut p = ParsedBlock::default();
    let mut subs = Vec::new();
    fill(&b, b.first_ifd()?, TIFF_TAGS, &mut p.tiff, &mut subs);
    for (tag, off) in subs {
        let mut none = Vec::new();
        if tag == 0x8769 {
            fill(&b, off, EXIF_TAGS, &mut p.exif, &mut none);
        } else {
            fill(&b, off, GPS_TAGS, &mut p.gps, &mut none);
        }
    }
    Some(p)
}

pub(super) struct OutEntry {
    pub(super) tag: u16,
    pub(super) typ: u16,
    pub(super) count: u32,
    pub(super) data: Vec<u8>,
}

fn encode_entry(table: &[(u16, &str)], name: &str, v: &PropValue) -> Option<OutEntry> {
    let tag = tag_id(table, name)?;
    let rational = RATIONAL_TAGS.contains(&name);
    let le16 = |x: u16| x.to_le_bytes();
    let rat = |f: f64| -> (u16, Vec<u8>) {
        let den: f64 = if f.abs() < 4000.0 { 1_000_000.0 } else { 1.0 };
        let num = (f * den).round();
        let mut b = Vec::new();
        if num < 0.0 {
            b.extend((num as i32).to_le_bytes());
            b.extend((den as i32).to_le_bytes());
            (10, b)
        } else {
            b.extend((num as u32).to_le_bytes());
            b.extend((den as u32).to_le_bytes());
            (5, b)
        }
    };
    match v {
        PropValue::Text(s) => {
            let mut data = s.as_bytes().to_vec();
            data.push(0);
            Some(OutEntry { tag, typ: 2, count: data.len() as u32, data })
        }
        PropValue::Bytes(b) => Some(OutEntry { tag, typ: 7, count: b.len() as u32, data: b.clone() }),
        PropValue::Int(_) | PropValue::Float(_) | PropValue::Ints(_) | PropValue::Floats(_) if rational => {
            let list: Vec<f64> = match v {
                PropValue::Int(i) => vec![*i as f64],
                PropValue::Float(f) => vec![*f],
                PropValue::Ints(l) => l.iter().map(|&i| i as f64).collect(),
                PropValue::Floats(l) => l.clone(),
                _ => unreachable!(),
            };
            let mut data = Vec::new();
            let mut typ = 5;
            for f in &list {
                let (t, b) = rat(*f);
                typ = t;
                data.extend(b);
            }
            Some(OutEntry { tag, typ, count: list.len() as u32, data })
        }
        PropValue::Int(_) | PropValue::Ints(_) => {
            let list: Vec<i64> = match v {
                PropValue::Int(i) => vec![*i],
                PropValue::Ints(l) => l.clone(),
                _ => unreachable!(),
            };
            if BYTE_TAGS.contains(&name) {
                return Some(OutEntry { tag, typ: 1, count: list.len() as u32, data: list.iter().map(|&i| i as u8).collect() });
            }
            if list.iter().all(|&i| (0..=65535).contains(&i)) {
                Some(OutEntry { tag, typ: 3, count: list.len() as u32, data: list.iter().flat_map(|&i| le16(i as u16)).collect() })
            } else {
                Some(OutEntry { tag, typ: 4, count: list.len() as u32, data: list.iter().flat_map(|&i| (i as u32).to_le_bytes()).collect() })
            }
        }
        PropValue::Float(f) => {
            let (typ, data) = rat(*f);
            Some(OutEntry { tag, typ, count: 1, data })
        }
        PropValue::Floats(l) => {
            let mut data = Vec::new();
            let mut typ = 5;
            for f in l {
                let (t, b) = rat(*f);
                typ = t;
                data.extend(b);
            }
            Some(OutEntry { tag, typ, count: l.len() as u32, data })
        }
        PropValue::Dict(_) => None,
    }
}

/// One directory with its out-of-line values, placed at absolute offset `start`; the next-directory field is 0.
pub(super) fn ifd_bytes(mut entries: Vec<OutEntry>, start: usize) -> Vec<u8> {
    entries.sort_by_key(|e| e.tag);
    let n = entries.len();
    let table = 2 + n * 12 + 4;
    let mut head = Vec::with_capacity(table);
    let mut tail: Vec<u8> = Vec::new();
    head.extend((n as u16).to_le_bytes());
    for e in &entries {
        head.extend(e.tag.to_le_bytes());
        head.extend(e.typ.to_le_bytes());
        head.extend(e.count.to_le_bytes());
        if e.data.len() <= 4 {
            let mut v = e.data.clone();
            v.resize(4, 0);
            head.extend(v);
        } else {
            head.extend(((start + table + tail.len()) as u32).to_le_bytes());
            tail.extend(&e.data);
            if tail.len() % 2 == 1 {
                tail.push(0);
            }
        }
    }
    head.extend(0u32.to_le_bytes());
    head.extend(tail);
    head
}

pub(super) fn entries_of(table: &[(u16, &str)], d: &Dict) -> Vec<OutEntry> {
    d.iter().filter_map(|(k, v)| encode_entry(table, k, v)).collect()
}

/// Builds a little-endian TIFF-structured block from the dictionaries. `None` when all are empty.
pub fn build_block(tiff: &Dict, exif: &Dict, gps: &Dict) -> Option<Vec<u8>> {
    if tiff.is_empty() && exif.is_empty() && gps.is_empty() {
        return None;
    }
    let ptr = |tag: u16, v: u32| OutEntry { tag, typ: 4, count: 1, data: v.to_le_bytes().to_vec() };
    let mk0 = |ex: u32, gp: u32| {
        let mut e = entries_of(TIFF_TAGS, tiff);
        if !exif.is_empty() {
            e.push(ptr(0x8769, ex));
        }
        if !gps.is_empty() {
            e.push(ptr(0x8825, gp));
        }
        e
    };
    let len0 = ifd_bytes(mk0(0, 0), 8).len();
    let exif_off = 8 + len0;
    let exif_ifd = if exif.is_empty() { Vec::new() } else { ifd_bytes(entries_of(EXIF_TAGS, exif), exif_off) };
    let gps_off = exif_off + exif_ifd.len();
    let gps_ifd = if gps.is_empty() { Vec::new() } else { ifd_bytes(entries_of(GPS_TAGS, gps), gps_off) };
    let mut out = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    out.extend(ifd_bytes(mk0(exif_off as u32, gps_off as u32), 8));
    out.extend(exif_ifd);
    out.extend(gps_ifd);
    Some(out)
}

/// Like [`build_block`], with a JPEG thumbnail in a second directory (IFD1: Compression 6, JPEGInterchangeFormat and
/// its length), the place camera files keep the small preview. Without a thumbnail this is `build_block`.
pub fn build_block_with_thumbnail(tiff: &Dict, exif: &Dict, gps: &Dict, thumbnail: Option<&[u8]>) -> Option<Vec<u8>> {
    let Some(thumb) = thumbnail.filter(|t| t.starts_with(&[0xFF, 0xD8])) else {
        return build_block(tiff, exif, gps);
    };
    let mut out = match build_block(tiff, exif, gps) {
        Some(b) => b,
        // No other metadata: an empty IFD0 that only links to IFD1.
        None => vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    };
    let ifd1_off = out.len();
    let n0 = u16::from_le_bytes([out[8], out[9]]) as usize;
    let next_at = 8 + 2 + n0 * 12;
    out[next_at..next_at + 4].copy_from_slice(&(ifd1_off as u32).to_le_bytes());
    let long = |tag: u16, v: u32| OutEntry { tag, typ: 4, count: 1, data: v.to_le_bytes().to_vec() };
    // Three entries whose values fit in place: 2 + 3 * 12 + 4 bytes, then the JPEG bytes.
    let thumb_off = ifd1_off + 2 + 3 * 12 + 4;
    let entries = vec![OutEntry { tag: 0x0103, typ: 3, count: 1, data: 6u16.to_le_bytes().to_vec() }, long(0x0201, thumb_off as u32), long(0x0202, thumb.len() as u32)];
    out.extend(ifd_bytes(entries, ifd1_off));
    out.extend(thumb);
    Some(out)
}

/// The JPEG thumbnail that IFD1 of a TIFF-structured block points at (JPEGInterchangeFormat and its length).
/// `None` when the block has no second directory, the directory has no JPEG, or the bytes are not a JPEG.
pub fn thumbnail_of_block(d: &[u8]) -> Option<&[u8]> {
    let b = Block::new(d)?;
    let (_, next) = b.ifd(b.first_ifd()?)?;
    if next == 0 {
        return None;
    }
    let (entries, _) = b.ifd(next)?;
    let value = |tag: u16| entries.iter().find(|e| e.tag == tag).and_then(|e| b.uints(e).first().copied());
    let (off, len) = (value(0x0201)? as usize, value(0x0202)? as usize);
    let jpeg = d.get(off..off.checked_add(len)?)?;
    jpeg.starts_with(&[0xFF, 0xD8]).then_some(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_round_trips() {
        let mut tiff = Dict::new();
        tiff.insert("Make".into(), PropValue::Text("Acme".into()));
        tiff.insert("Orientation".into(), PropValue::Int(6));
        tiff.insert("XResolution".into(), PropValue::Float(300.0));
        tiff.insert("ResolutionUnit".into(), PropValue::Int(2));
        let mut exif = Dict::new();
        exif.insert("ExposureTime".into(), PropValue::Float(0.004));
        exif.insert("ISOSpeedRatings".into(), PropValue::Int(200));
        exif.insert("DateTimeOriginal".into(), PropValue::Text("2026:10:13 10:00:00".into()));
        let mut gps = Dict::new();
        gps.insert("LatitudeRef".into(), PropValue::Text("N".into()));
        gps.insert("Latitude".into(), PropValue::Floats(vec![64.0, 8.0, 30.5]));
        let blob = build_block(&tiff, &exif, &gps).unwrap();
        let p = parse_block(&blob).unwrap();
        assert_eq!(p.tiff["Make"], PropValue::Text("Acme".into()));
        assert_eq!(p.tiff["Orientation"], PropValue::Int(6));
        assert_eq!(p.tiff["XResolution"].as_f64(), Some(300.0));
        assert_eq!(p.exif["ISOSpeedRatings"], PropValue::Int(200));
        assert!((p.exif["ExposureTime"].as_f64().unwrap() - 0.004).abs() < 1e-9);
        assert_eq!(p.exif["DateTimeOriginal"].as_str(), Some("2026:10:13 10:00:00"));
        assert_eq!(p.gps["Latitude"], PropValue::Floats(vec![64.0, 8.0, 30.5]));
        let mut props = Properties::new(1, 1);
        props.absorb_exif(&blob);
        assert_eq!(props.orientation, 6);
        assert_eq!(props.dpi_width, Some(300.0));
    }

    #[test]
    fn a_thumbnail_rides_in_ifd1_and_comes_back() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9, 1, 2, 3];
        let mut tiff = Dict::new();
        tiff.insert("Orientation".into(), PropValue::Int(6));
        let mut gps = Dict::new();
        gps.insert("LatitudeRef".into(), PropValue::Text("N".into()));
        let blob = build_block_with_thumbnail(&tiff, &Dict::new(), &gps, Some(&jpeg)).unwrap();
        assert_eq!(thumbnail_of_block(&blob), Some(jpeg.as_slice()));
        let p = parse_block(&blob).unwrap();
        assert_eq!(p.tiff["Orientation"], PropValue::Int(6));
        assert_eq!(p.gps["LatitudeRef"].as_str(), Some("N"));
        assert!(!p.tiff.contains_key("Tag_0201"), "IFD1 is not read as image metadata");
        let only = build_block_with_thumbnail(&Dict::new(), &Dict::new(), &Dict::new(), Some(&jpeg)).unwrap();
        assert_eq!(thumbnail_of_block(&only), Some(jpeg.as_slice()));
        assert!(build_block_with_thumbnail(&Dict::new(), &Dict::new(), &Dict::new(), Some(b"not a jpeg")).is_none());
        assert_eq!(thumbnail_of_block(&build_block(&tiff, &Dict::new(), &Dict::new()).unwrap()), None);
    }

    #[test]
    fn empty_and_garbage_blocks() {
        assert!(build_block(&Dict::new(), &Dict::new(), &Dict::new()).is_none());
        assert!(parse_block(b"nope").is_none());
        assert!(parse_block(&[b'I', b'I', 42, 0, 255, 255, 255, 255]).unwrap().tiff.is_empty());
    }

    #[test]
    fn big_endian_blocks_are_read() {
        // MM header, one entry: Orientation SHORT 3.
        let blob = [b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 3, 0, 0, 0, 0, 0, 0];
        let p = parse_block(&blob).unwrap();
        assert_eq!(p.tiff["Orientation"], PropValue::Int(3));
    }

    #[test]
    fn unknown_tags_keep_a_hex_name() {
        let mut t = Dict::new();
        t.insert("Tag_C4A5".into(), PropValue::Int(7));
        let p = parse_block(&build_block(&t, &Dict::new(), &Dict::new()).unwrap()).unwrap();
        assert_eq!(p.tiff["Tag_C4A5"], PropValue::Int(7));
    }
}
