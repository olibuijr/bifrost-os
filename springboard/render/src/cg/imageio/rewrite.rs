//! Lossless metadata edits: a new file with the same compressed pixels and changed metadata, the way the reference
//! destination copies an image source. JPEG scan data, PNG image chunks and WebP bitstreams are copied byte for
//! byte; only the metadata containers (the EXIF block, the XMP packet and, in JPEG, the APP13 image resources) are
//! rebuilt. JPEG, PNG and WebP are supported; TIFF keeps its metadata in the same directories as the image
//! structure and is refused, as are the formats without metadata (GIF, BMP, ICO).
//!
//! What is kept: every tag of IFD0, the EXIF and the GPS directories (unknown tags by number), the EXIF IFD1
//! preview, and every other segment or chunk. What is dropped when the EXIF block is rebuilt: maker notes and
//! the pointers to directories this reader does not follow (interoperability, sub-IFDs), whose offsets would no
//! longer be right.

use super::encode::{chunk, XMP_APP1_HEADER};
use super::exif::{self, Dict, PropValue, Properties};
use super::{jpeg, png, Format, ImageSource};

/// Metadata to merge into, or put in place of, a file's own. An empty `xmp` or `image_resources` removes that
/// container.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetadataSet {
    pub tiff: Dict,
    pub exif: Dict,
    pub gps: Dict,
    pub xmp: Option<Vec<u8>>,
    /// A JPEG APP13 payload (header included).
    pub image_resources: Option<Vec<u8>>,
}

/// How [`copy_image_source`] changes the metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CopyOptions {
    /// New metadata. With `merge` its tags are added to the file's (a tag in both takes the new value); without, it
    /// replaces the file's EXIF, TIFF, GPS, XMP and IPTC metadata, except the orientation, which describes the
    /// stored pixels and is kept unless the set or `orientation` gives a new one.
    pub metadata: Option<MetadataSet>,
    pub merge: bool,
    /// Remove the GPS directory and the `exif:GPS*` properties of the XMP packet (a packet that cannot be cleaned
    /// is dropped whole, so no location survives).
    pub exclude_gps: bool,
    /// Remove the XMP packet.
    pub exclude_xmp: bool,
    /// `YYYY:MM:DD HH:MM:SS`: written as the TIFF DateTime and the EXIF DateTimeOriginal and DateTimeDigitized.
    pub date_time: Option<String>,
    /// A new orientation 1..=8 (the pixels are not touched; viewers turn them).
    pub orientation: Option<u8>,
}

/// Why a copy was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// The data is not a picture format this module reads.
    UnknownType,
    /// A picture format whose metadata cannot be rewritten without re-encoding (TIFF) or that has none.
    Unsupported(Format),
    /// The file's structure does not parse cleanly, so it is not copied.
    Malformed,
    /// An orientation outside 1..=8 or a date not in the EXIF form.
    BadOptions,
    /// The new metadata does not fit its container (a JPEG segment holds at most 65533 bytes).
    TooLarge,
}

/// Tags whose values are offsets into the old block; they would point at the wrong bytes after a rebuild.
const OFFSET_TAGS: &[&str] = &["Tag_A005", "Tag_014A", "Tag_0201", "Tag_0202", "Tag_0111", "Tag_0117"];

/// Whether `text` has the EXIF date form `YYYY:MM:DD HH:MM:SS`.
fn is_exif_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 | 13 | 16 => c == b':',
            10 => c == b' ',
            _ => c.is_ascii_digit(),
        })
}

