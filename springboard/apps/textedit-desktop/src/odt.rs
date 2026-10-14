//! OpenDocument text (`.odt`): a ZIP whose `content.xml` holds the body and the automatic styles, and whose
//! `styles.xml` may hold named ones. The reader resolves each paragraph's and span's style (through its parent styles)
//! into bold, italic, underline, strikethrough, size, colour, highlight, baseline shift and font family, and the
//! paragraph's alignment, line spacing and list; spaces, tabs and line breaks follow the format's whitespace rules.
//! The writer emits the same subset, so a document survives a round trip.

use crate::links;
use crate::properties::{self, Properties};
use crate::rtf;
use crate::text::{family_for_name, Align, Cell, ListKind, Style, COLORS, DEFAULT_SIZE, FAMILY_NAMES, MAX_PARAGRAPH_SPACE, SPACINGS};
use crate::xml::{self, Token};
use crate::zipdoc;
use std::collections::HashMap;

/// The document's file type.
pub const TYPE: &str = "org.oasis-open.opendocument.text";

const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

/// Whether the bytes are an OpenDocument text package.
pub fn is_odt(bytes: &[u8]) -> bool {
    zipdoc::is_zip(bytes)
        && zipdoc::read_entry(bytes, "mimetype").is_some_and(|m| m.starts_with(MIMETYPE.as_bytes()))
        && zipdoc::read_entry(bytes, "content.xml").is_some()
}

/// The properties a style sets; a property it leaves out is `None`, so a span can override only what it names.
#[derive(Clone, Debug, Default, PartialEq)]
struct Props {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    size: Option<f32>,
    color: Option<u8>,
    highlight: Option<u8>,
    script: Option<i8>,
    font: Option<u8>,
    align: Option<Align>,
    spacing: Option<u8>,
    /// paragraph space before and after, in points, and the right to left writing mode
    space_before: Option<u8>,
    space_after: Option<u8>,
    rtl: Option<bool>,
    /// the link of an anchor, set by `text:a` and carried like a span's properties
    link: Option<u16>,
}

impl Props {
    /// Lets the properties of `over` win where it sets them.
    fn merge(&mut self, over: &Props) {
        fn pick<T: Clone>(mine: &mut Option<T>, theirs: &Option<T>) {
            if theirs.is_some() {
                *mine = theirs.clone();
            }
        }
        pick(&mut self.bold, &over.bold);
        pick(&mut self.italic, &over.italic);
        pick(&mut self.underline, &over.underline);
        pick(&mut self.strike, &over.strike);
        pick(&mut self.size, &over.size);
        pick(&mut self.color, &over.color);
        pick(&mut self.highlight, &over.highlight);
        pick(&mut self.script, &over.script);
        pick(&mut self.font, &over.font);
        pick(&mut self.align, &over.align);
        pick(&mut self.spacing, &over.spacing);
        pick(&mut self.space_before, &over.space_before);
        pick(&mut self.space_after, &over.space_after);
        pick(&mut self.rtl, &over.rtl);
        pick(&mut self.link, &over.link);
    }

    /// The character properties onto a style.
    fn apply(&self, style: &mut Style) {
        if let Some(v) = self.bold {
            style.bold = v;
        }
        if let Some(v) = self.italic {
            style.italic = v;
        }
        if let Some(v) = self.underline {
            style.underline = v;
        }
        if let Some(v) = self.strike {
            style.strike = v;
        }
        if let Some(v) = self.size {
            style.size = v.clamp(crate::text::MIN_SIZE, crate::text::MAX_SIZE);
        }
        if let Some(v) = self.color {
            style.color = v;
        }
        if let Some(v) = self.highlight {
            style.highlight = v;
        }
        if let Some(v) = self.script {
            style.script = v;
        }
        if let Some(v) = self.font {
            style.font = v;
        }
        if let Some(v) = self.link {
            style.link = v;
        }
    }
}

/// A named style: its parent and what it sets itself.
#[derive(Clone, Debug, Default)]
struct StyleDef {
    parent: Option<String>,
    props: Props,
}

