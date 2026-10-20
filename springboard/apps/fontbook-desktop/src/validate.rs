//! Font validation and repertoire. A font is checked before it is installed: it must be a complete TrueType or
//! OpenType file, or a font collection whose first face is complete, with the tables a renderer needs and a character
//! map it can read. The same reading gives the repertoire: how many characters the font maps and which Unicode blocks
//! they fall in.
//!
//! Pure: bytes in, a `Report` out. Nothing here draws or writes; `model.rs` and `main.rs` use the result.

use std::path::Path;

/// The first four bytes of a TrueType outline font (version 1.0), of a font with 'true' tags, and of an
/// OpenType font with CFF outlines ('OTTO'). A font collection starts with 'ttcf'.
const SFNT_TRUETYPE: u32 = 0x0001_0000;
const SFNT_TRUE: u32 = 0x7472_7565;
const SFNT_OTTO: u32 = 0x4F54_544F;
const SFNT_COLLECTION: u32 = 0x7474_6366;

/// The largest code point Unicode defines.
const MAX_CODEPOINT: u32 = 0x10_FFFF;

/// Unicode blocks the repertoire is named with, as (first, last, name). Blocks not listed are not named.
const BLOCKS: &[(u32, u32, &str)] = &[
    (0x0000, 0x007F, "Basic Latin"),
    (0x0080, 0x00FF, "Latin-1 Supplement"),
    (0x0100, 0x017F, "Latin Extended-A"),
    (0x0180, 0x024F, "Latin Extended-B"),
    (0x0370, 0x03FF, "Greek and Coptic"),
    (0x0400, 0x04FF, "Cyrillic"),
    (0x0590, 0x05FF, "Hebrew"),
    (0x0600, 0x06FF, "Arabic"),
    (0x0900, 0x097F, "Devanagari"),
    (0x0E00, 0x0E7F, "Thai"),
    (0x1E00, 0x1EFF, "Latin Extended Additional"),
    (0x2000, 0x206F, "General Punctuation"),
    (0x20A0, 0x20CF, "Currency Symbols"),
    (0x2100, 0x214F, "Letterlike Symbols"),
    (0x2190, 0x21FF, "Arrows"),
    (0x2200, 0x22FF, "Mathematical Operators"),
    (0x2500, 0x257F, "Box Drawing"),
    (0x25A0, 0x25FF, "Geometric Shapes"),
    (0x3040, 0x309F, "Hiragana"),
    (0x30A0, 0x30FF, "Katakana"),
    (0x4E00, 0x9FFF, "CJK Unified Ideographs"),
    (0xAC00, 0xD7AF, "Hangul Syllables"),
    (0xE000, 0xF8FF, "Private Use Area"),
    (0xFF00, 0xFFEF, "Halfwidth and Fullwidth Forms"),
    (0x1F300, 0x1F5FF, "Miscellaneous Symbols and Pictographs"),
];

/// What the checks found. A font is usable when `problems` is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Why the file cannot be installed, one sentence each. Empty when the font passes.
    pub problems: Vec<String>,
    /// Glyphs the font declares (`maxp`).
    pub glyphs: u32,
    /// Characters the character map covers.
    pub characters: u32,
    /// Names of the Unicode blocks with at least one covered character, in block order.
    pub blocks: Vec<String>,
    /// Things that do not stop an install but may surprise: a missing optional table, no Basic Latin coverage.
    pub warnings: Vec<String>,
}

/// The outcome of a validation: pass, pass with warnings, or fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
}