/// Writes `data` again with its metadata changed as `o` says. The compressed pixels are not decoded or re-encoded.
pub fn copy_image_source(data: &[u8], o: &CopyOptions) -> Result<Vec<u8>, CopyError> {
    if o.orientation.is_some_and(|v| !(1..=8).contains(&v)) || o.date_time.as_deref().is_some_and(|d| !is_exif_date(d)) {
        return Err(CopyError::BadOptions);
    }
    let format = Format::detect(data).ok_or(CopyError::UnknownType)?;
    if !matches!(format, Format::Jpeg | Format::Png | Format::Webp) {
        return Err(CopyError::Unsupported(format));
    }
    let src = ImageSource::new(data.to_vec());
    let props = src.properties_at(0).ok_or(CopyError::Malformed)?;
    let old_block = src.exif_block();
    let preview = old_block.and_then(exif::thumbnail_of_block).map(<[u8]>::to_vec);
    let parsed = old_block.and_then(exif::parse_block).unwrap_or_default();
    let (mut tiff, mut ex, mut gps) = (parsed.tiff, parsed.exif, parsed.gps);
    let mut xmp = props.xmp.clone();
    let mut resources = props.image_resources.clone();
    let mut xmp_changed = false;
    if let Some(m) = &o.metadata {
        if !o.merge {
            let orientation = tiff.get("Orientation").cloned();
            tiff.clear();
            ex.clear();
            gps.clear();
            if let Some(v) = orientation {
                tiff.insert("Orientation".into(), v);
            }
            xmp = None;
            resources = None;
            xmp_changed = true;
        }
        tiff.extend(m.tiff.clone());
        ex.extend(m.exif.clone());
        gps.extend(m.gps.clone());
        if let Some(x) = &m.xmp {
            xmp = (!x.is_empty()).then(|| x.clone());
            xmp_changed = true;
        }
        if let Some(r) = &m.image_resources {
            resources = (!r.is_empty()).then(|| r.clone());
        }
    }
    for d in [&mut tiff, &mut ex, &mut gps] {
        d.retain(|k, _| !OFFSET_TAGS.contains(&k.as_str()));
    }
    if let Some(v) = o.orientation {
        tiff.insert("Orientation".into(), PropValue::Int(v as i64));
    }
    if let Some(d) = &o.date_time {
        tiff.insert("DateTime".into(), PropValue::Text(d.clone()));
        ex.insert("DateTimeOriginal".into(), PropValue::Text(d.clone()));
        ex.insert("DateTimeDigitized".into(), PropValue::Text(d.clone()));
    }
    if o.exclude_gps {
        gps.clear();
        if let Some(x) = xmp.take() {
            xmp = strip_xmp_gps(&x);
            xmp_changed |= xmp.as_deref() != Some(x.as_slice());
        }
    }
    if o.exclude_xmp {
        xmp_changed |= xmp.is_some();
        xmp = None;
    }
    let block = exif::build_block_with_thumbnail(&tiff, &ex, &gps, preview.as_deref());
    match format {
        Format::Jpeg => rewrite_jpeg(data, block.as_deref(), xmp.as_deref(), resources.as_deref(), !xmp_changed),
        Format::Png => rewrite_png(data, block.as_deref(), xmp.as_deref()),
        _ => rewrite_webp(data, &props, block.as_deref(), xmp.as_deref()),
    }
}

/// Removes the `exif:GPS*` elements and attributes of an XMP packet. `None` when the packet is not UTF-8 or a GPS
/// property is left that the scan could not take out: the caller then drops the packet rather than keep a location.
pub fn strip_xmp_gps(packet: &[u8]) -> Option<Vec<u8>> {
    let mut s = std::str::from_utf8(packet).ok()?.to_string();
    while let Some(start) = s.find("<exif:GPS") {
        let after = &s[start + 1..];
        let name_len = after.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(after.len());
        let name = after[..name_len].to_string();
        let gt = start + s[start..].find('>')?;
        let end = if s[..gt].ends_with('/') {
            gt + 1
        } else {
            let close = format!("</{name}>");
            gt + s[gt..].find(&close)? + close.len()
        };
        s.replace_range(start..end, "");
    }
    let mut from = 0;
    while let Some(p) = s[from..].find("exif:GPS") {
        let at = from + p;
        let before = s[..at].chars().last();
        let rest = &s[at..];
        let attr = rest.find('=').filter(|&e| rest[..e].chars().all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '_'));
        let quote = attr.and_then(|e| rest[e + 1..].chars().next()).filter(|q| *q == '"' || *q == '\'');
        match (before.filter(|c| c.is_whitespace()), attr, quote) {
            (Some(ws), Some(e), Some(q)) => {
                let value_start = at + e + 2;
                let value_end = value_start + s[value_start..].find(q)?;
                let ws_start = at - ws.len_utf8();
                s.replace_range(ws_start..value_end + 1, "");
                from = ws_start;
            }
            _ => from = at + 1,
        }
    }
    (!s.contains("exif:GPS")).then(|| s.into_bytes())
}

fn push_segment(out: &mut Vec<u8>, marker: u8, parts: &[&[u8]]) -> Result<(), CopyError> {
    let len: usize = parts.iter().map(|p| p.len()).sum::<usize>() + 2;
    if len > 65535 {
        return Err(CopyError::TooLarge);
    }
    out.extend([0xFF, marker]);
    out.extend((len as u16).to_be_bytes());
    for p in parts {
        out.extend(*p);
    }
    Ok(())
}