/// The properties of a style with its parents' underneath (up to eight levels).
fn resolve(defs: &HashMap<String, StyleDef>, name: &str) -> Props {
    let mut chain: Vec<&StyleDef> = Vec::new();
    let mut at = Some(name);
    while let Some(n) = at {
        let Some(def) = defs.get(n) else { break };
        chain.push(def);
        if chain.len() >= 8 {
            break;
        }
        at = def.parent.as_deref();
    }
    let mut out = Props::default();
    for def in chain.iter().rev() {
        out.merge(&def.props);
    }
    out
}

/// A colour attribute (`#rrggbb`) as a palette index; `None` for anything else.
fn color_of(value: &str) -> Option<u8> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |at: usize| i32::from_str_radix(&hex[at..at + 2], 16).ok();
    Some(rtf::text_color([channel(0)?, channel(2)?, channel(4)?]))
}

/// A length in points ("12pt", "10.5pt"); `None` for another unit.
fn points(value: &str) -> Option<f32> {
    value.strip_suffix("pt")?.trim().parse().ok().filter(|&v: &f32| v > 0.0)
}

fn on(value: &str) -> bool {
    value != "none" && value != "normal"
}

/// Reads the text properties of a style element into `props`.
fn text_properties(attrs: &[(String, String)], props: &mut Props) {
    for (key, value) in attrs {
        match key.as_str() {
            "fo:font-weight" => props.bold = Some(value == "bold" || value.parse::<u32>().is_ok_and(|w| w >= 600)),
            "fo:font-style" => props.italic = Some(value == "italic" || value == "oblique"),
            "style:text-underline-style" => props.underline = Some(on(value)),
            "style:text-line-through-style" => props.strike = Some(on(value)),
            "fo:font-size" => {
                if let Some(size) = points(value) {
                    props.size = Some(size);
                }
            }
            "fo:color" => props.color = color_of(value),
            "fo:background-color" => props.highlight = Some(if value == "transparent" { 0 } else { color_of(value).filter(|&c| c > 0).unwrap_or(0) }),
            "style:text-position" => {
                let first = value.split_whitespace().next().unwrap_or("");
                props.script = Some(if first.starts_with("super") {
                    1
                } else if first.starts_with("sub") {
                    -1
                } else {
                    match first.trim_end_matches('%').parse::<f32>() {
                        Ok(v) if v > 0.0 => 1,
                        Ok(v) if v < 0.0 => -1,
                        _ => 0,
                    }
                });
            }
            "fo:font-family" | "style:font-name" => props.font = Some(family_for_name(value.trim_matches('\''))),
            _ => {}
        }
    }
}

fn paragraph_properties(attrs: &[(String, String)], props: &mut Props) {
    for (key, value) in attrs {
        match key.as_str() {
            "fo:text-align" => {
                props.align = Some(match value.as_str() {
                    "center" => Align::Center,
                    "end" | "right" => Align::Right,
                    "justify" => Align::Justified,
                    _ => Align::Left,
                });
            }
            "fo:line-height" => {
                if let Some(percent) = value.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()).filter(|&p| p > 0.0) {
                    props.spacing = Some(rtf::nearest_spacing(percent / 100.0));
                }
            }
            "fo:margin-top" => props.space_before = points(value).map(|p| p.round().min(MAX_PARAGRAPH_SPACE as f32) as u8),
            "fo:margin-bottom" => props.space_after = points(value).map(|p| p.round().min(MAX_PARAGRAPH_SPACE as f32) as u8),
            "style:writing-mode" => props.rtl = Some(value == "rl-tb"),
            _ => {}
        }
    }
}

