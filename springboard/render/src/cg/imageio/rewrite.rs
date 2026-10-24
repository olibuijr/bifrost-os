//! Lossless metadata edits: a new file with the same compressed pixels and changed metadata, the way the reference
//! destination copies an image source. JPEG scan data, PNG image chunks and WebP bitstreams are copied byte for
//! byte; only the metadata containers (the EXIF block, the XMP packet and, in JPEG, the APP13 image resources) are
//! rebuilt. JPEG, PNG and WebP are supported; TIFF keeps its metadata in the same directories as the image
//! structure and is refused, as are the formats without metadata (GIF, BMP, ICO).
//!
//! What is kept: every tag of IFD0, the EXIF and the GPS directories (unknown tags by number), the EXIF IFD1
//! preview, and every other segment or chunk. An edit that only sets the orientation and the capture date (or
//! changes nothing in EXIF) patches those values in the old block, so the block stays byte for byte otherwise,
//! maker notes and offset tags included, as the reference keeps them. Only when the EXIF block has to be rebuilt
//! (tags added, replaced or removed, GPS removal, a value the old block lacks or stores another way) are maker
//! notes and the pointers to directories this reader does not follow (interoperability, sub-IFDs) dropped, since
//! their offsets would no longer be right. The XMP packet is kept as it is unless the edit replaces it, removes it
//! or takes its GPS properties out; a new orientation is also written into its `tiff:Orientation` when it has one.

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
    if let (Some(v), Some(x)) = (o.orientation, xmp.as_mut()) {
        set_xmp_orientation(x, v);
    }
    // Nothing but the orientation and the dates changes in EXIF: patch them in the old block, which keeps maker
    // notes and offset tags. A value the block lacks, or stores another way, rebuilds it instead.
    let exif_untouched = !o.exclude_gps && o.metadata.as_ref().is_none_or(|m| o.merge && m.tiff.is_empty() && m.exif.is_empty() && m.gps.is_empty());
    let patched = old_block.filter(|_| exif_untouched).and_then(|b| patch_in_place(b, o.orientation, o.date_time.as_deref()));
    let block = match patched {
        Some(b) => Some(b),
        None => exif::build_block_with_thumbnail(&tiff, &ex, &gps, preview.as_deref()),
    };
    match format {
        Format::Jpeg => rewrite_jpeg(data, block.as_deref(), xmp.as_deref(), resources.as_deref(), !xmp_changed),
        Format::Png => rewrite_png(data, block.as_deref(), xmp.as_deref()),
        _ => rewrite_webp(data, &props, block.as_deref(), xmp.as_deref()),
    }
}

/// Removes the `exif:GPS*` elements and attributes of an XMP packet. `None` when the packet is not UTF-8 or a GPS
/// property is left that the scan could not take out: the caller then drops the packet rather than keep a location.
pub fn strip_xmp_gps(packet: &[u8]) -> Option<Vec<u8>> {
    strip_xmp_properties(packet, &|name: &str| name.starts_with("exif:GPS")).filter(|s| !s.windows(8).any(|w| w == b"exif:GPS"))
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.')
}

/// The XML name that starts at byte `at` of `s` (empty when none does).
fn name_at(s: &str, at: usize) -> &str {
    let rest = &s[at..];
    &rest[..rest.find(|c: char| !is_name_char(c)).unwrap_or(rest.len())]
}

/// Removes the XMP properties whose qualified name (`dc:title`, `exif:GPSLatitude`, ...) `remove` accepts, written
/// as elements (with everything inside them) or as attributes of an `rdf:Description`. Every other property, the
/// namespace declarations and the packet wrapper stay as they were. `None` when the packet is not UTF-8, a matching
/// element is not closed, or a matching name is left that the scan could not take out.
pub fn strip_xmp_properties(packet: &[u8], remove: &dyn Fn(&str) -> bool) -> Option<Vec<u8>> {
    let mut s = std::str::from_utf8(packet).ok()?.to_string();
    let mut from = 0;
    while let Some(p) = s[from..].find('<') {
        let start = from + p;
        let name = name_at(&s, start + 1).to_string();
        if name.is_empty() || !remove(&name) {
            from = start + 1;
            continue;
        }
        let gt = start + s[start..].find('>')?;
        let end = if s[..gt].ends_with('/') {
            gt + 1
        } else {
            let close = format!("</{name}>");
            gt + s[gt..].find(&close)? + close.len()
        };
        s.replace_range(start..end, "");
        from = start;
    }
    let mut from = 0;
    while let Some(p) = s[from..].find(char::is_whitespace) {
        let ws = from + p;
        let at = ws + s[ws..].chars().next().map_or(1, char::len_utf8);
        let name = name_at(&s, at).to_string();
        let after = at + name.len();
        let quote = s[after..].strip_prefix('=').and_then(|r| r.chars().next()).filter(|q| *q == '"' || *q == '\'');
        match quote {
            Some(q) if !name.is_empty() && remove(&name) => {
                let value_start = after + 2;
                let value_end = value_start + s[value_start..].find(q)?;
                s.replace_range(ws..value_end + 1, "");
                from = ws;
            }
            _ => from = at,
        }
    }
    let left = s.split(|c: char| !is_name_char(c)).any(|token| remove(token));
    (!left).then(|| s.into_bytes())
}

