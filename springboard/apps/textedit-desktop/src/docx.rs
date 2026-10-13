//! Word-processing documents in the Office Open XML package format (`.docx`): a ZIP of XML parts. The reader takes the
//! body's paragraphs and runs (bold, italic, underline, strikethrough, size, colour, highlight, baseline shift, font
//! family), paragraph alignment, line spacing and bullet or numbered lists, and the tabs and line breaks between them.
//! The writer emits the same subset, so a document survives a round trip. Pictures, tables' borders, headers and
//! footers, comments and tracked changes are not carried: their text, where it is in the body, is.

use crate::properties::{self, Properties};
use crate::text::{family_for_name, Align, Cell, ListKind, Style, COLORS, FAMILY_NAMES, LINE_BREAK, PAGE_BREAK, SPACINGS};
use crate::xml::{self, Token};
use crate::{rtf, zipdoc};
use std::collections::HashMap;

/// The document's file type.
pub const TYPE: &str = "org.openxmlformats.wordprocessingml.document";

/// The highlight names a run may carry, by palette colour index (the default entry means none).
const HIGHLIGHTS: [&str; 8] = ["none", "red", "darkYellow", "yellow", "green", "blue", "magenta", "lightGray"];

const NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// Whether the bytes are a `.docx` package: a ZIP with a main document part.
pub fn is_docx(bytes: &[u8]) -> bool {
    zipdoc::is_zip(bytes) && zipdoc::read_entry(bytes, "word/document.xml").is_some()
}

/// The palette highlight for a run's highlight name; `None` (no highlight) is 0.
fn highlight_index(name: &str) -> u8 {
    match name {
        "none" | "white" => 0,
        "red" | "darkRed" => 1,
        "darkYellow" => 2,
        "yellow" => 3,
        "green" | "darkGreen" => 4,
        "blue" | "cyan" | "darkBlue" | "darkCyan" => 5,
        "magenta" | "darkMagenta" => 6,
        "lightGray" | "darkGray" | "black" => 7,
        _ => 3,
    }
}

/// A heading style's size, from its style name ("Heading1", "Title").
fn heading_size(style: &str) -> Option<f32> {
    let lower = style.to_ascii_lowercase();
    if lower == "title" {
        return Some(26.0);
    }
    let level: u32 = lower.strip_prefix("heading")?.trim().parse().ok()?;
    Some(match level {
        1 => 20.0,
        2 => 16.0,
        3 => 14.0,
        _ => 12.0,
    })
}

/// Whether an on/off property is on: no value or anything but a false word.
fn on(attrs: &[(String, String)]) -> bool {
    !matches!(xml::attr(attrs, "w:val"), Some("0" | "false" | "off" | "none"))
}

/// The list kind of each numbering id, from the numbering part: a bullet format is a bullet list, any other a numbered
/// one.
fn list_kinds(numbering: &str) -> HashMap<String, ListKind> {
    let mut abstract_kind: HashMap<String, ListKind> = HashMap::new();
    let mut nums: Vec<(String, String)> = Vec::new();
    let mut current_abstract: Option<String> = None;
    let mut top_level = false;
    let mut current_num: Option<String> = None;
    for token in xml::tokens(numbering) {
        match token {
            Token::Open { name, attrs, .. } => match name.as_str() {
                "w:abstractNum" => {
                    current_abstract = xml::attr(&attrs, "w:abstractNumId").map(str::to_string);
                    top_level = false;
                }
                "w:lvl" => top_level = xml::attr(&attrs, "w:ilvl") == Some("0"),
                "w:numFmt" if top_level => {
                    if let Some(id) = &current_abstract {
                        let kind = if xml::attr(&attrs, "w:val") == Some("bullet") { ListKind::Bullet } else { ListKind::Numbered };
                        abstract_kind.entry(id.clone()).or_insert(kind);
                    }
                }
                "w:num" => current_num = xml::attr(&attrs, "w:numId").map(str::to_string),
                "w:abstractNumId" => {
                    if let (Some(num), Some(value)) = (&current_num, xml::attr(&attrs, "w:val")) {
                        nums.push((num.clone(), value.to_string()));
                    }
                }
                _ => {}
            },
            Token::Close(name) => match name.as_str() {
                "w:abstractNum" => current_abstract = None,
                "w:lvl" => top_level = false,
                "w:num" => current_num = None,
                _ => {}
            },
            Token::Text(_) => {}
        }
    }
    nums.into_iter().filter_map(|(num, id)| abstract_kind.get(&id).map(|kind| (num, *kind))).collect()
}

