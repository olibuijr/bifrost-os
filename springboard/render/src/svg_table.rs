//! OpenType `SVG ` table index: which SVG document draws which glyph ids.
//!
//! The table holds a list of records. Each record covers a glyph id range (`start..=end`) and points at one SVG
//! document, which may be gzip-compressed. This module reads that index, gunzips the document
//! ([`SvgTable::document_text_for`]), cuts out one glyph's markup ([`glyph_markup`]) and draws it with the SVG reader
//! ([`SvgTable::rasterize_glyph`]). The text engine does not call `rasterize_glyph` yet.

/// One record of the SVG document list: a glyph id range and the byte range of its document in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvgRecord {
    pub start: u16,
    pub end: u16,
    /// Offset of the document from the start of the table.
    pub offset: usize,
    pub len: usize,
}

/// A parsed `SVG ` table. Records whose document lies outside the table are dropped.
#[derive(Clone, Debug)]
pub struct SvgTable<'a> {
    data: &'a [u8],
    records: Vec<SvgRecord>,
}

fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]))
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(o)?, *d.get(o + 1)?, *d.get(o + 2)?, *d.get(o + 3)?]))
}

impl<'a> SvgTable<'a> {
    /// Parses the table header and the document list. Returns `None` when the header or the list is truncated.
    pub fn parse(table: &'a [u8]) -> Option<Self> {
        let list = be32(table, 2)? as usize;
        let count = be16(table, list)? as usize;
        let mut records = Vec::with_capacity(count);
        for i in 0..count {
            let r = list + 2 + i * 12;
            let start = be16(table, r)?;
            let end = be16(table, r + 2)?;
            // Document offsets are relative to the start of the document list.
            let offset = list + be32(table, r + 4)? as usize;
            let len = be32(table, r + 8)? as usize;
            if start > end || offset.checked_add(len).map_or(true, |e| e > table.len()) {
                continue;
            }
            records.push(SvgRecord { start, end, offset, len });
        }
        Some(SvgTable { data: table, records })
    }

    /// The records in table order.
    pub fn records(&self) -> &[SvgRecord] {
        &self.records
    }

    /// The raw bytes of the document that covers `gid`: the first record in table order whose range holds it.
    pub fn document_for(&self, gid: u16) -> Option<&'a [u8]> {
        let r = self.records.iter().find(|r| r.start <= gid && gid <= r.end)?;
        self.data.get(r.offset..r.offset + r.len)
    }

    /// Whether a document is gzip-compressed (starts with the gzip magic bytes).
    pub fn is_gzip(doc: &[u8]) -> bool {
        doc.starts_with(&[0x1f, 0x8b])
    }

    /// The SVG text of the document that covers `gid`, gunzipped when the document is compressed. `None` when no
    /// record covers the glyph, the document is damaged, or its bytes are not UTF-8.
    pub fn document_text_for(&self, gid: u16) -> Option<String> {
        let doc = self.document_for(gid)?;
        let bytes = if Self::is_gzip(doc) { gunzip(doc)? } else { doc.to_vec() };
        String::from_utf8(bytes).ok()
    }
}

/// The metrics of the glyph to draw, in font units (`unitsPerEm` per em). `ascent` is the height above the baseline and
/// `descent` the depth below it, both positive; `advance` is the glyph's advance width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub units_per_em: u16,
    pub ascent: u16,
    pub descent: u16,
    pub advance: u16,
}

/// A colour glyph drawn to ARGB pixels (`0xAARRGGBB`, not premultiplied).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGlyph {
    pub image: crate::svg::SvgImage,
    /// Pixels from the top row of the image down to the baseline: place the image this far above the baseline.
    pub baseline: u32,
}

/// The markup that draws glyph `gid` out of a document. A document that holds several glyphs marks each one with an
/// element `id="glyph<gid>"`, and that element is returned whole. A document without such an element draws one glyph,
/// and everything between its root `<svg>` tags is returned. `None` when the document is not an SVG document.
pub fn glyph_markup(doc: &str, gid: u16) -> Option<&str> {
    for quote in ['"', '\''] {
        let needle = format!("id={quote}glyph{gid}{quote}");
        if let Some(at) = doc.find(&needle) {
            let start = doc[..at].rfind('<')?;
            return element_at(doc, start);
        }
    }
    let open = doc.find("<svg")?;
    let inner_start = open + doc[open..].find('>')? + 1;
    if doc[open..inner_start].ends_with("/>") {
        return Some("");
    }
    let end = doc.rfind("</svg>")?;
    (end >= inner_start).then(|| &doc[inner_start..end])
}

