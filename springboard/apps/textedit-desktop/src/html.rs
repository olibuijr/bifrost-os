//! HTML documents: the writer emits one paragraph element per paragraph with inline styles (font, size, colour,
//! alignment, line spacing, lists), and the reader accepts that and the common forms other writers produce (headings,
//! `b`/`strong`, `i`/`em`, `u`, `span` and `font` styles, `br`, lists, entities). Text outside the elements it does not
//! know is kept; `head`, `style`, `script` and comments are dropped.
//!
//! The SDK's attributed-string HTML codec (`bifrost_app::attributed`) keeps only named size classes and colours and no
//! alignment or lists, which would lose a document's formatting on the first save, so the editor's codec works on the
//! editor's own cells and carries all of it.

use crate::links;
use crate::properties::Properties;
use crate::rtf::{nearest_color, nearest_spacing};
use crate::text::{Align, Cell, ListKind, Style, COLORS, DEFAULT_TAB, FAMILIES, MAX_PARAGRAPH_SPACE, MAX_SIZE, MAX_TAB, MIN_SIZE, MIN_TAB, SPACINGS};

const BOLD_WEIGHT_THRESHOLD: u32 = 400 + 200;

/// Whether `bytes` look like an HTML file.
pub fn is_html(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    let head = head.trim_start();
    head.starts_with("<!doctype html") || head.starts_with("<html") || (head.starts_with("<!--") && head.contains("<html"))
}

fn escape(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\t' => out.push_str("&#9;"),
        c => out.push(c),
    }
}

fn align_css(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justified => "justify",
    }
}

/// Writes the cells as an HTML document. `tab` is the distance between tab stops in points.
#[cfg_attr(not(test), allow(dead_code))]
pub fn write(cells: &[Cell], tab: f32) -> Vec<u8> {
    write_with(cells, tab, &Properties::default())
}