/// Collects the style definitions and list styles of one XML part.
fn collect_styles(part: &str, defs: &mut HashMap<String, StyleDef>, lists: &mut HashMap<String, ListKind>) {
    let mut current: Option<String> = None;
    let mut list: Option<String> = None;
    for token in xml::tokens(part) {
        match token {
            Token::Open { name, attrs, empty } => match name.as_str() {
                "style:style" => {
                    if let Some(style_name) = xml::attr(&attrs, "style:name") {
                        let parent = xml::attr(&attrs, "style:parent-style-name").map(str::to_string);
                        defs.insert(style_name.to_string(), StyleDef { parent, props: Props::default() });
                        current = if empty { None } else { Some(style_name.to_string()) };
                    }
                }
                "style:text-properties" => {
                    if let Some(def) = current.as_ref().and_then(|n| defs.get_mut(n)) {
                        text_properties(&attrs, &mut def.props);
                    }
                }
                "style:paragraph-properties" => {
                    if let Some(def) = current.as_ref().and_then(|n| defs.get_mut(n)) {
                        paragraph_properties(&attrs, &mut def.props);
                    }
                }
                "text:list-style" => list = xml::attr(&attrs, "style:name").map(str::to_string),
                "text:list-level-style-bullet" => {
                    if let Some(l) = &list {
                        lists.entry(l.clone()).or_insert(ListKind::Bullet);
                    }
                }
                "text:list-level-style-number" | "text:list-level-style-image" => {
                    if let Some(l) = &list {
                        lists.entry(l.clone()).or_insert(ListKind::Numbered);
                    }
                }
                _ => {}
            },
            Token::Close(name) => match name.as_str() {
                "style:style" => current = None,
                "text:list-style" => list = None,
                _ => {}
            },
            Token::Text(_) => {}
        }
    }
}

/// A heading's size by outline level.
fn heading_size(level: u32) -> f32 {
    match level {
        1 => 20.0,
        2 => 16.0,
        3 => 14.0,
        _ => DEFAULT_SIZE,
    }
}

/// The document properties in an OpenDocument package (`meta.xml`); a package without them has none.
pub fn read_properties(bytes: &[u8]) -> Properties {
    zipdoc::read_entry(bytes, "meta.xml").map(|meta| properties::read_xml(&String::from_utf8_lossy(&meta))).unwrap_or_default()
}

/// The metadata part for the properties that an OpenDocument package carries (the copyright has no place in it).
fn meta_part(props: &Properties) -> String {
    let mut body = String::new();
    for (tag, index) in [("dc:title", 0), ("dc:subject", 2), ("meta:initial-creator", 1)] {
        let value = props.get(index);
        if !value.is_empty() {
            body += &format!("<{tag}>{}</{tag}>", xml::escape(value));
        }
    }
    for keyword in props.keywords.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        body += &format!("<meta:keyword>{}</meta:keyword>", xml::escape(keyword));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" office:version=\"1.2\"><office:meta>{body}</office:meta></office:document-meta>"
    )
}

/// Reads an OpenDocument text package into cells. `None` when it has no readable content part.
pub fn read(bytes: &[u8]) -> Option<Vec<Cell>> {
    let content = zipdoc::read_entry(bytes, "content.xml")?;
    let mut defs = HashMap::new();
    let mut lists = HashMap::new();
    if let Some(styles) = zipdoc::read_entry(bytes, "styles.xml") {
        collect_styles(&String::from_utf8_lossy(&styles), &mut defs, &mut lists);
    }
    let content = String::from_utf8_lossy(&content).into_owned();
    collect_styles(&content, &mut defs, &mut lists);
    Some(parse(&content, &defs, &lists))
}