/// The whole element that starts at `start` (a `<`): to its own `/>`, or to the close tag that balances it.
fn element_at(doc: &str, start: usize) -> Option<&str> {
    let tag_end = start + doc[start..].find('>')?;
    if doc[start..tag_end].ends_with('/') {
        return Some(&doc[start..=tag_end]);
    }
    let name = doc[start + 1..tag_end].split(|c: char| c.is_whitespace() || c == '/').next()?;
    let (mut depth, mut at) = (1usize, tag_end + 1);
    while let Some(lt) = doc[at..].find('<') {
        let tag_start = at + lt;
        let end = tag_start + doc[tag_start..].find('>')?;
        let body = &doc[tag_start + 1..end];
        if let Some(close) = body.strip_prefix('/') {
            if close.trim() == name {
                depth -= 1;
                if depth == 0 {
                    return Some(&doc[start..=end]);
                }
            }
        } else if !body.ends_with('/') && body.split(|c: char| c.is_whitespace()).next() == Some(name) {
            depth += 1;
        }
        at = end + 1;
    }
    None
}

impl SvgTable<'_> {
    /// Draws glyph `gid` at `px_per_em` pixels per em: finds its document, gunzips it, cuts out the glyph's markup and
    /// rasterises it with the SVG reader. The glyph's origin is at the baseline with the y axis pointing down, as in
    /// OpenType-SVG, so the image covers `-ascent..descent` vertically and `0..advance` horizontally. Features the SVG
    /// reader does not draw (gradients, transforms, strokes) leave their pixels clear. `None` when no document covers the
    /// glyph, the metrics are empty, or the size is zero or too large.
    pub fn rasterize_glyph(&self, gid: u16, m: GlyphMetrics, px_per_em: f32) -> Option<ColorGlyph> {
        let height_units = u32::from(m.ascent) + u32::from(m.descent);
        if m.units_per_em == 0 || m.advance == 0 || height_units == 0 || !(px_per_em > 0.0) {
            return None;
        }
        let scale = px_per_em / f32::from(m.units_per_em);
        // a hair is taken off before rounding up, so 7.0000005 pixels of float noise does not cost a whole row
        let cells = |units: u16| (f32::from(units) * scale - 0.001).ceil().max(0.0);
        let width = cells(m.advance).max(1.0);
        let up = cells(m.ascent);
        let height = (up + cells(m.descent)).max(1.0);
        if width > 8192.0 || height > 8192.0 {
            return None;
        }
        let doc = self.document_text_for(gid)?;
        let markup = glyph_markup(&doc, gid)?;
        // The view box is the pixel box in font units, so one font unit is the same number of pixels both ways.
        let wrapped = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 {} {} {}\">{markup}</svg>",
            -(up / scale),
            width / scale,
            height / scale
        );
        let image = crate::svg::rasterize(&wrapped, width as u32, height as u32)?;
        Some(ColorGlyph { image, baseline: up as u32 })
    }
}