/// The `<rdf:Description ...>` blocks of `packet`, whole, in order.
fn description_blocks(packet: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = packet[from..].find("<rdf:Description") {
        let start = from + p;
        let Some(gt) = packet[start..].find('>').map(|g| start + g) else { break };
        let end = if packet[..gt].ends_with('/') {
            gt + 1
        } else {
            match packet[gt..].find("</rdf:Description>") {
                Some(e) => gt + e + "</rdf:Description>".len(),
                None => break,
            }
        };
        out.push(&packet[start..end]);
        from = end;
    }
    out
}

/// Merges the properties of the packet `new` into the packet `old`: the properties named in `names` are taken out
/// of `old` and the `rdf:Description` blocks of `new` are added beside the old ones (the same subject may be
/// described by several blocks), so every other property of `old` (camera, editing history, location,
/// orientation) is kept, as the reference merge keeps them. An empty `new` only removes `names`. `None` when the
/// packets cannot be edited this way (not UTF-8, no `</rdf:RDF>`, a property the scan cannot take out); the caller
/// then writes `new` in place of `old`.
pub fn merge_xmp(old: &[u8], names: &[&str], new: &[u8]) -> Option<Vec<u8>> {
    let stripped = String::from_utf8(strip_xmp_properties(old, &|n: &str| names.iter().any(|m| *m == n))?).ok()?;
    let new = std::str::from_utf8(new).ok()?;
    let close = stripped.rfind("</rdf:RDF>")?;
    let blocks = description_blocks(new);
    if !new.trim().is_empty() && blocks.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(stripped.len() + new.len());
    out.push_str(&stripped[..close]);
    for b in blocks {
        out.push_str(b);
        out.push('\n');
    }
    out.push_str(&stripped[close..]);
    Some(out.into_bytes())
}

/// Sets the value of every `tiff:Orientation` attribute or element of an XMP packet to `v` (1..=8).
fn set_xmp_orientation(packet: &mut Vec<u8>, v: u8) {
    let Ok(text) = std::str::from_utf8(packet) else { return };
    let mut s = text.to_string();
    let digit = char::from(b'0' + v.min(9)).to_string();
    for (open, close) in [("tiff:Orientation=\"", "\""), ("tiff:Orientation='", "'"), ("<tiff:Orientation>", "</tiff:Orientation>")] {
        let mut from = 0;
        while let Some(p) = s[from..].find(open) {
            let value_start = from + p + open.len();
            let Some(e) = s[value_start..].find(close) else { break };
            s.replace_range(value_start..value_start + e, &digit);
            from = value_start + digit.len();
        }
    }
    *packet = s.into_bytes();
}

/// The TIFF structure of an EXIF block, read for an in-place patch.
struct TiffBytes<'a> {
    d: &'a [u8],
    le: bool,
}