/// Reads a `.docx` package into cells. `None` when the package has no readable main document part.
pub fn read(bytes: &[u8]) -> Option<Vec<Cell>> {
    let document = zipdoc::read_entry(bytes, "word/document.xml")?;
    let lists = zipdoc::read_entry(bytes, "word/numbering.xml").map(|n| list_kinds(&String::from_utf8_lossy(&n))).unwrap_or_default();
    Some(parse(&String::from_utf8_lossy(&document), &lists))
}

/// The cells of the main document part. A paragraph's newline is added when the next paragraph starts, so the last
/// paragraph has none, as in a file written by `write`.
fn parse(document: &str, lists: &HashMap<String, ListKind>) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    let mut para = Style::default();
    let mut run = Style::default();
    let mut heading: Option<f32> = None;
    let mut size_set = false;
    let mut seen_paragraph = false;
    let (mut in_ppr, mut in_rpr, mut in_text) = (false, false, false);
    let cell_style = |para: &Style, run: &Style, heading: Option<f32>, size_set: bool| {
        let mut s = Style { align: para.align, spacing: para.spacing, list: para.list, ..*run };
        if let Some(h) = heading {
            s.bold = true;
            if !size_set {
                s.size = h;
            }
        }
        s
    };
    for token in xml::tokens(document) {
        match token {
            Token::Open { name, attrs, empty } => match name.as_str() {
                "w:p" => {
                    if seen_paragraph {
                        // the break ends the paragraph before, which keeps that paragraph's properties
                        cells.push(Cell { ch: '\n', style: Style { align: para.align, spacing: para.spacing, list: para.list, ..Style::default() } });
                    }
                    seen_paragraph = true;
                    para = Style::default();
                    run = Style::default();
                    heading = None;
                    size_set = false;
                }
                "w:pPr" => in_ppr = !empty,
                "w:rPr" => in_rpr = !empty,
                "w:r" => {
                    run = Style::default();
                    size_set = false;
                }
                "w:t" => in_text = !empty,
                "w:pStyle" if in_ppr => heading = xml::attr(&attrs, "w:val").and_then(heading_size),
                "w:jc" if in_ppr => {
                    para.align = match xml::attr(&attrs, "w:val") {
                        Some("center") => Align::Center,
                        Some("right" | "end") => Align::Right,
                        Some("both" | "distribute" | "justify") => Align::Justified,
                        _ => Align::Left,
                    };
                }
                "w:spacing" if in_ppr && !in_rpr => {
                    let auto = matches!(xml::attr(&attrs, "w:lineRule"), None | Some("auto"));
                    if let Some(line) = xml::attr(&attrs, "w:line").and_then(|v| v.parse::<f32>().ok()).filter(|&l| auto && l > 0.0) {
                        para.spacing = rtf::nearest_spacing(line / 240.0);
                    }
                }
                "w:numId" if in_ppr => {
                    if let Some(id) = xml::attr(&attrs, "w:val").filter(|id| *id != "0") {
                        para.list = lists.get(id).copied().unwrap_or(ListKind::Bullet);
                    }
                }
                "w:b" if in_rpr => run.bold = on(&attrs),
                "w:i" if in_rpr => run.italic = on(&attrs),
                "w:u" if in_rpr => run.underline = on(&attrs),
                "w:strike" | "w:dstrike" if in_rpr => run.strike = on(&attrs),
                "w:sz" if in_rpr => {
                    if let Some(half) = xml::attr(&attrs, "w:val").and_then(|v| v.parse::<f32>().ok()).filter(|&h| h > 0.0) {
                        run.size = (half / 2.0).clamp(crate::text::MIN_SIZE, crate::text::MAX_SIZE);
                        size_set = true;
                    }
                }
                "w:color" if in_rpr => {
                    run.color = match xml::attr(&attrs, "w:val") {
                        Some(hex) if hex.len() == 6 && hex.is_ascii() => {
                            let channel = |at: usize| i32::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
                            rtf::text_color([channel(0), channel(2), channel(4)])
                        }
                        _ => 0,
                    };
                }
                "w:highlight" if in_rpr => run.highlight = xml::attr(&attrs, "w:val").map_or(0, highlight_index),
                "w:vertAlign" if in_rpr => {
                    run.script = match xml::attr(&attrs, "w:val") {
                        Some("superscript") => 1,
                        Some("subscript") => -1,
                        _ => 0,
                    };
                }
                "w:rFonts" if in_rpr => {
                    if let Some(font) = xml::attr(&attrs, "w:ascii").or_else(|| xml::attr(&attrs, "w:hAnsi")) {
                        run.font = family_for_name(font);
                    }
                }
                "w:tab" if !in_ppr => cells.push(Cell { ch: '\t', style: cell_style(&para, &run, heading, size_set) }),
                "w:br" | "w:cr" => {
                    let ch = if xml::attr(&attrs, "w:type") == Some("page") { PAGE_BREAK } else { LINE_BREAK };
                    cells.push(Cell { ch, style: cell_style(&para, &run, heading, size_set) });
                }
                _ => {}
            },
            Token::Close(name) => match name.as_str() {
                "w:pPr" => in_ppr = false,
                "w:rPr" => in_rpr = false,
                "w:t" => in_text = false,
                _ => {}
            },
            Token::Text(text) => {
                if in_text {
                    let style = cell_style(&para, &run, heading, size_set);
                    cells.extend(text.chars().filter(|c| !c.is_control()).map(|ch| Cell { ch, style }));
                }
            }
        }
    }
    cells
}