/// The segments a JPEG rewrite replaces: the EXIF and XMP APP1 segments and the APP13 image resources. Extended XMP
/// segments go too unless the main packet is unchanged.
fn is_metadata_segment(marker: u8, body: &[u8], keep_extended_xmp: bool) -> bool {
    match marker {
        0xE1 => body.starts_with(b"Exif\0\0") || body.starts_with(XMP_APP1_HEADER) || (!keep_extended_xmp && body.starts_with(b"http://ns.adobe.com/xmp/extension/\0")),
        0xED => body.starts_with(b"Photoshop 3.0\0"),
        _ => false,
    }
}

fn rewrite_jpeg(data: &[u8], block: Option<&[u8]>, xmp: Option<&[u8]>, resources: Option<&[u8]>, keep_extended_xmp: bool) -> Result<Vec<u8>, CopyError> {
    let (segs, scan) = jpeg::header_segments(data).ok_or(CopyError::Malformed)?;
    let mut out = vec![0xFF, 0xD8];
    // JFIF and its extension stay first, as their specification asks; the metadata follows.
    let lead = segs.iter().take_while(|s| s.marker == 0xE0).count();
    for s in &segs[..lead] {
        out.extend(&data[s.start..s.end]);
    }
    if let Some(b) = block {
        push_segment(&mut out, 0xE1, &[b"Exif\0\0", b])?;
    }
    if let Some(x) = xmp {
        push_segment(&mut out, 0xE1, &[XMP_APP1_HEADER, x])?;
    }
    if let Some(r) = resources {
        push_segment(&mut out, 0xED, &[r])?;
    }
    for s in &segs[lead..] {
        if !is_metadata_segment(s.marker, s.body(data), keep_extended_xmp) {
            out.extend(&data[s.start..s.end]);
        }
    }
    out.extend(&data[scan..]);
    Ok(out)
}

fn rewrite_png(data: &[u8], block: Option<&[u8]>, xmp: Option<&[u8]>) -> Result<Vec<u8>, CopyError> {
    let spans = png::chunk_spans(data).ok_or(CopyError::Malformed)?;
    let mut out = png::SIGNATURE.to_vec();
    let mut placed = false;
    for (ty, start, end) in spans {
        let body = &data[start + 8..end - 4];
        if &ty == b"eXIf" || (&ty == b"iTXt" && body.starts_with(b"XML:com.adobe.xmp\0")) {
            continue;
        }
        // eXIf must come before the image data; the packet goes with it.
        if !placed && matches!(&ty, b"IDAT" | b"IEND") {
            if let Some(b) = block {
                chunk(&mut out, b"eXIf", b);
            }
            if let Some(x) = xmp {
                let mut d = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
                d.extend(x);
                chunk(&mut out, b"iTXt", &d);
            }
            placed = true;
        }
        out.extend(&data[start..end]);
    }
    Ok(out)
}