/// Writes the cells as an HTML document, with the document's properties in the head: the title element, and meta
/// elements for the author, description (the subject), keywords and copyright.
pub fn write_with(cells: &[Cell], tab: f32, props: &Properties) -> Vec<u8> {
    let mut out = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">");
    out += &format!("<meta name=\"tab-stop\" content=\"{tab}\"><title>");
    props.title.chars().for_each(|ch| escape(ch, &mut out));
    out += "</title>";
    for (name, value) in [("author", &props.author), ("description", &props.subject), ("keywords", &props.keywords), ("copyright", &props.copyright)] {
        if !value.is_empty() {
            out += &format!("<meta name=\"{name}\" content=\"");
            value.chars().for_each(|ch| escape(ch, &mut out));
            out += "\">";
        }
    }
    out += "<style>p{margin:0;min-height:1em;white-space:pre-wrap}ul,ol{margin:0}</style></head><body>\n";
    let mut open_list: Option<ListKind> = None;
    let mut start = 0;
    loop {
        let end = cells[start..].iter().position(|c| c.ch == '\n').map_or(cells.len(), |i| start + i);
        let para = &cells[start..end];
        // the paragraph's properties live on its first cell (an empty paragraph's, on its break, or the one before)
        let style = cells.get(start).or(cells.last()).map(|c| c.style).unwrap_or_default();
        let want = if style.list == ListKind::None { None } else { Some(style.list) };
        if open_list != want {
            match open_list {
                Some(ListKind::Bullet) => out += "</ul>\n",
                Some(ListKind::Numbered) => out += "</ol>\n",
                _ => {}
            }
            match want {
                Some(ListKind::Bullet) => out += "<ul>\n",
                Some(ListKind::Numbered) => out += "<ol>\n",
                _ => {}
            }
            open_list = want;
        }
        let tag = if want.is_some() { "li" } else { "p" };
        let mut css = format!("text-align:{};white-space:pre-wrap", align_css(style.align));
        if style.spacing > 0 {
            css += &format!(";line-height:{}", SPACINGS.get(style.spacing as usize).copied().unwrap_or(1.0));
        }
        if style.space_before > 0 {
            css += &format!(";margin-top:{}pt", style.space_before);
        }
        if style.space_after > 0 {
            css += &format!(";margin-bottom:{}pt", style.space_after);
        }
        out += &format!("<{tag} style=\"{css}\">");
        let mut i = 0;
        while i < para.len() {
            let mut j = i + 1;
            while j < para.len() && para[i].style.same_run(&para[j].style) {
                j += 1;
            }
            let s = para[i].style;
            let mut span = format!("font-family:{};font-size:{}pt", crate::view::family_name(s.font), s.size);
            if s.color > 0 {
                let [r, g, b] = COLORS[(s.color as usize).min(COLORS.len() - 1)].1;
                span += &format!(";color:#{r:02x}{g:02x}{b:02x}");
            }
            out += &format!("<span style=\"{span}\">");
            // a link is an anchor inside the span
            let anchor = links::url(s.link);
            if let Some(url) = &anchor {
                out += "<a href=\"";
                url.chars().for_each(|ch| escape(ch, &mut out));
                out += "\">";
            }
            let (b, it, u) = (s.bold, s.italic, s.underline);
            if s.strike {
                out += "<s>";
            }
            match s.script {
                1 => out += "<sup>",
                -1 => out += "<sub>",
                _ => {}
            }
            if b {
                out += "<b>";
            }
            if it {
                out += "<i>";
            }
            if u {
                out += "<u>";
            }
            for c in &para[i..j] {
                match c.ch {
                    crate::text::LINE_BREAK => out += "<br>",
                    crate::text::PAGE_BREAK => out += "<br style=\"page-break-after:always\">",
                    ch => escape(ch, &mut out),
                }
            }
            if u {
                out += "</u>";
            }
            if it {
                out += "</i>";
            }
            if b {
                out += "</b>";
            }
            match s.script {
                1 => out += "</sup>",
                -1 => out += "</sub>",
                _ => {}
            }
            if s.strike {
                out += "</s>";
            }
            if anchor.is_some() {
                out += "</a>";
            }
            out += "</span>";
            i = j;
        }
        out += &format!("</{tag}>\n");
        if end >= cells.len() {
            break;
        }
        start = end + 1;
    }
    match open_list {
        Some(ListKind::Bullet) => out += "</ul>\n",
        Some(ListKind::Numbered) => out += "</ol>\n",
        _ => {}
    }
    out += "</body></html>\n";
    out.into_bytes()
}

/// What the reader is inside, with the style to go back to when it closes.
struct Open {
    name: String,
    saved: Style,
    pre: bool,
    list: bool,
}

struct Reader {
    cells: Vec<Cell>,
    /// the character style, and the paragraph properties applied to the paragraph when it ends
    cur: Style,
    para: Style,
    stack: Vec<Open>,
    lists: Vec<ListKind>,
    pre: bool,
    /// the first cell of the paragraph being read
    para_start: usize,
    /// a line break just ended a paragraph and nothing has been read since
    broke: bool,
    /// the previous character was a collapsed space
    space: bool,
    tab: f32,
}

const SKIPPED: [&str; 4] = ["head", "style", "script", "title"];

/// Reads an HTML document into cells and the tab stop distance.
pub fn read(bytes: &[u8]) -> (Vec<Cell>, f32) {
    let text = String::from_utf8_lossy(bytes);
    let mut r = Reader {
        cells: Vec::new(),
        cur: Style::default(),
        para: Style::default(),
        stack: Vec::new(),
        lists: Vec::new(),
        pre: false,
        para_start: 0,
        broke: false,
        space: true,
        tab: DEFAULT_TAB,
    };
    r.run(&text);
    // the last paragraph's terminating break is the document's end, not a blank paragraph
    if r.cells.last().is_some_and(|c| c.ch == '\n') {
        r.cells.pop();
    }
    (r.cells, r.tab.clamp(MIN_TAB, MAX_TAB))
}

