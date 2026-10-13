//! HTML documents: the writer emits one paragraph element per paragraph with inline styles (font, size, colour,
//! alignment, line spacing, lists), and the reader accepts that and the common forms other writers produce (headings,
//! `b`/`strong`, `i`/`em`, `u`, `span` and `font` styles, `br`, lists, entities). Text outside the elements it does not
//! know is kept; `head`, `style`, `script` and comments are dropped.
//!
//! The SDK's attributed-string HTML codec (`bifrost_app::attributed`) keeps only named size classes and colours and no
//! alignment or lists, which would lose a document's formatting on the first save, so the editor's codec works on the
//! editor's own cells and carries all of it.

use crate::rtf::{nearest_color, nearest_spacing};
use crate::text::{Align, Cell, ListKind, Style, COLORS, DEFAULT_TAB, FAMILIES, MAX_SIZE, MAX_TAB, MIN_SIZE, MIN_TAB, SPACINGS};

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
pub fn write(cells: &[Cell], tab: f32) -> Vec<u8> {
    let mut out = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">");
    out += &format!("<meta name=\"tab-stop\" content=\"{tab}\"><title></title>");
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
        out += &format!("<{tag} style=\"{css}\">");
        let mut i = 0;
        while i < para.len() {
            let mut j = i + 1;
            while j < para.len() && para[i].style.same_run(&para[j].style) {
                j += 1;
            }
            let s = para[i].style;
            let mut span = format!("font-family:{};font-size:{}pt", FAMILIES[(s.font as usize).min(FAMILIES.len() - 1)], s.size);
            if s.color > 0 {
                let [r, g, b] = COLORS[(s.color as usize).min(COLORS.len() - 1)].1;
                span += &format!(";color:#{r:02x}{g:02x}{b:02x}");
            }
            out += &format!("<span style=\"{span}\">");
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
            "br" => {
                self.end_paragraph();
                return;
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
                    "font-weight" => self.cur.bold = v == "bold" || v.parse::<u32>().is_ok_and(|w| w >= 600),
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
    fn a_trailing_break_and_an_empty_document_survive() {
        let cells = vec![cell('a', Style::default()), cell('\n', Style::default())];
        assert_eq!(read(&write(&cells, 36.0)).0, cells);
        assert!(read(&write(&[], 36.0)).0.is_empty());
    }

    #[test]
    fn reads_common_markup_from_other_writers() {
        let (cells, _) = read(b"<html><head><title>t</title><style>p{}</style></head><body><h1>Title</h1><p>one <b>two</b>&nbsp;&amp; <i>three</i><br>four</p><ul><li>a</li><li>b</li></ul></body></html>");
        assert_eq!(text_of(&cells), "Title\none two\u{a0}& three\nfour\na\nb");
        assert!(cells[0].style.bold && cells[0].style.size == 24.0);
        let a = cells.iter().rposition(|c| c.ch == 'a').unwrap();
        assert_eq!(cells[a].style.list, ListKind::Bullet);
        assert_eq!(cells[0].style.list, ListKind::None);
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
