//! Import of spreadsheets (.xlsx) and presentations (.pptx) as plain text. A workbook becomes its sheets in workbook
//! order: one line per row, cells joined by tabs, shared strings resolved, a blank line between sheets. A deck becomes
//! its slides in order: one line per paragraph, a blank line between slides. Formulas and cell formats are not read;
//! the text the user sees is what comes in. The packages are read with the ZIP reader and the XML tokenizer that the
//! word processor already uses.

use crate::xml::{self, Token};
use crate::zipdoc;

/// Whether the bytes are an .xlsx workbook (a ZIP with a workbook part).
pub fn is_xlsx(bytes: &[u8]) -> bool {
    zipdoc::is_zip(bytes) && zipdoc::read_entry(bytes, "xl/workbook.xml").is_some()
}

/// Whether the bytes are a .pptx presentation (a ZIP with a presentation part).
pub fn is_pptx(bytes: &[u8]) -> bool {
    zipdoc::is_zip(bytes) && zipdoc::read_entry(bytes, "ppt/presentation.xml").is_some()
}

/// Whether the bytes are a spreadsheet or a presentation this module reads.
pub fn is_office(bytes: &[u8]) -> bool {
    is_xlsx(bytes) || is_pptx(bytes)
}

/// The plain text of a workbook or a deck, or `None` when the package has no readable text part.
pub fn text(bytes: &[u8]) -> Option<String> {
    if is_xlsx(bytes) {
        xlsx_text(bytes)
    } else if is_pptx(bytes) {
        pptx_text(bytes)
    } else {
        None
    }
}

/// The number in a worksheet part's name (`xl/worksheets/sheet3.xml` is 3), or `None` for another part.
fn sheet_number(name: &str) -> Option<u32> {
    name.strip_prefix("xl/worksheets/sheet")?.strip_suffix(".xml")?.parse().ok()
}

/// The number in a slide part's name (`ppt/slides/slide12.xml` is 12), or `None` for another part.
fn slide_number(name: &str) -> Option<u32> {
    name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()
}

/// The sheets of a workbook as text, in sheet-number order.
fn xlsx_text(bytes: &[u8]) -> Option<String> {
    let shared = zipdoc::read_entry(bytes, "xl/sharedStrings.xml").map(|x| shared_strings(&String::from_utf8_lossy(&x))).unwrap_or_default();
    let mut sheets: Vec<(u32, String)> = zipdoc::names(bytes).into_iter().filter_map(|name| sheet_number(&name).map(|n| (n, name))).collect();
    if sheets.is_empty() {
        return None;
    }
    sheets.sort();
    let texts: Vec<String> = sheets
        .iter()
        .filter_map(|(_, name)| zipdoc::read_entry(bytes, name))
        .map(|xml| sheet_text(&String::from_utf8_lossy(&xml), &shared))
        .collect();
    Some(texts.join("\n\n"))
}

/// The shared strings table: one string per `si`, its `t` runs joined.
fn shared_strings(xml_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    for token in xml::tokens(xml_text) {
        match token {
            Token::Open { name, empty, .. } if name == "si" => {
                current.clear();
                if empty {
                    out.push(String::new());
                }
            }
            Token::Open { name, empty: false, .. } if name == "t" => in_text = true,
            Token::Text(s) if in_text => current.push_str(&s),
            Token::Close(name) if name == "t" => in_text = false,
            Token::Close(name) if name == "si" => out.push(std::mem::take(&mut current)),
            _ => {}
        }
    }
    out
}

/// One worksheet as text: a line per row, its cells tab-separated. A cell typed `s` names a shared string.
fn sheet_text(xml_text: &str, shared: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell_type = String::new();
    let mut value = String::new();
    let mut in_text = false;
    for token in xml::tokens(xml_text) {
        match token {
            Token::Open { name, .. } if name == "row" => row.clear(),
            Token::Open { name, attrs, empty } if name == "c" => {
                cell_type = xml::attr(&attrs, "t").unwrap_or("n").to_string();
                value.clear();
                if empty {
                    row.push(String::new());
                }
            }
            Token::Open { name, empty: false, .. } if name == "v" || name == "t" => in_text = true,
            Token::Text(s) if in_text => value.push_str(&s),
            Token::Close(name) if name == "v" || name == "t" => in_text = false,
            Token::Close(name) if name == "c" => {
                let resolved = if cell_type == "s" {
                    value.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default()
                } else {
                    std::mem::take(&mut value)
                };
                row.push(resolved);
            }
            Token::Close(name) if name == "row" => lines.push(row.join("\t")),
            _ => {}
        }
    }
    lines.join("\n")
}