/// The numbering part: id 1 is the bullet list and id 2 the numbered one.
fn numbering_part() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{NS}\">\
<w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"hybridMultilevel\"/>\
<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"\u{2022}\"/><w:lvlJc w:val=\"left\"/>\
<w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum>\
<w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"hybridMultilevel\"/>\
<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/>\
<w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum>\
<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>\
<w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num></w:numbering>"
    )
}

fn run_properties(style: &Style) -> String {
    let mut out = String::new();
    let font = FAMILY_NAMES[(style.font as usize).min(FAMILY_NAMES.len() - 1)];
    out += &format!("<w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:cs=\"{font}\"/>");
    if style.bold {
        out += "<w:b/>";
    }
    if style.italic {
        out += "<w:i/>";
    }
    if style.strike {
        out += "<w:strike/>";
    }
    if style.color > 0 {
        let [r, g, b] = COLORS[(style.color as usize).min(COLORS.len() - 1)].1;
        out += &format!("<w:color w:val=\"{r:02X}{g:02X}{b:02X}\"/>");
    }
    out += &format!("<w:sz w:val=\"{}\"/>", (style.size * 2.0).round() as i32);
    if style.highlight > 0 {
        out += &format!("<w:highlight w:val=\"{}\"/>", HIGHLIGHTS[(style.highlight as usize).min(HIGHLIGHTS.len() - 1)]);
    }
    if style.underline {
        out += "<w:u w:val=\"single\"/>";
    }
    match style.script {
        1 => out += "<w:vertAlign w:val=\"superscript\"/>",
        -1 => out += "<w:vertAlign w:val=\"subscript\"/>",
        _ => {}
    }
    out
}

fn run_xml(cells: &[Cell], out: &mut String) {
    out.push_str("<w:r><w:rPr>");
    out.push_str(&run_properties(&cells[0].style));
    out.push_str("</w:rPr>");
    let mut buffer = String::new();
    let flush = |buffer: &mut String, out: &mut String| {
        if !buffer.is_empty() {
            out.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>", xml::escape(buffer)));
            buffer.clear();
        }
    };
    for cell in cells {
        match cell.ch {
            '\t' => {
                flush(&mut buffer, out);
                out.push_str("<w:tab/>");
            }
            LINE_BREAK => {
                flush(&mut buffer, out);
                out.push_str("<w:br/>");
            }
            PAGE_BREAK => {
                flush(&mut buffer, out);
                out.push_str("<w:br w:type=\"page\"/>");
            }
            c if c.is_control() => {}
            c => buffer.push(c),
        }
    }
    flush(&mut buffer, out);
    out.push_str("</w:r>");
}