/// The document properties in an HTML head: the title element and the meta elements the writer uses (author,
/// description as the subject, keywords and copyright). Other pages' meta elements that match those names count too.
pub fn read_properties(bytes: &[u8]) -> Properties {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    let mut props = Properties::default();
    if let Some(start) = lower.find("<title") {
        if let Some(open_end) = lower[start..].find('>').map(|i| start + i + 1) {
            if let Some(close) = lower[open_end..].find("</title").map(|i| open_end + i) {
                props.title = crate::properties::clean(&decode(&text[open_end..close]));
            }
        }
    }
    let mut from = 0;
    while let Some(i) = lower[from..].find("<meta") {
        let at = from + i;
        let end = lower[at..].find('>').map_or(lower.len(), |e| at + e);
        let tag = &text[at + "<meta".len()..end];
        from = end.max(at + 1);
        let (Some(name), Some(content)) = (attribute(tag, "name"), attribute(tag, "content")) else { continue };
        match name.to_ascii_lowercase().as_str() {
            "author" => props.set_named("author", &content),
            "description" => props.set_named("subject", &content),
            "keywords" => props.set_named("keywords", &content),
            "copyright" => props.set_named("copyright", &content),
            _ => {}
        }
    }
    props
}

impl Reader {
    fn run(&mut self, text: &str) {
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix("<!--") {
                rest = after.find("-->").map_or("", |i| &after[i + 3..]);
            } else if let Some(after) = rest.strip_prefix('<') {
                let end = after.find('>').unwrap_or(after.len());
                let tag = &after[..end];
                rest = after.get(end + 1..).unwrap_or("");
                if tag.starts_with('!') || tag.starts_with('?') {
                    continue;
                }
                let closing = tag.starts_with('/');
                let body = tag.trim_start_matches('/').trim_end_matches('/');
                let name = body.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_ascii_lowercase();
                if closing {
                    self.close(&name);
                } else {
                    if SKIPPED.contains(&name.as_str()) {
                        if name == "head" {
                            // the head holds the tab setting and nothing to show
                            let close = rest.find("</head").unwrap_or(rest.len());
                            self.head(&rest[..close]);
                            rest = &rest[close..];
                        } else {
                            let needle = format!("</{name}");
                            let close = rest.to_ascii_lowercase().find(&needle).unwrap_or(rest.len());
                            rest = &rest[close..];
                        }
                        continue;
                    }
                    self.open(&name, body);
                }
            } else {
                let end = rest.find('<').unwrap_or(rest.len());
                self.text(&rest[..end]);
                rest = &rest[end..];
            }
        }
    }

    fn head(&mut self, head: &str) {
        let lower = head.to_ascii_lowercase();
        if let Some(at) = lower.find("name=\"tab-stop\"") {
            if let Some(c) = lower[at..].find("content=\"") {
                let value = &lower[at + c + 9..];
                if let Some(v) = value.split('"').next().and_then(|v| v.parse::<f32>().ok()) {
                    self.tab = v;
                }
            }
        }
    }

    fn text(&mut self, raw: &str) {
        let decoded = decode(raw);
        for ch in decoded.chars() {
            if !self.pre && ch.is_whitespace() && ch != '\u{a0}' {
                if self.space {
                    continue;
                }
                self.space = true;
                self.push(' ');
            } else {
                self.space = false;
                self.push(ch);
            }
        }
    }

    fn push(&mut self, ch: char) {
        self.broke = false;
        self.cells.push(Cell { ch, style: self.cur });
    }

    /// Ends the paragraph: the paragraph properties go on its cells and its break.
    fn end_paragraph(&mut self) {
        let para = self.para;
        self.cells.push(Cell { ch: '\n', style: self.cur });
        for c in &mut self.cells[self.para_start..] {
            c.style.align = para.align;
            c.style.spacing = para.spacing;
            c.style.list = para.list;
            c.style.space_before = para.space_before;
            c.style.space_after = para.space_after;
        }
        self.para_start = self.cells.len();
        self.space = true;
        self.broke = true;
    }

    /// Ends the paragraph in progress when it has content (a block starting or ending around loose text).
    fn flush(&mut self) {
        if self.cells.len() > self.para_start {
            self.end_paragraph();
        }
    }

    fn open(&mut self, name: &str, attrs: &str) {
        let saved = self.cur;
        let pre_before = self.pre;
        let (mut pre, mut list) = (self.pre, false);
        let style_attr = attribute(attrs, "style").unwrap_or_default().to_ascii_lowercase();
        match name {
            // a break inside a paragraph is a line break; a page break is the form the writer uses for one
            "br" => {
                let page = style_attr.contains("page-break");
                self.push(if page { crate::text::PAGE_BREAK } else { crate::text::LINE_BREAK });
                self.space = false;
                return;
            }
            "a" => {
                if let Some(url) = attribute(attrs, "href").and_then(|href| links::normalize(&decode(&href))) {
                    self.cur.link = links::intern(&url);
                }
            }
            "b" | "strong" => self.cur.bold = true,
            "i" | "em" => self.cur.italic = true,
            "u" => self.cur.underline = true,
            "s" | "strike" | "del" => self.cur.strike = true,
            "sup" => self.cur.script = 1,
            "sub" => self.cur.script = -1,
            "ul" | "ol" => {
                self.flush();
                self.lists.push(if name == "ul" { ListKind::Bullet } else { ListKind::Numbered });
                list = true;
            }
            "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "blockquote" => {
                self.flush();
                // a new block starts with no break of its own, so an empty block still makes its paragraph
                self.broke = false;
                self.para = Style::default();
                if name == "li" {
                    self.para.list = self.lists.last().copied().unwrap_or(ListKind::Bullet);
                }
                match name {
                    "h1" => self.heading(24.0),
                    "h2" => self.heading(18.0),
                    "h3" => self.heading(14.0),
                    "h4" | "h5" | "h6" => self.heading(12.0),
                    "pre" => {
                        self.cur.font = 2;
                        pre = true;
                    }
                    _ => {}
                }
                if let Some(a) = attribute(attrs, "align") {
                    self.para.align = align_of(&a);
                }
                for (k, v) in declarations(&style_attr) {
                    match k.as_str() {
                        "text-align" => self.para.align = align_of(&v),
                        "line-height" => {
                            if let Ok(m) = v.trim_end_matches('%').parse::<f32>() {
                                let m = if v.ends_with('%') { m / 100.0 } else { m };
                                self.para.spacing = nearest_spacing(m);
                            }
                        }
                        "white-space" if v.starts_with("pre") => pre = true,
                        "margin-top" => {
                            if let Some(points) = space_of(&v) {
                                self.para.space_before = points;
                            }
                        }
                        "margin-bottom" => {
                            if let Some(points) = space_of(&v) {
                                self.para.space_after = points;
                            }
                        }
                        _ => {}
                    }
                }
            }
            "span" | "font" => {
                if name == "font" {
                    if let Some(c) = attribute(attrs, "color") {
                        self.cur.color = color_of(&c);
                    }
                    if let Some(f) = attribute(attrs, "face") {
                        self.cur.font = family_of(&f);
                    }
                }
            }
            _ => {}
        }
        if matches!(name, "span" | "font" | "b" | "strong" | "i" | "em" | "u" | "p" | "div" | "li" | "h1" | "h2" | "h3") {
            for (k, v) in declarations(&style_attr) {
                match k.as_str() {
                    "font-size" => {
                        if let Some(pt) = size_of(&v) {
                            self.cur.size = pt;
                        }
                    }
                    "font-family" => self.cur.font = family_of(&v),
                    "color" => self.cur.color = color_of(&v),
                    "font-weight" => self.cur.bold = v == "bold" || v.parse::<u32>().is_ok_and(|w| w >= BOLD_WEIGHT_THRESHOLD),
                    "font-style" => self.cur.italic = v == "italic" || v == "oblique",
                    "text-decoration" | "text-decoration-line" => self.cur.underline = v.contains("underline"),
                    _ => {}
                }
            }
        }
        self.pre = pre;
        self.stack.push(Open { name: name.to_string(), saved, pre: pre_before, list });
    }

    fn heading(&mut self, size: f32) {
        self.cur.bold = true;
        self.cur.size = size;
    }

    fn close(&mut self, name: &str) {
        let Some(at) = self.stack.iter().rposition(|o| o.name == name) else { return };
        let open = self.stack.remove(at);
        self.stack.truncate(at);
        if matches!(name, "p" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "blockquote") {
            // a paragraph ended by a break has no further content: the break was its end
            if !(self.broke && self.cells.len() == self.para_start) {
                self.end_paragraph();
            }
        } else if name == "div" {
            self.flush();
        }
        if open.list {
            self.lists.pop();
        }
        self.cur = open.saved;
        self.pre = open.pre;
    }
}