fn rewrite_webp(data: &[u8], props: &Properties, block: Option<&[u8]>, xmp: Option<&[u8]>) -> Result<Vec<u8>, CopyError> {
    if data.get(..4) != Some(b"RIFF".as_slice()) || data.get(8..12) != Some(b"WEBP".as_slice()) {
        return Err(CopyError::Malformed);
    }
    let mut chunks: Vec<([u8; 4], &[u8])> = Vec::new();
    let mut p = 12;
    while p + 8 <= data.len() {
        let ty = [data[p], data[p + 1], data[p + 2], data[p + 3]];
        let n = u32::from_le_bytes([data[p + 4], data[p + 5], data[p + 6], data[p + 7]]) as usize;
        let end = (p + 8).checked_add(n).ok_or(CopyError::Malformed)?;
        if end > data.len() {
            return Err(CopyError::Malformed);
        }
        chunks.push((ty, &data[p + 8..end]));
        p = end + (n & 1);
    }
    let vp8x = chunks.iter().find(|c| &c.0 == b"VP8X").map(|c| c.1.to_vec());
    let mut body = Vec::new();
    let mut put = |ty: &[u8; 4], d: &[u8]| {
        body.extend(ty);
        body.extend((d.len() as u32).to_le_bytes());
        body.extend(d);
        if d.len() % 2 == 1 {
            body.push(0);
        }
    };
    if vp8x.is_some() || block.is_some() || xmp.is_some() {
        let mut x = match vp8x {
            Some(x) if x.len() >= 10 => x,
            Some(_) => return Err(CopyError::Malformed),
            None => {
                // A simple file becomes an extended one: canvas size and alpha come from the bitstream header.
                let (w, h) = (props.pixel_width.saturating_sub(1), props.pixel_height.saturating_sub(1));
                let mut v = vec![0u8; 10];
                v[4..7].copy_from_slice(&(w as u32).to_le_bytes()[..3]);
                v[7..10].copy_from_slice(&(h as u32).to_le_bytes()[..3]);
                if props.has_alpha {
                    v[0] |= 0x10;
                }
                v
            }
        };
        let exif_flag: u8 = if block.is_some() { 0x08 } else { 0 };
        let xmp_flag: u8 = if xmp.is_some() { 0x04 } else { 0 };
        x[0] = (x[0] & !0x0C) | exif_flag | xmp_flag;
        put(b"VP8X", &x);
    }
    for (ty, d) in &chunks {
        if !matches!(ty, b"VP8X" | b"EXIF" | b"XMP ") {
            put(ty, d);
        }
    }
    if let Some(b) = block {
        put(b"EXIF", b);
    }
    if let Some(x) = xmp {
        put(b"XMP ", x);
    }
    let mut out = b"RIFF".to_vec();
    out.extend(((body.len() + 4) as u32).to_le_bytes());
    out.extend(b"WEBP");
    out.extend(body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::{ImageDestination, WriteOptions};
    use super::*;
    use crate::cg::image::CgImage;

    fn picture() -> CgImage {
        let px: Vec<u8> = (0..12 * 8).flat_map(|i| [(i * 9) as u8, (i * 5) as u8, 70, 255]).collect();
        CgImage::from_rgba8(12, 8, px).unwrap()
    }

    const XMP_WITH_GPS: &str = "<x:xmpmeta><rdf:RDF><rdf:Description xmlns:exif=\"http://ns.adobe.com/exif/1.0/\" exif:GPSLatitude=\"64,8.5N\" dc:format=\"image/jpeg\">\n<exif:GPSLongitude>21,56.4W</exif:GPSLongitude><exif:GPSAltitude rdf:resource=\"x\"/><dc:title>Bay</dc:title></rdf:Description></rdf:RDF></x:xmpmeta>";

    fn located(format: Format) -> Vec<u8> {
        let mut gps = Dict::new();
        gps.insert("LatitudeRef".into(), PropValue::Text("N".into()));
        gps.insert("Latitude".into(), PropValue::Floats(vec![64.0, 8.0, 30.0]));
        let mut tiff = Dict::new();
        tiff.insert("Make".into(), PropValue::Text("Acme".into()));
        let mut d = ImageDestination::new(format, 1).unwrap();
        d.embed_thumbnail = true;
        d.add_image(&picture(), Some(WriteOptions { tiff, gps, orientation: Some(3), xmp: Some(XMP_WITH_GPS.as_bytes().to_vec()), ..Default::default() }));
        d.finalize().unwrap()
    }

    #[test]
    fn gps_is_removed_and_everything_else_kept_without_touching_the_pixels() {
        for format in [Format::Jpeg, Format::Png, Format::Webp] {
            let original = located(format);
            let out = copy_image_source(&original, &CopyOptions { exclude_gps: true, ..Default::default() }).unwrap();
            let p = ImageSource::new(out.clone()).properties_at(0).unwrap();
            assert!(p.gps.is_empty(), "{format:?}");
            assert_eq!(p.tiff["Make"].as_str(), Some("Acme"), "{format:?}");
            assert_eq!(p.orientation, 3, "{format:?}");
            let packet = String::from_utf8(p.xmp.clone().unwrap()).unwrap();
            assert!(!packet.contains("GPS") && packet.contains("<dc:title>Bay</dc:title>") && packet.contains("dc:format=\"image/jpeg\""), "{format:?}: {packet}");
            let before = ImageSource::new(original).image_at(0).unwrap().to_rgba8();
            assert_eq!(ImageSource::new(out).image_at(0).unwrap().to_rgba8(), before, "{format:?}: same pixels");
        }
    }

    #[test]
    fn the_jpeg_scan_and_preview_are_copied_byte_for_byte() {
        let original = located(Format::Jpeg);
        let out = copy_image_source(&original, &CopyOptions { orientation: Some(6), date_time: Some("2026:10:24 09:00:00".into()), ..Default::default() }).unwrap();
        let scan = |d: &[u8]| d[jpeg::header_segments(d).unwrap().1..].to_vec();
        assert_eq!(scan(&out), scan(&original));
        let s = ImageSource::new(out);
        let p = s.properties_at(0).unwrap();
        assert_eq!(p.orientation, 6);
        assert_eq!(p.exif["DateTimeOriginal"].as_str(), Some("2026:10:24 09:00:00"));
        assert_eq!(p.tiff["DateTime"].as_str(), Some("2026:10:24 09:00:00"));
        assert_eq!(p.gps["LatitudeRef"].as_str(), Some("N"), "GPS kept unless excluded");
        assert_eq!(s.embedded_thumbnail(0), ImageSource::new(original).embedded_thumbnail(0), "the preview rides along");
    }

    #[test]
    fn replacing_drops_the_old_metadata_but_keeps_the_orientation() {
        let mut exif_set = Dict::new();
        exif_set.insert("DateTimeOriginal".into(), PropValue::Text("2020:01:02 03:04:05".into()));
        let set = MetadataSet { exif: exif_set, ..Default::default() };
        let out = copy_image_source(&located(Format::Png), &CopyOptions { metadata: Some(set.clone()), ..Default::default() }).unwrap();
        let p = ImageSource::new(out).properties_at(0).unwrap();
        assert!(p.gps.is_empty() && !p.tiff.contains_key("Make") && p.xmp.is_none());
        assert_eq!(p.orientation, 3);
        assert_eq!(p.exif["DateTimeOriginal"].as_str(), Some("2020:01:02 03:04:05"));
        let merged = copy_image_source(&located(Format::Png), &CopyOptions { metadata: Some(set), merge: true, ..Default::default() }).unwrap();
        let m = ImageSource::new(merged).properties_at(0).unwrap();
        assert_eq!(m.tiff["Make"].as_str(), Some("Acme"));
        assert_eq!(m.exif["DateTimeOriginal"].as_str(), Some("2020:01:02 03:04:05"));
    }

    #[test]
    fn a_simple_webp_becomes_extended_when_it_gets_metadata() {
        let mut d = ImageDestination::new(Format::Webp, 1).unwrap();
        d.add_image(&picture(), None);
        let simple = d.finalize().unwrap();
        assert_eq!(&simple[12..16], b"VP8L");
        let out = copy_image_source(&simple, &CopyOptions { orientation: Some(8), ..Default::default() }).unwrap();
        assert_eq!(&out[12..16], b"VP8X");
        let p = ImageSource::new(out.clone()).properties_at(0).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height, p.orientation), (12, 8, 8));
        assert_eq!(u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize, out.len() - 8, "the RIFF size is right");
        assert_eq!(ImageSource::new(out).image_at(0).unwrap().to_rgba8(), picture().to_rgba8());
    }

    #[test]
    fn xmp_can_be_excluded_and_bad_requests_are_refused() {
        let out = copy_image_source(&located(Format::Jpeg), &CopyOptions { exclude_xmp: true, ..Default::default() }).unwrap();
        assert!(ImageSource::new(out).properties_at(0).unwrap().xmp.is_none());
        let tiff = {
            let mut d = ImageDestination::new(Format::Tiff, 1).unwrap();
            d.add_image(&picture(), None);
            d.finalize().unwrap()
        };
        assert_eq!(copy_image_source(&tiff, &CopyOptions::default()), Err(CopyError::Unsupported(Format::Tiff)));
        assert_eq!(copy_image_source(b"not a picture", &CopyOptions::default()), Err(CopyError::UnknownType));
        assert_eq!(copy_image_source(&located(Format::Jpeg), &CopyOptions { orientation: Some(9), ..Default::default() }), Err(CopyError::BadOptions));
        assert_eq!(copy_image_source(&located(Format::Jpeg), &CopyOptions { date_time: Some("today".into()), ..Default::default() }), Err(CopyError::BadOptions));
        let mut cut = located(Format::Png);
        cut.truncate(cut.len() - 6);
        assert_eq!(copy_image_source(&cut, &CopyOptions::default()), Err(CopyError::Malformed));
    }

    #[test]
    fn gps_attributes_and_elements_leave_the_packet() {
        let cleaned = String::from_utf8(strip_xmp_gps(XMP_WITH_GPS.as_bytes()).unwrap()).unwrap();
        assert!(!cleaned.contains("GPS"), "{cleaned}");
        assert!(cleaned.contains("xmlns:exif=") && cleaned.contains("<dc:title>Bay</dc:title>"));
        assert_eq!(strip_xmp_gps(&[0xFF, 0xFE]), None, "not UTF-8: dropped");
        assert_eq!(strip_xmp_gps(b"<a exif:GPSLatitude=unquoted/>"), None, "a property the scan cannot take out drops the packet");
    }
}
