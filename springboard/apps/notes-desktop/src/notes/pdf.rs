//! Share as PDF: a note becomes a plain PDF document, its title in bold and
//! its body in Helvetica, wrapped to the page width and paginated. Only the
//! standard 14 fonts are referenced (nothing is embedded), so the file is
//! built from bytes alone: Latin-1 text is kept, other characters become `?`.
//! Pipe tables are flattened into one line per row, separator rows dropped.

/// Lines per page at 14 points leading between the top and bottom margins.
const LINES_PER_PAGE: usize = 46;
/// Characters per line before a word wraps.
const WRAP: usize = 88;
const BODY_PT: u32 = 11;
const TITLE_PT: u32 = 13;
const LEADING: u32 = 14;
/// The text block starts this far below the top edge (792 - 72 margin).
const START_Y: u32 = 720;

/// The PDF bytes for a note: `title` as the bold first lines, then `body`
/// (the note's Markdown, already reduced to plain text by the caller or not).
pub fn render(title: &str, body: &str) -> Vec<u8> {
    let mut lines: Vec<(bool, String)> = wrap(title).into_iter().map(|l| (true, l)).collect();
    lines.push((false, String::new()));
    for raw in flatten(body) {
        lines.extend(wrap(&raw).into_iter().map(|l| (false, l)));
    }
    let pages: Vec<&[(bool, String)]> = lines.chunks(LINES_PER_PAGE).collect();
    write_document(&pages)
}

/// Pipe-table rows become cells joined with bars; the `| --- |` rule goes.
fn flatten(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let t = line.trim();
            if !t.starts_with('|') {
                return Some(line.to_string());
            }
            if t.chars().all(|c| "|-: ".contains(c)) {
                return None;
            }
            let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
            Some(cells.join("   |   "))
        })
        .collect()
}

/// Splits a line on whitespace into lines of at most [`WRAP`] characters. A
/// blank line stays one blank line; a word longer than the width is cut.
fn wrap(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split_whitespace() {
        let chars: Vec<char> = word.chars().collect();
        for piece in chars.chunks(WRAP) {
            let piece: String = piece.iter().collect();
            if !cur.is_empty() && cur.chars().count() + 1 + piece.chars().count() > WRAP {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(&piece);
        }
    }
    out.push(cur);
    out
}

/// The WinAnsi byte for a character: printable ASCII and Latin-1 keep their
/// code, everything else is `?`.
fn byte_of(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{a0}'..='\u{ff}' => c as u32 as u8,
        _ => b'?',
    }
}

/// Appends `text` as a PDF string literal shown with `Tj`.
fn push_text(out: &mut Vec<u8>, text: &str) {
    out.push(b'(');
    for c in text.chars() {
        let b = byte_of(c);
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            0x20..=0x7e => out.push(b),
            _ => out.extend(format!("\\{b:03o}").bytes()),
        }
    }
    out.extend_from_slice(b") Tj T*\n");
}

fn content_stream(lines: &[(bool, String)]) -> Vec<u8> {
    let mut content = format!("BT\n{LEADING} TL 72 {START_Y} Td\n").into_bytes();
    for (bold, text) in lines {
        let (font, size) = if *bold { ("F2", TITLE_PT) } else { ("F1", BODY_PT) };
        content.extend(format!("/{font} {size} Tf ").bytes());
        push_text(&mut content, text);
    }
    content.extend_from_slice(b"ET\n");
    content
}

/// Assembles the objects: 1 catalog, 2 page tree, 3 and 4 the two fonts, then
/// a page and its content stream for each page of text.
fn write_document(pages: &[&[(bool, String)]]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let kids: Vec<String> = (0..pages.len()).map(|k| format!("{} 0 R", 5 + 2 * k)).collect();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
    for (k, page) in pages.iter().enumerate() {
        let content_no = 6 + 2 * k;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_no} 0 R >>").into_bytes());
        let stream = content_stream(page);
        let mut obj = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        obj.extend(stream);
        obj.extend_from_slice(b"endstream");
        objects.push(obj);
    }

    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n", objects.len() + 1).bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The byte offsets listed in the cross-reference table, in object order.
    fn xref_offsets(pdf: &[u8]) -> Vec<usize> {
        let text = String::from_utf8_lossy(pdf);
        let start = text.rfind("xref\n").unwrap();
        text[start..].lines().skip(3).take_while(|l| l.ends_with(" n ")).map(|l| l[..10].parse().unwrap()).collect()
    }

    #[test]
    fn xref_offsets_point_at_each_object_header() {
        let pdf = render("My List", "one\ntwo");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        let offsets = xref_offsets(&pdf);
        assert_eq!(offsets.len(), 6, "catalog, pages, two fonts and one page with its content");
        for (i, off) in offsets.iter().enumerate() {
            assert!(pdf[*off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
        }
        assert!(pdf.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn text_is_escaped_and_non_latin_characters_become_question_marks() {
        let pdf = render("T", "a (b) \\ c \u{e9} \u{2022} d");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains(r"(a \(b\) \\ c \351 ? d) Tj"), "{text}");
    }

    #[test]
    fn long_lines_wrap_at_words_and_keep_every_word() {
        let body = "word ".repeat(40);
        let lines = wrap(body.trim());
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| l.chars().count() <= WRAP));
        assert_eq!(lines.join(" ").split_whitespace().count(), 40);
    }

    #[test]
    fn a_word_longer_than_the_line_is_cut_not_dropped() {
        let word = "x".repeat(WRAP * 2 + 5);
        let lines = wrap(&word);
        assert_eq!(lines.concat().len(), word.len());
    }

    #[test]
    fn tables_flatten_to_rows_without_the_rule() {
        let rows = flatten("| Name | Qty |\n| --- | :-: |\n| eggs | 12 |");
        assert_eq!(rows, vec!["Name   |   Qty".to_string(), "eggs   |   12".to_string()]);
    }

    #[test]
    fn long_notes_paginate() {
        let body: String = (0..LINES_PER_PAGE * 2).map(|i| format!("line {i}\n")).collect();
        let pdf = render("Long", &body);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 3"), "title plus body needs three pages");
        assert_eq!(xref_offsets(&pdf).len(), 4 + 2 * 3);
    }

    #[test]
    fn empty_note_still_makes_one_page() {
        let pdf = render("", "");
        assert!(String::from_utf8_lossy(&pdf).contains("/Count 1"));
    }
}