/// `value` of the attribute `name` in a tag's text, with or without quotes.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let word_start = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let Some(after_eq) = tag[from..].trim_start().strip_prefix('=') else { continue };
        if !word_start {
            continue;
        }
        let after_eq = after_eq.trim_start();
        let (quote, body) = match after_eq.chars().next()? {
            q @ ('"' | '\'') => (Some(q), &after_eq[1..]),
            _ => (None, after_eq),
        };
        let end = match quote {
            Some(q) => body.find(q).unwrap_or(body.len()),
            None => body.find(char::is_whitespace).unwrap_or(body.len()),
        };
        return Some(decode(&body[..end]));
    }
    None
}

/// `key: value` pairs of an inline style, lower-cased and trimmed.
fn declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn align_of(v: &str) -> Align {
    match v.trim() {
        "center" => Align::Center,
        "right" => Align::Right,
        "justify" | "justified" => Align::Justified,
        _ => Align::Left,
    }
}

fn family_of(v: &str) -> u8 {
    let lower = v.to_ascii_lowercase();
    let first = lower.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    // an installed family by its exact name, else a base family the name starts with
    if let Some(i) = crate::text::family_index(&first) {
        return i;
    }
    if let Some(i) = FAMILIES.iter().position(|f| first.starts_with(&f.to_ascii_lowercase())) {
        return i as u8;
    }
    if first.contains("mono") || first.contains("courier") || first.contains("consolas") {
        2
    } else if first.contains("serif") && !first.contains("sans") || first.contains("georgia") || first.contains("times") {
        1
    } else {
        0
    }
}