fn paragraph_xml(cells: &[Cell], style: &Style, out: &mut String) {
    out.push_str("<w:p><w:pPr>");
    match style.list {
        ListKind::None => {}
        ListKind::Bullet => out.push_str("<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr>"),
        ListKind::Numbered => out.push_str("<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"2\"/></w:numPr>"),
    }
    if style.spacing > 0 {
        let multiple = SPACINGS.get(style.spacing as usize).copied().unwrap_or(1.0);
        out.push_str(&format!("<w:spacing w:line=\"{}\" w:lineRule=\"auto\"/>", (multiple * 240.0).round() as i32));
    }
    let jc = match style.align {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justified => "both",
    };
    out.push_str(&format!("<w:jc w:val=\"{jc}\"/></w:pPr>"));
    let mut i = 0;
    while i < cells.len() {
        let mut j = i + 1;
        while j < cells.len() && cells[i].style.same_run(&cells[j].style) {
            j += 1;
        }
        run_xml(&cells[i..j], out);
        i = j;
    }
    out.push_str("</w:p>");
}

/// The document properties in a `.docx` package (title, subject, author, keywords); a package without them has none.
pub fn read_properties(bytes: &[u8]) -> Properties {
    zipdoc::read_entry(bytes, "docProps/core.xml").map(|core| properties::read_xml(&String::from_utf8_lossy(&core))).unwrap_or_default()
}