/// Gunzips one gzip member (RFC 1952) with the in-tree DEFLATE decoder. Reads the optional header fields (extra,
/// name, comment, header CRC) and returns the inflated bytes. The trailing CRC and size are not checked. `None` when
/// the header is not gzip deflate, is truncated, or the stream is damaged.
pub fn gunzip(doc: &[u8]) -> Option<Vec<u8>> {
    if !SvgTable::is_gzip(doc) || doc.len() < 10 || doc[2] != 8 {
        return None;
    }
    let flags = doc[3];
    let mut at = 10;
    if flags & 0x04 != 0 {
        let xlen = u16::from_le_bytes([*doc.get(at)?, *doc.get(at + 1)?]) as usize;
        at += 2 + xlen;
    }
    for flag in [0x08u8, 0x10] {
        // FNAME and FCOMMENT are zero-terminated strings
        if flags & flag != 0 {
            while *doc.get(at)? != 0 {
                at += 1;
            }
            at += 1;
        }
    }
    if flags & 0x02 != 0 {
        at += 2;
    }
    crate::cg::pdf::inflate::inflate(doc.get(at..)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table with two records: glyphs 3..=5 and glyph 7.
    fn table() -> Vec<u8> {
        let doc1 = b"<svg>a</svg>";
        let doc2 = b"abc";
        let mut t = vec![0, 0, 0, 0, 0, 6]; // version 0, document list at offset 6
        t.extend_from_slice(&2u16.to_be_bytes()); // two records
        // Document offsets are relative to the list, and the documents follow the 2 + 2 * 12 byte list.
        let first = 2 + 2 * 12;
        for (s, e, off, len) in [(3u16, 5u16, first, doc1.len()), (7, 7, first + doc1.len(), doc2.len())] {
            t.extend_from_slice(&s.to_be_bytes());
            t.extend_from_slice(&e.to_be_bytes());
            t.extend_from_slice(&(off as u32).to_be_bytes());
            t.extend_from_slice(&(len as u32).to_be_bytes());
        }
        t.extend_from_slice(doc1);
        t.extend_from_slice(doc2);
        t
    }

    #[test]
    fn a_glyph_finds_the_document_of_its_range() {
        let t = table();
        let svg = SvgTable::parse(&t).unwrap();
        assert_eq!(svg.records().len(), 2);
        assert_eq!(svg.document_for(3), Some(&b"<svg>a</svg>"[..]));
        assert_eq!(svg.document_for(5), Some(&b"<svg>a</svg>"[..]));
        assert_eq!(svg.document_for(7), Some(&b"abc"[..]));
        assert_eq!(svg.document_for(6), None);
    }

    #[test]
    fn records_outside_the_table_are_dropped() {
        let t = table();
        // Cut the second document short: its record is dropped, the first still resolves.
        let svg = SvgTable::parse(&t[..t.len() - 1]).unwrap();
        assert_eq!(svg.records().len(), 1);
        assert_eq!(svg.document_for(7), None);
        assert!(svg.document_for(4).is_some());
    }

    #[test]
    fn a_truncated_header_is_refused() {
        assert!(SvgTable::parse(&[0, 0, 0]).is_none());
        assert!(SvgTable::parse(&[0, 0, 0, 0, 0, 6]).is_none());
    }

    #[test]
    fn gzip_documents_are_recognised() {
        assert!(SvgTable::is_gzip(&[0x1f, 0x8b, 8, 0]));
        assert!(!SvgTable::is_gzip(b"<svg"));
    }

    /// A gzip member whose deflate data is one stored (uncompressed) block. `flags` may set FEXTRA (0x04),
    /// FNAME (0x08) or FHCRC (0x02); the header fields are written in RFC 1952 order.
    fn gzip_stored(payload: &[u8], flags: u8) -> Vec<u8> {
        let mut g = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 0xff];
        if flags & 0x04 != 0 {
            g.extend_from_slice(&[2, 0, b'x', b'y']); // XLEN is little-endian
        }
        if flags & 0x08 != 0 {
            g.extend_from_slice(b"icon.svg\0");
        }
        if flags & 0x02 != 0 {
            g.extend_from_slice(&[0, 0]);
        }
        g.push(0x01); // BFINAL = 1, BTYPE = 00 (stored)
        g.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        g.extend_from_slice(&(!(payload.len() as u16)).to_le_bytes());
        g.extend_from_slice(payload);
        g.extend_from_slice(&[0; 8]); // CRC-32 and size trailer, not checked
        g
    }

    #[test]
    fn a_gzip_member_gunzips_to_its_svg_text() {
        let svg = b"<svg>gz</svg>";
        assert_eq!(gunzip(&gzip_stored(svg, 0)).as_deref(), Some(&svg[..]));
        assert_eq!(gunzip(&gzip_stored(svg, 0x04 | 0x08 | 0x02)).as_deref(), Some(&svg[..]));
    }

    #[test]
    fn plain_or_truncated_input_does_not_gunzip() {
        assert_eq!(gunzip(b"<svg>"), None);
        assert_eq!(gunzip(&[0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 0xff, b'a']), None);
    }

    /// A table with one record covering `start..=end` whose document is `doc` (stored, uncompressed).
    fn table_of(start: u16, end: u16, doc: &str) -> Vec<u8> {
        let mut t = vec![0, 0, 0, 0, 0, 6];
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&start.to_be_bytes());
        t.extend_from_slice(&end.to_be_bytes());
        t.extend_from_slice(&14u32.to_be_bytes());
        t.extend_from_slice(&(doc.len() as u32).to_be_bytes());
        t.extend_from_slice(doc.as_bytes());
        t
    }

    const METRICS: GlyphMetrics = GlyphMetrics { units_per_em: 1000, ascent: 800, descent: 200, advance: 1000 };

    const TWO_GLYPHS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph5"><rect x="0" y="-800" width="500" height="800" fill="#ff0000"/></g><g id="glyph6"><g><rect x="500" y="-800" width="500" height="800" fill="#0000ff"/></g></g></svg>"##;

    #[test]
    fn a_shared_document_gives_each_glyph_its_own_element() {
        let five = glyph_markup(TWO_GLYPHS, 5).unwrap();
        assert!(five.starts_with("<g id=\"glyph5\">") && five.ends_with("</g>") && five.contains("#ff0000"));
        assert!(!five.contains("#0000ff"));
        let six = glyph_markup(TWO_GLYPHS, 6).unwrap();
        assert!(six.contains("#0000ff") && !six.contains("#ff0000"), "nested groups stay inside the glyph");
        assert!(six.ends_with("</g></g>"));
        let single = r#"<svg viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#;
        assert_eq!(glyph_markup(single, 9), Some(r#"<rect width="1" height="1"/>"#));
        assert_eq!(glyph_markup("not svg", 1), None);
        assert_eq!(glyph_markup(r#"<svg><circle id="glyph7" r="2"/></svg>"#, 7), Some(r#"<circle id="glyph7" r="2"/>"#));
        let near = r#"<svg><g id="glyph50"><rect fill="red"/></g><g id="glyph5"><rect fill="blue"/></g></svg>"#;
        let picked = glyph_markup(near, 5).unwrap();
        assert!(picked.contains("blue") && !picked.contains("red"), "glyph5 is not glyph50");
    }

    #[test]
    fn a_colour_glyph_is_drawn_above_and_below_its_baseline() {
        let t = table_of(5, 6, TWO_GLYPHS);
        let table = SvgTable::parse(&t).unwrap();
        let g = table.rasterize_glyph(5, METRICS, 10.0).unwrap();
        assert_eq!((g.image.width, g.image.height, g.baseline), (10, 10, 8));
        let px = |x: u32, y: u32| g.image.pixels[(y * g.image.width + x) as usize];
        // an edge that lands a hair off a pixel boundary may cost a unit of alpha, so a hit is "opaque enough"
        let hit = |p: u32, rgb: u32| p >> 24 >= 0xfe && p & 0xff_ffff == rgb;
        assert!(hit(px(2, 0), 0xff0000), "left half, top row");
        assert!(hit(px(2, 7), 0xff0000), "left half, just above the baseline");
        assert_eq!(px(2, 9), 0, "below the baseline is clear");
        assert_eq!(px(7, 3), 0, "glyph 5 does not draw glyph 6's half");
        let six = table.rasterize_glyph(6, METRICS, 10.0).unwrap();
        assert!(hit(six.image.pixels[(3 * 10 + 7) as usize], 0x0000ff));
        assert_eq!(six.image.pixels[(3 * 10 + 2) as usize], 0);
    }

    #[test]
    fn a_glyph_without_a_document_or_with_bad_metrics_is_not_drawn() {
        let t = table_of(5, 6, TWO_GLYPHS);
        let table = SvgTable::parse(&t).unwrap();
        assert_eq!(table.rasterize_glyph(9, METRICS, 10.0), None);
        assert_eq!(table.rasterize_glyph(5, GlyphMetrics { units_per_em: 0, ..METRICS }, 10.0), None);
        assert_eq!(table.rasterize_glyph(5, GlyphMetrics { advance: 0, ..METRICS }, 10.0), None);
        assert_eq!(table.rasterize_glyph(5, METRICS, 0.0), None);
        assert_eq!(table.rasterize_glyph(5, METRICS, 1.0e6), None);
    }

    #[test]
    fn the_document_text_of_a_glyph_is_gunzipped_when_compressed() {
        let svg = b"<svg>glyph</svg>";
        let doc = gzip_stored(svg, 0);
        // one record: glyph 3, its document at list offset 14 (the list header is 2 + 12 bytes)
        let mut t = vec![0, 0, 0, 0, 0, 6];
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&3u16.to_be_bytes());
        t.extend_from_slice(&3u16.to_be_bytes());
        t.extend_from_slice(&14u32.to_be_bytes());
        t.extend_from_slice(&(doc.len() as u32).to_be_bytes());
        t.extend_from_slice(&doc);
        let table = SvgTable::parse(&t).unwrap();
        assert_eq!(table.document_text_for(3).as_deref(), Some("<svg>glyph</svg>"));
        assert_eq!(table.document_text_for(4), None);
    }
}