/// A point size from `12pt`, `16px` (0.75 pt each) or a bare number of points.
fn size_of(v: &str) -> Option<f32> {
    let v = v.trim();
    let pt = if let Some(n) = v.strip_suffix("pt") {
        n.trim().parse::<f32>().ok()?
    } else if let Some(n) = v.strip_suffix("px") {
        n.trim().parse::<f32>().ok()? * 0.75
    } else {
        return None;
    };
    Some(pt.clamp(MIN_SIZE, MAX_SIZE))
}

/// A paragraph's space from a margin value: points, or pixels at 0.75 point each. Negative and unreadable values are
/// no space.
fn space_of(v: &str) -> Option<u8> {
    let v = v.trim();
    let points = if let Some(n) = v.strip_suffix("pt") {
        n.trim().parse::<f32>().ok()?
    } else if let Some(n) = v.strip_suffix("px") {
        n.trim().parse::<f32>().ok()? * 0.75
    } else {
        return None;
    };
    Some(points.round().clamp(0.0, MAX_PARAGRAPH_SPACE as f32) as u8)
}

/// The palette colour for `#rgb`, `#rrggbb` or `rgb(r, g, b)`; the default colour for black, white and anything else.
fn color_of(v: &str) -> u8 {
    let v = v.trim();
    let rgb: Option<[i32; 3]> = if let Some(hex) = v.strip_prefix('#').filter(|h| h.is_ascii()) {
        match hex.len() {
            6 => (0..3).map(|k| i32::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()).collect::<Option<Vec<_>>>().map(|c| [c[0], c[1], c[2]]),
            3 => (0..3).map(|k| i32::from_str_radix(&hex[k..k + 1], 16).ok().map(|d| d * 17)).collect::<Option<Vec<_>>>().map(|c| [c[0], c[1], c[2]]),
            _ => None,
        }
    } else if let Some(args) = v.strip_prefix("rgb(").and_then(|a| a.strip_suffix(')')) {
        let c: Vec<i32> = args.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        (c.len() == 3).then(|| [c[0], c[1], c[2]])
    } else {
        None
    };
    match rgb {
        // black, white and near greys are the text colour, so pasted body text does not turn gray
        Some(c) if c.iter().max().unwrap() - c.iter().min().unwrap() < 24 && (c[0] < 64 || c[0] > 230) => 0,
        Some(c) => nearest_color(c),
        None => 0,
    }
}