/// The cells of the body. A paragraph's newline is added when the next paragraph starts.
fn parse(content: &str, defs: &HashMap<String, StyleDef>, lists: &HashMap<String, ListKind>) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    let mut seen_paragraph = false;
    let mut in_paragraph = false;
    let mut last_space = true;
    let mut skip = 0usize;
    let mut para_style = Style::default();
    let mut para_props = Props::default();
    let mut spans: Vec<Props> = Vec::new();
    let mut list_stack: Vec<ListKind> = Vec::new();
    for token in xml::tokens(content) {
        let current = |para_style: &Style, para_props: &Props, spans: &[Props]| {
            let mut props = para_props.clone();
            for span in spans {
                props.merge(span);
            }
            let mut style = *para_style;
            props.apply(&mut style);
            style
        };
        match token {
            Token::Open { name, attrs, empty } => {
                if skip > 0 {
                    if !empty && matches!(name.as_str(), "text:note" | "office:annotation") {
                        skip += 1;
                    }
                    continue;
                }
                match name.as_str() {
                    "text:note" | "office:annotation" => {
                        if !empty {
                            skip = 1;
                        }
                    }
                    "text:p" | "text:h" => {
                        if seen_paragraph {
                            cells.push(Cell { ch: '\n', style: para_style });
                        }
                        seen_paragraph = true;
                        let mut props = xml::attr(&attrs, "text:style-name").map(|n| resolve(defs, n)).unwrap_or_default();
                        if name == "text:h" {
                            let level = xml::attr(&attrs, "text:outline-level").and_then(|l| l.parse().ok()).unwrap_or(1);
                            props.bold.get_or_insert(true);
                            props.size.get_or_insert(heading_size(level));
                        }
                        para_style = Style {
                            align: props.align.unwrap_or(Align::Left),
                            spacing: props.spacing.unwrap_or(0),
                            space_before: props.space_before.unwrap_or(0),
                            space_after: props.space_after.unwrap_or(0),
                            rtl: props.rtl.unwrap_or(false),
                            list: list_stack.last().copied().unwrap_or(ListKind::None),
                            ..Style::default()
                        };
                        para_props = props;
                        spans.clear();
                        in_paragraph = !empty;
                        last_space = true;
                    }
                    "text:span" => {
                        let props = xml::attr(&attrs, "text:style-name").map(|n| resolve(defs, n)).unwrap_or_default();
                        if !empty {
                            spans.push(props);
                        }
                    }
                    "text:a" => {
                        // an anchor is a span that carries a link; one with no usable address is a plain span
                        let link = xml::attr(&attrs, "xlink:href").and_then(links::normalize).map(|url| links::intern(&url));
                        if !empty {
                            spans.push(Props { link, ..Props::default() });
                        }
                    }
                    "text:list" => {
                        let kind = xml::attr(&attrs, "text:style-name")
                            .and_then(|n| lists.get(n).copied())
                            .or_else(|| list_stack.last().copied())
                            .unwrap_or(ListKind::Bullet);
                        if !empty {
                            list_stack.push(kind);
                        }
                    }
                    "text:s" if in_paragraph => {
                        let count = xml::attr(&attrs, "text:c").and_then(|c| c.parse::<usize>().ok()).unwrap_or(1).min(1000);
                        let style = current(&para_style, &para_props, spans.as_slice());
                        cells.extend(std::iter::repeat(Cell { ch: ' ', style }).take(count));
                        last_space = false;
                    }
                    "text:tab" if in_paragraph => {
                        cells.push(Cell { ch: '\t', style: current(&para_style, &para_props, spans.as_slice()) });
                        last_space = false;
                    }
                    "text:line-break" if in_paragraph => {
                        cells.push(Cell { ch: crate::text::LINE_BREAK, style: current(&para_style, &para_props, spans.as_slice()) });
                        last_space = true;
                    }
                    _ => {}
                }
            }
            Token::Close(name) => {
                if skip > 0 {
                    if matches!(name.as_str(), "text:note" | "office:annotation") {
                        skip -= 1;
                    }
                    continue;
                }
                match name.as_str() {
                    "text:p" | "text:h" => in_paragraph = false,
                    "text:span" | "text:a" => {
                        spans.pop();
                    }
                    "text:list" => {
                        list_stack.pop();
                    }
                    _ => {}
                }
            }
            Token::Text(text) => {
                if skip == 0 && in_paragraph {
                    let style = current(&para_style, &para_props, spans.as_slice());
                    for ch in text.chars() {
                        if ch.is_whitespace() {
                            if !last_space {
                                cells.push(Cell { ch: ' ', style });
                                last_space = true;
                            }
                        } else if !ch.is_control() {
                            cells.push(Cell { ch, style });
                            last_space = false;
                        }
                    }
                }
            }
        }
    }
    cells
}

