//! OpenType `SVG ` table index: which SVG document draws which glyph ids.
//!
//! The table holds a list of records. Each record covers a glyph id range (`start..=end`) and points at one SVG
//! document, which may be gzip-compressed. This module only reads that index and hands back the document bytes. It
//! does not decompress or rasterise them, so colour glyphs are not drawn yet.

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