/// Decodes the entities of a text: the named ones HTML writers use and numeric ones.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|&n| n <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
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
    fn formatting_alignment_lists_and_tabs_round_trip() {
        let bold = Style { bold: true, size: 18.0, color: 5, font: 1, ..Style::default() };
        let centred = Style { align: Align::Center, spacing: 2, underline: true, ..Style::default() };
        let centred_break = Style { align: Align::Center, spacing: 2, ..Style::default() };
        let listed = Style { list: ListKind::Bullet, ..Style::default() };
        let cells = vec![
            cell('a', bold),
            cell('&', bold),
            cell('\n', Style::default()),
            cell('c', centred),
            cell('\t', centred),
            cell('\n', centred_break),
            cell('x', listed),
            cell('\n', listed),
            cell('y', Style::default()),
        ];
        let (back, tab) = read(&write(&cells, 48.0));
        assert_eq!(back, cells);
        assert_eq!(tab, 48.0);
    }

    #[test]
    fn strikethrough_and_baseline_shift_round_trip() {
        let struck = Style { strike: true, ..Style::default() };
        let raised = Style { script: 1, ..Style::default() };
        let lowered = Style { script: -1, ..Style::default() };
        let cells = vec![cell('a', struck), cell('b', raised), cell('c', lowered), cell('d', Style::default())];
        let (back, _) = read(&write(&cells, 36.0));
        assert_eq!(back, cells);
    }

    #[test]
    fn line_and_page_breaks_are_breaks_inside_a_paragraph() {
        let cells = vec![
            cell('a', Style::default()),
            cell(crate::text::LINE_BREAK, Style::default()),
            cell('b', Style::default()),
            cell(crate::text::PAGE_BREAK, Style::default()),
            cell('c', Style::default()),
            cell('\n', Style::default()),
        ];
        let html = String::from_utf8(write(&cells, 36.0)).unwrap();
        assert!(html.contains("<br>") && html.contains("page-break-after"), "{html}");
        assert_eq!(read(html.as_bytes()).0, cells, "a break is not a new paragraph");
    }

    #[test]
    fn a_trailing_break_and_an_empty_document_survive() {
        let cells = vec![cell('a', Style::default()), cell('\n', Style::default())];
        assert_eq!(read(&write(&cells, 36.0)).0, cells);
        assert!(read(&write(&[], 36.0)).0.is_empty());
    }

    #[test]
    fn reads_common_markup_from_other_writers() {
        let (cells, _) = read(b"<html><head><title>t</title><style>p{}</style></head><body><h1>Title</h1><p>one <b>two</b>&nbsp;&amp; <i>three</i><br>four</p><ul><li>a</li><li>b</li></ul></body></html>");
        assert_eq!(text_of(&cells), "Title\none two\u{a0}& three\u{2028}four\na\nb");
        assert!(cells[0].style.bold && cells[0].style.size == 24.0);
        let a = cells.iter().rposition(|c| c.ch == 'a').unwrap();
        assert_eq!(cells[a].style.list, ListKind::Bullet);
        assert_eq!(cells[0].style.list, ListKind::None);
    }

    #[test]
    fn links_are_anchors_and_round_trip() {
        let id = links::intern("https://example.com/a?x=1&y=2");
        let linked = Style { link: id, ..Style::default() };
        let mut cells: Vec<Cell> = "go ".chars().map(|ch| cell(ch, Style::default())).collect();
        cells.extend("there".chars().map(|ch| cell(ch, linked)));
        cells.extend(" now".chars().map(|ch| cell(ch, Style::default())));
        let html = String::from_utf8(write(&cells, 36.0)).unwrap();
        assert!(html.contains("<a href=\"https://example.com/a?x=1&amp;y=2\">there</a>"), "{html}");
        let (back, _) = read(html.as_bytes());
        assert_eq!(back, cells);
    }

    #[test]
    fn foreign_anchors_are_read_and_unsafe_ones_are_plain_text() {
        let (cells, _) = read(b"<p>see <a href=\"https://example.org/x\">the <b>docs</b></a> and <a href=\"javascript:alert(1)\">this</a> or <a name=\"top\">that</a></p>");
        assert_eq!(text_of(&cells), "see the docs and this or that");
        let linked: String = cells.iter().filter(|c| c.style.link != 0).map(|c| c.ch).collect();
        assert_eq!(linked, "the docs", "an unsafe address and an anchor without one are not links");
        let first = cells.iter().find(|c| c.style.link != 0).unwrap();
        assert_eq!(links::url(first.style.link).as_deref(), Some("https://example.org/x"));
        assert!(cells.iter().filter(|c| c.style.bold).all(|c| c.style.link != 0), "the bold text inside the anchor keeps the link");
    }

    #[test]
    fn the_document_properties_are_head_elements_and_read_back() {
        let props = Properties { title: "Plan & <notes>".into(), author: "Ada".into(), subject: "Q3".into(), keywords: "a, b".into(), copyright: "2026".into() };
        let html = write_with(&[cell('x', Style::default())], 36.0, &props);
        assert!(String::from_utf8_lossy(&html).contains("<title>Plan &amp; &lt;notes&gt;</title>"));
        assert_eq!(read_properties(&html), props);
        assert!(read_properties(b"<p>no head</p>").is_empty());
    }

    #[test]
    fn paragraph_space_round_trips_as_margins() {
        let spaced = Style { space_before: 10, space_after: 4, ..Style::default() };
        let cells = vec![cell('a', spaced), cell('\n', spaced), cell('b', Style::default())];
        let html = String::from_utf8(write(&cells, 36.0)).unwrap();
        assert!(html.contains("margin-top:10pt") && html.contains("margin-bottom:4pt"), "{html}");
        assert_eq!(read(html.as_bytes()).0, cells);
        let (px, _) = read(b"<p style=\"margin-top:20px\">x</p>");
        assert_eq!(px[0].style.space_before, 15, "a pixel is three quarters of a point");
    }

    #[test]
    fn whitespace_collapses_outside_preformatted_text() {
        let (cells, _) = read(b"<p>a \n   b</p><pre>c  d</pre>");
        assert_eq!(text_of(&cells), "a b\nc  d");
    }

    #[test]
    fn styles_map_to_the_palette_and_families() {
        let (cells, _) = read(b"<p style=\"text-align:right\"><span style=\"font-size:24px;color:#ff3b30;font-family:Courier New\">x</span></p>");
        let s = cells[0].style;
        assert_eq!(s.size, 18.0);
        assert_eq!(s.color, 1);
        assert_eq!(s.font, 2);
        assert_eq!(s.align, Align::Right);
    }

    #[test]
    fn entities_decode_and_unknown_ones_stay() {
        assert_eq!(decode("a &lt;b&gt; &#65;&#x42; &bogus; &"), "a <b> AB &bogus; &");
    }

    #[test]
    fn recognises_html_files() {
        assert!(is_html(b"<!DOCTYPE html><html>"));
        assert!(is_html(b"  <html><body>"));
        assert!(!is_html(b"plain text <html> later"));
    }
}