/// The slides of a deck as text, in slide-number order.
fn pptx_text(bytes: &[u8]) -> Option<String> {
    let mut slides: Vec<(u32, String)> = zipdoc::names(bytes).into_iter().filter_map(|name| slide_number(&name).map(|n| (n, name))).collect();
    if slides.is_empty() {
        return None;
    }
    slides.sort();
    let texts: Vec<String> = slides
        .iter()
        .filter_map(|(_, name)| zipdoc::read_entry(bytes, name))
        .map(|xml| slide_text(&String::from_utf8_lossy(&xml)))
        .collect();
    Some(texts.join("\n\n"))
}

/// One slide as text: each DrawingML paragraph (`a:p`) on its own line, its runs (`a:t`) joined.
fn slide_text(xml_text: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    for token in xml::tokens(xml_text) {
        match token {
            Token::Open { name, empty, .. } if name == "a:p" => {
                current.clear();
                if empty {
                    paragraphs.push(String::new());
                }
            }
            Token::Open { name, empty: false, .. } if name == "a:t" => in_text = true,
            Token::Text(s) if in_text => current.push_str(&s),
            Token::Close(name) if name == "a:t" => in_text = false,
            Token::Close(name) if name == "a:p" => paragraphs.push(std::mem::take(&mut current)),
            _ => {}
        }
    }
    paragraphs.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let entries: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (*n, b.as_bytes())).collect();
        zipdoc::write(&entries)
    }

    #[test]
    fn a_workbook_reads_its_sheets_with_shared_strings_resolved() {
        let bytes = package(&[
            ("xl/workbook.xml", "<workbook/>"),
            ("xl/sharedStrings.xml", "<sst><si><t>Name</t></si><si><r><t>Ur</t></r><r><t>sula</t></r></si></sst>"),
            ("xl/worksheets/sheet2.xml", r#"<worksheet><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>42</v></c><c r="C1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>inline</t></is></c><c r="B2"/></row></sheetData></worksheet>"#,
            ),
        ]);
        assert!(is_xlsx(&bytes) && is_office(&bytes) && !is_pptx(&bytes));
        // sheet1 before sheet2, whatever order the parts were written in
        assert_eq!(text(&bytes).unwrap(), "Name\t42\tUrsula\ninline\t\n\n7");
    }

    #[test]
    fn a_deck_reads_its_paragraphs_slide_by_slide() {
        let bytes = package(&[
            ("ppt/presentation.xml", "<p:presentation/>"),
            ("ppt/slides/slide10.xml", r#"<p:sld><a:p><a:r><a:t>Tenth</a:t></a:r></a:p></p:sld>"#),
            ("ppt/slides/slide1.xml", r#"<p:sld><a:p><a:r><a:t>Hello</a:t></a:r></a:p><a:p><a:r><a:t>World</a:t></a:r></a:p></p:sld>"#),
        ]);
        assert!(is_pptx(&bytes) && is_office(&bytes) && !is_xlsx(&bytes));
        assert_eq!(text(&bytes).unwrap(), "Hello\nWorld\n\nTenth");
    }

    #[test]
    fn a_package_without_a_workbook_or_deck_is_not_read() {
        let docx = package(&[("word/document.xml", "<w:document/>")]);
        assert!(!is_office(&docx));
        assert_eq!(text(&docx), None);
        assert_eq!(text(b"not a zip"), None);
        let empty = package(&[("xl/workbook.xml", "<workbook/>")]);
        assert_eq!(text(&empty), None, "a workbook with no sheets has no text");
    }
}