impl TiffBytes<'_> {
    fn u16_at(&self, p: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(p..p.checked_add(2)?)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32_at(&self, p: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(p..p.checked_add(4)?)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// The byte offset of the 12-byte entry for `tag` in the directory at `ifd`.
    fn entry(&self, ifd: usize, tag: u16) -> Option<usize> {
        let n = self.u16_at(ifd)? as usize;
        (0..n).map(|i| ifd + 2 + 12 * i).find(|&e| self.u16_at(e) == Some(tag))
    }
}

/// The EXIF block with IFD0's orientation and the three capture dates (TIFF DateTime, EXIF DateTimeOriginal and
/// DateTimeDigitized) set in place: every other byte is the old block's, so maker notes and offset tags stay valid.
/// With neither given it is the old block unchanged. `None` when a value to set has no entry of the expected type
/// and size (a single SHORT orientation, a 20-byte ASCII date), so the caller rebuilds the block instead.
fn patch_in_place(block: &[u8], orientation: Option<u8>, date: Option<&str>) -> Option<Vec<u8>> {
    let le = match block.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let t = TiffBytes { d: block, le };
    let ifd0 = t.u32_at(4)? as usize;
    let mut out = block.to_vec();
    if let Some(v) = orientation {
        let e = t.entry(ifd0, 0x0112)?;
        if t.u16_at(e + 2)? != 3 || t.u32_at(e + 4)? != 1 {
            return None;
        }
        let value = if le { u16::from(v).to_le_bytes() } else { u16::from(v).to_be_bytes() };
        out.get_mut(e + 8..e + 10)?.copy_from_slice(&value);
    }
    if let Some(d) = date {
        if d.len() != 19 {
            return None;
        }
        let exif_ifd = t.u32_at(t.entry(ifd0, 0x8769)? + 8)? as usize;
        for (ifd, tag) in [(ifd0, 0x0132), (exif_ifd, 0x9003), (exif_ifd, 0x9004)] {
            let e = t.entry(ifd, tag)?;
            if t.u16_at(e + 2)? != 2 || t.u32_at(e + 4)? != 20 {
                return None;
            }
            let at = t.u32_at(e + 8)? as usize;
            let slot = out.get_mut(at..at.checked_add(20)?)?;
            slot[..19].copy_from_slice(d.as_bytes());
            slot[19] = 0;
        }
    }
    Some(out)
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
            put(ty, *d);
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

    /// A little-endian EXIF block: IFD0 with an orientation of 1 and the EXIF pointer, an EXIF directory with an
    /// 8-byte maker note at offset 56.
    fn block_with_maker_note() -> Vec<u8> {
        let mut b = b"II*\0\x08\0\0\0".to_vec();
        b.extend([2, 0]);
        b.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        b.extend([0x69, 0x87, 4, 0, 1, 0, 0, 0, 38, 0, 0, 0]);
        b.extend([0, 0, 0, 0]);
        b.extend([1, 0]);
        b.extend([0x7C, 0x92, 7, 0, 8, 0, 0, 0, 56, 0, 0, 0]);
        b.extend([0, 0, 0, 0]);
        b.extend(b"ACME\x01\x02\x03\x04");
        assert_eq!(b.len(), 64);
        b
    }

    #[test]
    fn an_orientation_edit_keeps_the_maker_note_and_the_rest_of_the_block() {
        let block = block_with_maker_note();
        let plain = {
            let mut d = ImageDestination::new(Format::Jpeg, 1).unwrap();
            d.add_image(&picture(), None);
            d.finalize().unwrap()
        };
        let mut original = plain[..2].to_vec();
        push_segment(&mut original, 0xE1, &[b"Exif\0\0".as_slice(), block.as_slice()]).unwrap();
        original.extend(&plain[2..]);
        let out = copy_image_source(&original, &CopyOptions { orientation: Some(6), ..Default::default() }).unwrap();
        let s = ImageSource::new(out);
        let mut want = block.clone();
        want[18] = 6;
        assert_eq!(s.exif_block(), Some(want.as_slice()), "only the orientation value changed");
        assert_eq!(s.properties_at(0).unwrap().orientation, 6);
        // a new tag rebuilds the block, and the maker note, whose offsets would move, goes
        let mut tiff = Dict::new();
        tiff.insert("Make".into(), PropValue::Text("Acme".into()));
        let set = MetadataSet { tiff, ..Default::default() };
        let rebuilt = copy_image_source(&original, &CopyOptions { metadata: Some(set), merge: true, ..Default::default() }).unwrap();
        let rebuilt = ImageSource::new(rebuilt);
        assert!(!rebuilt.exif_block().unwrap().windows(4).any(|w| w == b"ACME"));
        assert_eq!(rebuilt.properties_at(0).unwrap().orientation, 1);
    }

    #[test]
    fn merging_a_packet_replaces_the_named_properties_and_keeps_the_rest() {
        let old = "<x:xmpmeta><rdf:RDF><rdf:Description rdf:about=\"\" xmlns:dc=\"d\" xmlns:tiff=\"t\" tiff:Orientation=\"3\" dc:format=\"image/jpeg\"><dc:title><rdf:Alt><rdf:li>Old</rdf:li></rdf:Alt></dc:title><dc:rights>Me</dc:rights></rdf:Description></rdf:RDF></x:xmpmeta>";
        let new = "<rdf:RDF><rdf:Description rdf:about=\"\" xmlns:dc=\"d\"><dc:title>New</dc:title></rdf:Description></rdf:RDF>";
        let merged = String::from_utf8(merge_xmp(old.as_bytes(), &["dc:title", "dc:rights"], new.as_bytes()).unwrap()).unwrap();
        assert!(!merged.contains("Old") && !merged.contains("dc:rights") && merged.contains("<dc:title>New</dc:title>"), "{merged}");
        assert!(merged.contains("tiff:Orientation=\"3\"") && merged.contains("dc:format=\"image/jpeg\"") && merged.ends_with("</rdf:RDF></x:xmpmeta>"), "{merged}");
        let removed = String::from_utf8(merge_xmp(old.as_bytes(), &["dc:title"], b"").unwrap()).unwrap();
        assert!(!removed.contains("dc:title") && removed.contains("<dc:rights>Me</dc:rights>"), "{removed}");
        assert_eq!(merge_xmp(old.as_bytes(), &["dc:title"], b"not a packet"), None, "a new packet without a description is not merged");
        let mut packet = old.as_bytes().to_vec();
        set_xmp_orientation(&mut packet, 6);
        assert!(String::from_utf8(packet).unwrap().contains("tiff:Orientation=\"6\""));
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