impl Report {
    pub fn is_valid(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn verdict(&self) -> Verdict {
        if !self.problems.is_empty() {
            Verdict::Fail
        } else if !self.warnings.is_empty() {
            Verdict::Warn
        } else {
            Verdict::Pass
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn tag_text(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}

/// Checks a font file's bytes. Every problem found is reported, so a broken file names all its faults.
pub fn check(bytes: &[u8]) -> Report {
    let mut r = Report::default();
    let Some(version) = u32_at(bytes, 0) else {
        r.problems.push("The file is too short to be a font.".to_string());
        return r;
    };
    // A collection holds several faces. The first face's table directory is checked; table offsets are absolute
    // in the file, so one reading serves a single face (directory at zero) and a collection (directory after the
    // collection header).
    let dir = if version == SFNT_COLLECTION {
        if u32_at(bytes, 8).unwrap_or(0) == 0 {
            r.problems.push("The font collection holds no fonts.".to_string());
            return r;
        }
        let Some(first) = u32_at(bytes, 12) else {
            r.problems.push("The font collection header is cut short.".to_string());
            return r;
        };
        r.warnings.push("This is a font collection: the first face is checked, the other faces are not.".to_string());
        first as usize
    } else {
        0
    };
    let Some(face_version) = u32_at(bytes, dir) else {
        r.problems.push("The font face is cut short.".to_string());
        return r;
    };
    if !matches!(face_version, SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO) {
        r.problems.push("The file has no TrueType or OpenType header.".to_string());
        return r;
    }
    let count = u16_at(bytes, dir + 4).unwrap_or(0) as usize;
    let mut tables: Vec<([u8; 4], usize, usize)> = Vec::new();
    for i in 0..count {
        let at = dir + 12 + 16 * i;
        let tag = bytes.get(at..at + 4).and_then(|t| <[u8; 4]>::try_from(t).ok());
        let (Some(tag), Some(off), Some(len)) = (tag, u32_at(bytes, at + 8), u32_at(bytes, at + 12)) else {
            r.problems.push("The table directory is cut short.".to_string());
            return r;
        };
        let (off, len) = (off as usize, len as usize);
        if off.checked_add(len).is_none_or(|end| end > bytes.len()) {
            r.problems.push(format!("The {} table runs past the end of the file.", tag_text(&tag)));
            continue;
        }
        tables.push((tag, off, len));
    }
    let find = |want: &[u8; 4]| tables.iter().find(|t| &t.0 == want).map(|t| (t.1, t.2));
    for want in [b"cmap", b"head", b"maxp"] {
        if find(want).is_none() {
            r.problems.push(format!("The {} table is missing.", tag_text(want)));
        }
    }
    if let Some((off, _)) = find(b"maxp") {
        r.glyphs = u16_at(bytes, off + 4).unwrap_or(0) as u32;
        if r.glyphs == 0 {
            r.problems.push("The font has no glyphs.".to_string());
        }
    }
    if let Some((off, len)) = find(b"cmap") {
        match repertoire(&bytes[off..off + len]) {
            Some(ranges) => {
                r.characters = ranges.iter().map(|(a, b)| b - a + 1).sum();
                r.blocks = blocks_of(&ranges);
            }
            None => r.problems.push("The font has no character map the desktop can read.".to_string()),
        }
    }
    let optional: [(&[u8; 4], &str); 3] = [
        (b"name", "The name table is missing, so the family name cannot be read."),
        (b"OS/2", "The OS/2 table is missing, so the weight and width are unknown."),
        (b"post", "The post table is missing, so glyph names are unavailable."),
    ];
    for (tag, why) in optional {
        if find(tag).is_none() {
            r.warnings.push(why.to_string());
        }
    }
    if [b"glyf", b"CFF ", b"CFF2"].iter().all(|tag| find(*tag).is_none()) {
        r.warnings.push("The font has no outline table (glyf or CFF), so it may draw nothing.".to_string());
    }
    if r.characters > 0 && !r.blocks.iter().any(|b| b == "Basic Latin") {
        r.warnings.push("The font does not cover Basic Latin, so ordinary text falls back to another font.".to_string());
    }
    r
}

/// Checks the font file at `path`. A file that cannot be read is reported as a problem.
pub fn check_file(path: &Path) -> Report {
    match std::fs::read(path) {
        Ok(bytes) => check(&bytes),
        Err(e) => Report { problems: vec![format!("Cannot read the font: {e}")], ..Report::default() },
    }
}

/// The covered code points of the best character map in `cmap`, as sorted inclusive ranges. Unicode maps are
/// preferred (platform 3, encoding 10, then 3/1), then any platform 0 map. None when no map can be read or it is empty.
fn repertoire(cmap: &[u8]) -> Option<Vec<(u32, u32)>> {
    let count = u16_at(cmap, 2)? as usize;
    let mut subs: Vec<(u8, usize)> = Vec::new();
    for i in 0..count {
        let at = 4 + 8 * i;
        let (platform, encoding, off) = (u16_at(cmap, at)?, u16_at(cmap, at + 2)?, u32_at(cmap, at + 4)? as usize);
        let rank = match (platform, encoding) {
            (3, 10) => 3,
            (3, 1) => 2,
            (0, _) => 1,
            _ => continue,
        };
        subs.push((rank, off));
    }
    subs.sort_by(|a, b| b.0.cmp(&a.0));
    subs.iter().find_map(|&(_, off)| {
        let sub = cmap.get(off..)?;
        let ranges = match u16_at(sub, 0)? {
            4 => format4(sub)?,
            12 => format12(sub)?,
            _ => return None,
        };
        let ranges = normalize(ranges);
        (!ranges.is_empty()).then_some(ranges)
    })
}

/// Format 4: segments of 16-bit code points, each mapped by a delta, or through the glyph index array.
fn format4(sub: &[u8]) -> Option<Vec<(u32, u32)>> {
    let seg = (u16_at(sub, 6)? as usize) / 2;
    let end_at = 14;
    let start_at = end_at + 2 + 2 * seg; // the reserved pad word sits between the end and start arrays
    let delta_at = start_at + 2 * seg;
    let range_at = delta_at + 2 * seg;
    // The work is bounded by the table, not by what a segment claims: a segment that maps through the delta alone
    // is one range (less the one code point that lands on glyph 0), and a segment that reads the glyph index array
    // stops where the array ends. A shared iteration budget stops overlapping segments from multiplying that.
    let mut budget: u32 = MAX_CODEPOINT + 1;
    let mut out: Vec<(u32, u32)> = Vec::new();
    for i in 0..seg {
        let end = u16_at(sub, end_at + 2 * i)? as u32;
        let start = u16_at(sub, start_at + 2 * i)? as u32;
        let delta = u16_at(sub, delta_at + 2 * i)?;
        let ro_at = range_at + 2 * i;
        let ro = u16_at(sub, ro_at)? as usize;
        if start > end {
            continue;
        }
        if ro == 0 {
            // glyph = (c + delta) mod 65536; zero exactly at c = 65536 - delta (mod 65536)
            let hole = 0x1_0000u32.wrapping_sub(delta as u32) & 0xFFFF;
            if hole < start || hole > end {
                out.push((start, end));
            } else {
                if hole > start {
                    out.push((start, hole - 1));
                }
                if hole < end {
                    out.push((hole + 1, end));
                }
            }
            continue;
        }
        let mut run: Option<(u32, u32)> = None;
        for c in start..=end {
            let Some(g) = u16_at(sub, ro_at + ro + 2 * ((c - start) as usize)) else {
                break; // past the end of the glyph index array, every later code point is unmapped too
            };
            if budget == 0 {
                break;
            }
            budget -= 1;
            let glyph = if g == 0 { 0 } else { g.wrapping_add(delta) };
            match (glyph != 0, run) {
                (true, Some((a, _))) => run = Some((a, c)),
                (true, None) => run = Some((c, c)),
                (false, Some(r)) => {
                    out.push(r);
                    run = None;
                }
                (false, None) => {}
            }
        }
        if let Some(r) = run {
            out.push(r);
        }
    }
    Some(out)
}

/// Format 12: groups of consecutive code points mapped to consecutive glyphs. A group whose first glyph is 0 is empty.
fn format12(sub: &[u8]) -> Option<Vec<(u32, u32)>> {
    let groups = u32_at(sub, 12)? as usize;
    let mut out = Vec::new();
    for g in 0..groups {
        let at = 16 + 12 * g;
        let (start, end, first) = (u32_at(sub, at)?, u32_at(sub, at + 4)?, u32_at(sub, at + 8)?);
        if start <= end && start <= MAX_CODEPOINT && first != 0 {
            out.push((start, end.min(MAX_CODEPOINT)));
        }
    }
    Some(out)
}

/// Sorts ranges and merges the ones that touch or overlap.
fn normalize(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The names of the blocks that any of `ranges` reaches, in block order.
fn blocks_of(ranges: &[(u32, u32)]) -> Vec<String> {
    BLOCKS
        .iter()
        .filter(|(first, last, _)| ranges.iter().any(|&(a, b)| a <= *last && b >= *first))
        .map(|(_, _, name)| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-face font file with the given tables (tag, body), laid out as a table directory then the bodies.
    fn sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        face_after(Vec::new(), tables)
    }

    /// A font collection with one face: the collection header, then the face's table directory and bodies.
    fn collection(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = b"ttcf".to_vec();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&16u32.to_be_bytes());
        face_after(out, tables)
    }

    /// Appends a face (directory, then bodies) to `out`. Table offsets are absolute in the file.
    fn face_after(mut out: Vec<u8>, tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let start = out.len();
        out.extend_from_slice(&SFNT_TRUETYPE.to_be_bytes());
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0u8; 6]);
        let mut offset = start + 12 + 16 * tables.len();
        let mut data = Vec::new();
        for (tag, body) in tables {
            out.extend_from_slice(&tag[..]);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            offset += body.len();
            data.extend_from_slice(body);
        }
        out.extend_from_slice(&data);
        out
    }

    fn maxp(glyphs: u16) -> Vec<u8> {
        let mut t = 0x0000_5000u32.to_be_bytes().to_vec();
        t.extend_from_slice(&glyphs.to_be_bytes());
        t
    }

    /// A cmap with one subtable at offset 12, for the given platform and encoding.
    fn cmap(platform: u16, encoding: u16, sub: Vec<u8>) -> Vec<u8> {
        let mut out = vec![0, 0, 0, 1];
        out.extend_from_slice(&platform.to_be_bytes());
        out.extend_from_slice(&encoding.to_be_bytes());
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(&sub);
        out
    }

    /// Format 4 with one segment mapping 0x41..=0x43 to glyphs 1..=3, and the usual 0xFFFF sentinel.
    fn format4_basic() -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&4u16.to_be_bytes()); // format
        t.extend_from_slice(&0u16.to_be_bytes()); // length (not read)
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&4u16.to_be_bytes()); // segCountX2: two segments
        t.extend_from_slice(&[0u8; 6]); // searchRange, entrySelector, rangeShift
        for v in [0x0043u16, 0xFFFF] {
            t.extend_from_slice(&v.to_be_bytes()); // endCode
        }
        t.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
        for v in [0x0041u16, 0xFFFF] {
            t.extend_from_slice(&v.to_be_bytes()); // startCode
        }
        for v in [0xFFC0u16, 1] {
            t.extend_from_slice(&v.to_be_bytes()); // idDelta: 0x41 + 0xFFC0 = glyph 1
        }
        t.extend_from_slice(&[0u8; 4]); // idRangeOffset
        t
    }

    /// Format 12 with one group 0x4E00..=0x4E02 mapped to glyphs 5..=7.
    fn format12_cjk() -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&12u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&28u32.to_be_bytes());
        t.extend_from_slice(&0u32.to_be_bytes());
        t.extend_from_slice(&1u32.to_be_bytes()); // one group
        t.extend_from_slice(&0x4E00u32.to_be_bytes());
        t.extend_from_slice(&0x4E02u32.to_be_bytes());
        t.extend_from_slice(&5u32.to_be_bytes());
        t
    }

    fn good_font(cmap_table: Vec<u8>) -> Vec<u8> {
        sfnt(&[(b"cmap", cmap_table), (b"head", vec![0; 54]), (b"maxp", maxp(8))])
    }

    #[test]
    fn a_format_4_font_passes_and_reports_its_repertoire() {
        let r = check(&good_font(cmap(3, 1, format4_basic())));
        assert!(r.is_valid(), "{:?}", r.problems);
        assert_eq!(r.glyphs, 8);
        assert_eq!(r.characters, 3);
        assert_eq!(r.blocks, ["Basic Latin"]);
    }

    #[test]
    fn a_hostile_format_4_font_is_read_in_bounded_time_and_memory() {
        // 4000 segments, each 0..=0xFFFF with a zero range offset: one range each, not 65536 entries each.
        let seg = 4000u16;
        let mut t = Vec::new();
        t.extend_from_slice(&4u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&(seg * 2).to_be_bytes());
        t.extend_from_slice(&[0u8; 6]);
        for _ in 0..seg {
            t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        }
        t.extend_from_slice(&0u16.to_be_bytes());
        for _ in 0..seg {
            t.extend_from_slice(&0u16.to_be_bytes());
        }
        for _ in 0..seg {
            t.extend_from_slice(&1u16.to_be_bytes());
        }
        t.extend_from_slice(&vec![0u8; 2 * seg as usize]);
        let ranges = repertoire(&cmap(3, 1, t)).expect("readable");
        // glyph 0 falls on code point 0xFFFF only
        assert_eq!(ranges, vec![(0, 0xFFFE)]);
    }

    #[test]
    fn a_format_4_segment_through_the_glyph_array_stops_where_the_array_ends() {
        let mut t = Vec::new();
        t.extend_from_slice(&4u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&2u16.to_be_bytes()); // one segment
        t.extend_from_slice(&[0u8; 6]);
        t.extend_from_slice(&0xFFFFu16.to_be_bytes()); // end
        t.extend_from_slice(&0u16.to_be_bytes()); // pad
        t.extend_from_slice(&0u16.to_be_bytes()); // start
        t.extend_from_slice(&0u16.to_be_bytes()); // delta
        t.extend_from_slice(&2u16.to_be_bytes()); // idRangeOffset: the array follows this word
        t.extend_from_slice(&5u16.to_be_bytes()); // code point 0 -> glyph 5
        t.extend_from_slice(&6u16.to_be_bytes()); // code point 1 -> glyph 6
        t.extend_from_slice(&0u16.to_be_bytes()); // code point 2 -> none
        let ranges = repertoire(&cmap(3, 1, t)).expect("readable");
        assert_eq!(ranges, vec![(0, 1)]);
    }

    #[test]
    fn a_format_12_font_names_the_block_it_covers() {
        let r = check(&good_font(cmap(3, 10, format12_cjk())));
        assert!(r.is_valid(), "{:?}", r.problems);
        assert_eq!(r.characters, 3);
        assert_eq!(r.blocks, ["CJK Unified Ideographs"]);
    }

    #[test]
    fn missing_optional_tables_and_coverage_gaps_warn_without_failing() {
        let r = check(&good_font(cmap(3, 1, format4_basic())));
        assert_eq!(r.verdict(), Verdict::Warn);
        assert!(r.warnings.iter().any(|w| w.starts_with("The name table is missing")), "{:?}", r.warnings);
        assert!(r.warnings.iter().any(|w| w.starts_with("The OS/2 table is missing")));
        assert!(r.warnings.iter().any(|w| w.starts_with("The post table is missing")));
        assert!(r.warnings.iter().any(|w| w.starts_with("The font has no outline table")));
        assert!(!r.warnings.iter().any(|w| w.contains("Basic Latin")), "this font covers Basic Latin");
        let cjk = check(&good_font(cmap(3, 10, format12_cjk())));
        assert!(cjk.warnings.iter().any(|w| w.contains("does not cover Basic Latin")), "{:?}", cjk.warnings);
        assert!(cjk.is_valid());
    }

    #[test]
    fn a_font_with_every_table_passes_clean_and_a_broken_one_fails() {
        let full = sfnt(&[
            (b"cmap", cmap(3, 1, format4_basic())),
            (b"head", vec![0; 54]),
            (b"maxp", maxp(8)),
            (b"name", vec![0; 6]),
            (b"OS/2", vec![0; 8]),
            (b"post", vec![0; 8]),
            (b"glyf", vec![0; 4]),
        ]);
        let r = check(&full);
        assert_eq!(r.verdict(), Verdict::Pass, "{:?} {:?}", r.problems, r.warnings);
        assert_eq!(check(b"hello world").verdict(), Verdict::Fail);
    }

    #[test]
    fn a_missing_table_is_named() {
        let bytes = sfnt(&[(b"cmap", cmap(3, 1, format4_basic())), (b"maxp", maxp(8))]);
        let r = check(&bytes);
        assert_eq!(r.problems, ["The head table is missing."]);
    }

    #[test]
    fn a_table_past_the_end_is_reported_and_the_rest_still_checked() {
        let mut bytes = good_font(cmap(3, 1, format4_basic()));
        bytes.truncate(bytes.len() - 10); // the maxp body is cut
        let r = check(&bytes);
        assert!(r.problems.iter().any(|p| p == "The maxp table runs past the end of the file."), "{:?}", r.problems);
        assert!(r.problems.iter().any(|p| p == "The maxp table is missing."), "{:?}", r.problems);
    }

    #[test]
    fn files_that_are_not_sfnt_or_are_empty_collections_are_refused() {
        assert_eq!(check(b"hello world").problems, ["The file has no TrueType or OpenType header."]);
        assert_eq!(check(&[1, 2]).problems, ["The file is too short to be a font."]);
        assert_eq!(check(b"ttcf\0\0\0\x01").problems, ["The font collection holds no fonts."]);
        assert_eq!(check(b"ttcf\0\x01\0\0\0\0\0\x01").problems, ["The font collection header is cut short."]);
    }

    #[test]
    fn a_collection_passes_when_its_first_face_is_complete() {
        let bytes = collection(&[(b"cmap", cmap(3, 1, format4_basic())), (b"head", vec![0; 54]), (b"maxp", maxp(8))]);
        let r = check(&bytes);
        assert!(r.is_valid(), "{:?}", r.problems);
        assert_eq!(r.glyphs, 8);
        assert_eq!(r.characters, 3);
        assert_eq!(r.blocks, ["Basic Latin"]);
        assert!(r.warnings.iter().any(|w| w.starts_with("This is a font collection: the first face is checked")), "{:?}", r.warnings);
    }

    #[test]
    fn a_collection_with_a_broken_first_face_fails_with_the_face_problems() {
        let bytes = collection(&[(b"cmap", cmap(3, 1, format4_basic())), (b"head", vec![0; 54])]);
        assert_eq!(check(&bytes).problems, ["The maxp table is missing."]);
        let mut cut = collection(&[(b"cmap", cmap(3, 1, format4_basic())), (b"head", vec![0; 54]), (b"maxp", maxp(8))]);
        cut.truncate(18);
        assert_eq!(check(&cut).problems, ["The font face is cut short."]);
    }

    #[test]
    fn a_font_with_no_glyphs_or_no_readable_map_fails() {
        let no_glyphs = sfnt(&[(b"cmap", cmap(3, 1, format4_basic())), (b"head", vec![0; 54]), (b"maxp", maxp(0))]);
        assert_eq!(check(&no_glyphs).problems, ["The font has no glyphs."]);
        let unknown = good_font(cmap(3, 1, vec![0, 99, 0, 0]));
        assert_eq!(check(&unknown).problems, ["The font has no character map the desktop can read."]);
    }

    #[test]
    fn overlapping_and_touching_ranges_merge() {
        assert_eq!(normalize(vec![(5, 6), (1, 2), (3, 4), (6, 9)]), [(1, 9)]);
        assert_eq!(normalize(vec![(1, 2), (4, 4)]), [(1, 2), (4, 4)]);
    }

    #[test]
    fn blocks_are_named_in_block_order() {
        assert_eq!(blocks_of(&[(0x41, 0x41), (0x4E00, 0x4E00), (0x20AC, 0x20AC)]), ["Basic Latin", "Currency Symbols", "CJK Unified Ideographs"]);
    }

    #[test]
    fn check_file_reports_an_unreadable_path() {
        let r = check_file(Path::new("/nonexistent/font.ttf"));
        assert!(!r.is_valid());
        assert!(r.problems[0].starts_with("Cannot read the font:"));
    }
}
