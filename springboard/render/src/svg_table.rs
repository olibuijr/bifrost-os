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
}