fn rgb_hex(index: u8) -> String {
    let [r, g, b] = COLORS[(index as usize).min(COLORS.len() - 1)].1;
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The attributes of a character style: only what differs from plain 12 point text.
fn text_style_attrs(style: &Style) -> String {
    let mut out = String::new();
    if style.bold {
        out += " fo:font-weight=\"bold\"";
    }
    if style.italic {
        out += " fo:font-style=\"italic\"";
    }
    if style.underline {
        out += " style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"";
    }
    if style.strike {
        out += " style:text-line-through-style=\"solid\"";
    }
    if style.size != DEFAULT_SIZE {
        out += &format!(" fo:font-size=\"{}pt\"", style.size);
    }
    if style.color > 0 {
        out += &format!(" fo:color=\"{}\"", rgb_hex(style.color));
    }
    if style.highlight > 0 {
        out += &format!(" fo:background-color=\"{}\"", rgb_hex(style.highlight));
    }
    match style.script {
        1 => out += " style:text-position=\"super 58%\"",
        -1 => out += " style:text-position=\"sub 58%\"",
        _ => {}
    }
    if style.font > 0 {
        out += &format!(" fo:font-family=\"{}\"", FAMILY_NAMES[(style.font as usize).min(FAMILY_NAMES.len() - 1)]);
    }
    out
}

/// Writes the characters of a run, spaces after the first of a row (and at a paragraph's start) as `text:s`.
fn run_text(cells: &[Cell], prev_space: &mut bool, out: &mut String) {
    let mut buffer = String::new();
    for cell in cells {
        match cell.ch {
            ' ' => {
                if *prev_space {
                    out.push_str(&xml::escape(&buffer));
                    buffer.clear();
                    out.push_str("<text:s/>");
                } else {
                    buffer.push(' ');
                }
                *prev_space = true;
            }
            '\t' => {
                out.push_str(&xml::escape(&buffer));
                buffer.clear();
                out.push_str("<text:tab/>");
                *prev_space = false;
            }
            // a page break has no inline form in this format: it is written as a line break
            crate::text::LINE_BREAK | crate::text::PAGE_BREAK => {
                out.push_str(&xml::escape(&buffer));
                buffer.clear();
                out.push_str("<text:line-break/>");
                *prev_space = true;
            }
            c if c.is_control() => {}
            c => {
                buffer.push(c);
                *prev_space = false;
            }
        }
    }
    out.push_str(&xml::escape(&buffer));
}

/// The cells as an OpenDocument text package.
#[cfg_attr(not(test), allow(dead_code))]
pub fn write(cells: &[Cell]) -> Vec<u8> {
    write_with(cells, &Properties::default())
}

/// The cells as an OpenDocument text package carrying the document properties.
pub fn write_with(cells: &[Cell], props: &Properties) -> Vec<u8> {
    // paragraphs: their cells, their properties
    let mut paragraphs: Vec<(&[Cell], Style)> = Vec::new();
    let mut start = 0;
    loop {
        let end = cells[start..].iter().position(|c| c.ch == '\n').map_or(cells.len(), |i| start + i);
        let style = cells.get(end).or_else(|| cells.get(start)).map(|c| c.style).unwrap_or_default();
        paragraphs.push((&cells[start..end], style));
        if end >= cells.len() {
            break;
        }
        start = end + 1;
    }
    // character styles in order of first use, paragraph styles for paragraphs that differ from the default ones
    let mut text_styles: Vec<Style> = Vec::new();
    let mut para_styles: Vec<Style> = Vec::new();
    let mut body = String::new();
    let mut open_list: Option<ListKind> = None;
    for (para, style) in &paragraphs {
        let want = (style.list != ListKind::None).then_some(style.list);
        if want != open_list {
            if open_list.is_some() {
                body.push_str("</text:list>");
            }
            match want {
                Some(ListKind::Bullet) => body.push_str("<text:list text:style-name=\"L1\">"),
                Some(_) => body.push_str("<text:list text:style-name=\"L2\">"),
                None => {}
            }
            open_list = want;
        }
        if want.is_some() {
            body.push_str("<text:list-item>");
        }
        let key = Style {
            align: style.align,
            spacing: style.spacing,
            space_before: style.space_before,
            space_after: style.space_after,
            rtl: style.rtl,
            ..Style::default()
        };
        let attr = if key == Style::default() {
            String::new()
        } else {
            let at = para_styles.iter().position(|k| *k == key).unwrap_or_else(|| {
                para_styles.push(key);
                para_styles.len() - 1
            });
            format!(" text:style-name=\"P{}\"", at + 1)
        };
        if para.is_empty() {
            body.push_str(&format!("<text:p{attr}/>"));
        } else {
            body.push_str(&format!("<text:p{attr}>"));
            let mut prev_space = true;
            let mut i = 0;
            while i < para.len() {
                let mut j = i + 1;
                while j < para.len() && para[i].style.same_run(&para[j].style) {
                    j += 1;
                }
                let run = &para[i].style;
                let plain = text_style_attrs(run).is_empty();
                let anchor = links::url(run.link);
                if let Some(url) = &anchor {
                    body.push_str(&format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\">", xml::escape(url)));
                }
                if plain {
                    run_text(&para[i..j], &mut prev_space, &mut body);
                } else {
                    let at = text_styles.iter().position(|s| s.same_run(run)).unwrap_or_else(|| {
                        text_styles.push(*run);
                        text_styles.len() - 1
                    });
                    body.push_str(&format!("<text:span text:style-name=\"T{}\">", at + 1));
                    run_text(&para[i..j], &mut prev_space, &mut body);
                    body.push_str("</text:span>");
                }
                if anchor.is_some() {
                    body.push_str("</text:a>");
                }
                i = j;
            }
            body.push_str("</text:p>");
        }
        if want.is_some() {
            body.push_str("</text:list-item>");
        }
    }
    if open_list.is_some() {
        body.push_str("</text:list>");
    }
    let mut styles = String::new();
    for (i, style) in text_styles.iter().enumerate() {
        styles += &format!("<style:style style:name=\"T{}\" style:family=\"text\"><style:text-properties{}/></style:style>", i + 1, text_style_attrs(style));
    }
    for (i, key) in para_styles.iter().enumerate() {
        let mut attrs = String::new();
        let align = match key.align {
            Align::Left => "start",
            Align::Center => "center",
            Align::Right => "end",
            Align::Justified => "justify",
        };
        attrs += &format!(" fo:text-align=\"{align}\"");
        if key.spacing > 0 {
            let multiple = SPACINGS.get(key.spacing as usize).copied().unwrap_or(1.0);
            attrs += &format!(" fo:line-height=\"{}%\"", (multiple * 100.0).round() as i32);
        }
        if key.space_before > 0 {
            attrs += &format!(" fo:margin-top=\"{}pt\"", key.space_before);
        }
        if key.space_after > 0 {
            attrs += &format!(" fo:margin-bottom=\"{}pt\"", key.space_after);
        }
        if key.rtl {
            attrs += " style:writing-mode=\"rl-tb\"";
        }
        styles += &format!("<style:style style:name=\"P{}\" style:family=\"paragraph\"><style:paragraph-properties{attrs}/></style:style>", i + 1);
    }
    styles += "<text:list-style style:name=\"L1\"><text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"\u{2022}\"/></text:list-style>";
    styles += "<text:list-style style:name=\"L2\"><text:list-level-style-number text:level=\"1\" style:num-format=\"1\"/></text:list-style>";
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
xmlns:xlink=\"http://www.w3.org/1999/xlink\" office:version=\"1.2\">\
<office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"
    );
    let has_meta = (0..4).any(|i| !props.get(i).is_empty());
    let meta_entry = if has_meta { "<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>" } else { "" };
    let manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\">\
<manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{MIMETYPE}\"/>\
<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>{meta_entry}</manifest:manifest>"
    );
    let meta = meta_part(props);
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("mimetype", MIMETYPE.as_bytes()),
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("content.xml", content.as_bytes()),
    ];
    if has_meta {
        parts.push(("meta.xml", meta.as_bytes()));
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

    fn styled(text: &str, style: Style) -> Vec<Cell> {
        text.chars().map(|ch| cell(ch, style)).collect()
    }

    #[test]
    fn formatting_alignment_spacing_lists_and_spaces_round_trip() {
        let bold = Style { bold: true, size: 18.0, color: 5, font: 1, ..Style::default() };
        let fancy = Style { italic: true, underline: true, strike: true, highlight: 3, script: -1, ..Style::default() };
        let centred = Style { align: Align::Center, spacing: 2, ..Style::default() };
        let listed = Style { list: ListKind::Numbered, ..Style::default() };
        let bullet = Style { list: ListKind::Bullet, ..Style::default() };
        let mut cells = Vec::new();
        cells.extend(styled("a", bold));
        cells.extend(styled("&<", fancy));
        cells.extend(styled("  x ", Style::default()));
        cells.push(cell('\n', Style::default()));
        cells.extend(styled("c", centred));
        cells.push(cell('\t', centred));
        cells.extend(styled("d", centred));
        cells.push(cell('\n', centred));
        cells.extend(styled("1", listed));
        cells.push(cell('\n', listed));
        cells.extend(styled("2", listed));
        cells.push(cell('\n', listed));
        cells.extend(styled("b", bullet));
        let package = write(&cells);
        assert!(is_odt(&package));
        assert_eq!(read(&package), Some(cells));
    }

    #[test]
    fn paragraph_space_and_writing_direction_round_trip() {
        let spaced = Style { space_before: 12, space_after: 6, spacing: 1, rtl: true, ..Style::default() };
        let mut cells = styled("a", spaced);
        cells.push(cell('\n', spaced));
        cells.push(cell('b', Style::default()));
        let package = write(&cells);
        let back = read(&package).unwrap();
        assert_eq!(back, cells);
        assert!(back[0].style.rtl && back[0].style.space_before == 12 && back[0].style.space_after == 6);
        assert!(!back[2].style.rtl && back[2].style.space_before == 0, "a paragraph without the properties has none");
    }

    #[test]
    fn a_line_break_is_a_break_inside_the_paragraph() {
        let mut cells = styled("one", Style::default());
        cells.push(cell(crate::text::LINE_BREAK, Style::default()));
        cells.extend(styled("two", Style::default()));
        cells.push(cell('\n', Style::default()));
        cells.extend(styled("three", Style::default()));
        let back = read(&write(&cells)).unwrap();
        assert_eq!(back, cells);
        assert_eq!(text_of(&back).matches('\n').count(), 1, "only the paragraph break is a newline");
    }

    #[test]
    fn links_are_anchors_around_spans_and_round_trip() {
        let id = links::intern("https://example.com/a?x=1&y=2");
        let other = links::intern("mailto:ada@example.com");
        let bold = Style { bold: true, link: id, ..Style::default() };
        let mut cells = styled("go ", Style::default());
        cells.extend(styled("there", bold));
        cells.extend(styled(" or ", Style::default()));
        cells.extend(styled("mail", Style { link: other, ..Style::default() }));
        let package = write(&cells);
        let content = String::from_utf8(zipdoc::read_entry(&package, "content.xml").unwrap()).unwrap();
        assert!(content.contains("xlink:href=\"https://example.com/a?x=1&amp;y=2\""), "{content}");
        assert_eq!(read(&package), Some(cells));
    }

    #[test]
    fn the_document_properties_round_trip_and_an_unlabelled_package_has_none() {
        let mut props = Properties::default();
        props.title = "Plan <draft>".into();
        props.author = "Ada".into();
        props.subject = "Q3".into();
        props.keywords = "red, blue".into();
        let package = write_with(&[], &props);
        assert_eq!(read_properties(&package), props);
        assert_eq!(read(&package), Some(Vec::new()));
        assert!(read_properties(&write(&[])).is_empty());
        assert!(!zipdoc::names(&write(&[])).iter().any(|n| n == "meta.xml"));
    }

    #[test]
    fn an_empty_document_and_empty_paragraphs_survive() {
        assert_eq!(read(&write(&[])), Some(Vec::new()));
        let cells = vec![cell('a', Style::default()), cell('\n', Style::default()), cell('\n', Style::default()), cell('b', Style::default())];
        assert_eq!(text_of(&read(&write(&cells)).unwrap()), "a\n\nb");
    }

    #[test]
    fn a_foreign_document_resolves_named_styles_through_their_parents() {
        let styles = "<office:document-styles xmlns:style=\"s\"><office:styles>\
<style:style style:name=\"Standard\" style:family=\"paragraph\"><style:text-properties fo:font-size=\"14pt\" fo:font-family=\"'Liberation Serif'\"/></style:style>\
<style:style style:name=\"Quote\" style:family=\"paragraph\" style:parent-style-name=\"Standard\"><style:paragraph-properties fo:text-align=\"end\" fo:line-height=\"150%\"/>\
<style:text-properties fo:font-style=\"italic\"/></style:style></office:styles></office:document-styles>";
        let content = "<office:document-content><office:automatic-styles>\
<style:style style:name=\"T1\" style:family=\"text\"><style:text-properties fo:font-weight=\"bold\" fo:color=\"#ff0000\" style:text-position=\"33% 100%\"/></style:style>\
<text:list-style style:name=\"L9\"><text:list-level-style-number text:level=\"1\"/></text:list-style></office:automatic-styles>\
<office:body><office:text>\
<text:h text:outline-level=\"1\">Head</text:h>\
<text:p text:style-name=\"Quote\">  quoted\n   text <text:span text:style-name=\"T1\">bold</text:span><text:s text:c=\"2\"/>end<text:tab/>t<text:line-break/>l<text:note><text:note-body><text:p>footnote</text:p></text:note-body></text:note></text:p>\
<text:list text:style-name=\"L9\"><text:list-item><text:p>item</text:p></text:list-item></text:list>\
</office:text></office:body></office:document-content>";
        let package = zipdoc::write(&[
            ("mimetype", MIMETYPE.as_bytes()),
            ("styles.xml", styles.as_bytes()),
            ("content.xml", content.as_bytes()),
        ]);
        assert!(is_odt(&package));
        let cells = read(&package).unwrap();
        assert_eq!(text_of(&cells), "Head\nquoted text bold  end\tt\u{2028}l\nitem");
        let head = &cells[0];
        assert!(head.style.bold && head.style.size == 20.0);
        let quoted = cells.iter().find(|c| c.ch == 'q').unwrap();
        assert_eq!(quoted.style.size, 14.0, "inherited from the parent style");
        assert!(quoted.style.italic);
        assert_eq!(quoted.style.align, Align::Right);
        assert_eq!(quoted.style.spacing, 2);
        assert_eq!(quoted.style.font, 1);
        let bold = cells.iter().find(|c| c.ch == 'b').unwrap();
        assert!(bold.style.bold && bold.style.color == 1 && bold.style.script == 1 && bold.style.italic, "a span adds to its paragraph");
        let item = cells.iter().rev().find(|c| c.ch == 'i').unwrap();
        assert_eq!(item.style.list, ListKind::Numbered);
        assert!(!text_of(&cells).contains("footnote"), "notes are skipped");
    }

    #[test]
    fn a_package_that_is_not_text_is_not_a_document() {
        let sheet = zipdoc::write(&[("mimetype", &b"application/vnd.oasis.opendocument.spreadsheet"[..]), ("content.xml", &b"<a/>"[..])]);
        assert!(!is_odt(&sheet));
        assert!(!is_odt(b"plain"));
        assert_eq!(read(&zipdoc::write(&[("a", &b"x"[..])])), None);
    }
}