/// The core properties part for the properties that a `.docx` package carries (the copyright has no place in it).
fn core_part(props: &Properties) -> String {
    let mut body = String::new();
    for (tag, index) in [("dc:title", 0), ("dc:subject", 2), ("dc:creator", 1), ("cp:keywords", 3)] {
        let value = props.get(index);
        if !value.is_empty() {
            body += &format!("<{tag}>{}</{tag}>", xml::escape(value));
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties \
xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">{body}</cp:coreProperties>"
    )
}

/// The cells as a `.docx` package without properties.
#[cfg_attr(not(test), allow(dead_code))]
pub fn write(cells: &[Cell]) -> Vec<u8> {
    write_with(cells, &Properties::default())
}

/// The cells as a `.docx` package carrying the document properties.
pub fn write_with(cells: &[Cell], props: &Properties) -> Vec<u8> {
    let mut body = String::new();
    let mut start = 0;
    loop {
        let end = cells[start..].iter().position(|c| c.ch == '\n').map_or(cells.len(), |i| start + i);
        // the paragraph's properties are on its cells and its newline
        let style = cells.get(end).or_else(|| cells.get(start)).map(|c| c.style).unwrap_or_default();
        paragraph_xml(&cells[start..end], &style, &mut body);
        if end >= cells.len() {
            break;
        }
        start = end + 1;
    }
    let document = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{NS}\"><w:body>{body}</w:body></w:document>");
    let has_core = (0..4).any(|i| !props.get(i).is_empty());
    let core_type = if has_core {
        "<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>"
    } else {
        ""
    };
    let core_rel = if has_core {
        "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>"
    } else {
        ""
    };
    let content_types = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>{core_type}</Types>");
    let root_rels = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>{core_rel}</Relationships>");
    let document_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/></Relationships>";
    let numbering = numbering_part();
    let core = core_part(props);
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", document_rels.as_bytes()),
        ("word/numbering.xml", numbering.as_bytes()),
    ];
    if has_core {
        parts.push(("docProps/core.xml", core.as_bytes()));
    }
    zipdoc::write(&parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char, style: Style) -> Cell {
        Cell { ch, style }
    }

    fn text_of(cells: &[Cell]) -> String {
        cells.iter().map(|c| c.ch).collect()
    }

    #[test]
    fn formatting_alignment_spacing_and_lists_round_trip() {
        let bold = Style { bold: true, size: 18.0, color: 5, font: 1, ..Style::default() };
        let fancy = Style { italic: true, underline: true, strike: true, highlight: 3, script: 1, ..Style::default() };
        let centred = Style { align: Align::Center, spacing: 2, ..Style::default() };
        let listed = Style { list: ListKind::Numbered, ..Style::default() };
        let bullet = Style { list: ListKind::Bullet, ..Style::default() };
        let cells = vec![
            cell('a', bold),
            cell('&', fancy),
            cell('<', Style::default()),
            cell('\n', Style::default()),
            cell('c', centred),
            cell('\t', centred),
            cell('d', centred),
            cell('\n', centred),
            cell('x', listed),
            cell('\n', listed),
            cell('y', bullet),
        ];
        let package = write(&cells);
        assert!(is_docx(&package));
        assert_eq!(read(&package), Some(cells));
    }

    #[test]
    fn line_and_page_breaks_round_trip() {
        let cells: Vec<Cell> = "a\u{2028}b\u{c}c".chars().map(|ch| Cell { ch, style: Style::default() }).collect();
        let package = write(&cells);
        assert_eq!(read(&package), Some(cells));
    }

    #[test]
    fn the_document_properties_round_trip_and_an_unlabelled_package_has_none() {
        let mut props = Properties::default();
        props.title = "Plan & notes".into();
        props.author = "Ada".into();
        props.subject = "Q3".into();
        props.keywords = "a, b".into();
        props.copyright = "not carried".into();
        let package = write_with(&[], &props);
        let back = read_properties(&package);
        assert_eq!((back.title.as_str(), back.author.as_str(), back.subject.as_str(), back.keywords.as_str()), ("Plan & notes", "Ada", "Q3", "a, b"));
        assert_eq!(back.copyright, "", "a package has no copyright field");
        assert_eq!(read(&package), Some(Vec::new()));
        assert!(read_properties(&write(&[])).is_empty());
        assert!(!zipdoc::names(&write(&[])).iter().any(|n| n.starts_with("docProps")));
    }

    #[test]
    fn an_empty_document_and_empty_paragraphs_survive() {
        assert_eq!(read(&write(&[])), Some(Vec::new()));
        let cells = vec![cell('a', Style::default()), cell('\n', Style::default()), cell('\n', Style::default()), cell('b', Style::default())];
        assert_eq!(text_of(&read(&write(&cells)).unwrap()), "a\n\nb");
    }

    #[test]
    fn a_foreign_document_reads_its_text_runs_and_paragraph_properties() {
        let document = "<w:document xmlns:w=\"x\"><w:body>\
<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/><w:jc w:val=\"center\"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>\
<w:p><w:r><w:rPr><w:b w:val=\"0\"/><w:i/><w:color w:val=\"FF0000\"/><w:sz w:val=\"28\"/><w:rFonts w:ascii=\"Courier New\"/></w:rPr>\
<w:t xml:space=\"preserve\">one </w:t><w:tab/><w:t>two</w:t></w:r><w:r><w:rPr><w:vertAlign w:val=\"subscript\"/></w:rPr><w:t>3</w:t></w:r></w:p>\
<w:p/><w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"7\"/></w:numPr><w:tabs><w:tab w:val=\"left\" w:pos=\"720\"/></w:tabs></w:pPr><w:r><w:t>item</w:t></w:r></w:p>\
<w:p><w:r><w:delText>gone</w:delText></w:r><w:r><w:t>kept</w:t></w:r></w:p>\
</w:body></w:document>";
        let numbering = "<w:numbering xmlns:w=\"x\"><w:abstractNum w:abstractNumId=\"3\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/></w:lvl></w:abstractNum>\
<w:num w:numId=\"7\"><w:abstractNumId w:val=\"3\"/></w:num></w:numbering>";
        let lists = list_kinds(numbering);
        assert_eq!(lists.get("7"), Some(&ListKind::Numbered));
        let cells = parse(document, &lists);
        assert_eq!(text_of(&cells), "Title\none \ttwo3\n\nitem\nkept");
        assert!(cells[0].style.bold && cells[0].style.size == 20.0 && cells[0].style.align == Align::Center, "a heading");
        let one = &cells[6];
        assert!(!one.style.bold && one.style.italic && one.style.color == 1 && one.style.size == 14.0 && one.style.font == 2);
        let sub = cells.iter().find(|c| c.ch == '3').unwrap();
        assert_eq!(sub.style.script, -1);
        assert!(!sub.style.italic, "formatting does not leak into the next run");
        let item = cells.iter().rev().find(|c| c.ch == 'i').unwrap();
        assert_eq!(item.style.list, ListKind::Numbered, "a tab stop in the paragraph properties is not a tab character");
        assert_eq!(text_of(&cells).matches('\t').count(), 1);
    }

    #[test]
    fn a_package_without_a_main_part_is_not_a_document() {
        let other = zipdoc::write(&[("a.txt", &b"x"[..])]);
        assert!(!is_docx(&other));
        assert_eq!(read(&other), None);
        assert!(!is_docx(b"plain"));
    }

    #[test]
    fn a_deflated_package_reads_like_a_stored_one() {
        let cells = vec![cell('h', Style { bold: true, ..Style::default() }), cell('i', Style::default())];
        let stored = write(&cells);
        let document = zipdoc::read_entry(&stored, "word/document.xml").unwrap();
        let packed = zipdoc::write_deflated(&[("word/document.xml", document.as_slice())]);
        assert_eq!(read(&packed), Some(cells));
    }
}
