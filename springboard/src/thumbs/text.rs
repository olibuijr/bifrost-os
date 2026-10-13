//! Text thumbnails: the first page of a plain text file, a rich text file, a web page (text only) or a clipping, laid
//! out with the typesetter (`bifrost_ui::typeset`) and drawn on a paper page.
//!
//! The page is 200 x 260 points; the pixel size follows the requested edge. Only the head of the file is read
//! ([`HEAD_BYTES`]) and only the first [`MAX_CHARS`] characters are laid out, so a huge log costs the same as a note.

use super::canvas::{make_ui, Canvas, INK, MUTED, PAPER, RED};
use bifrost_render::ui::Weight;
use bifrost_ui::textmetrics::{FnSource, FontMetrics, FontSpec};
use bifrost_ui::thumb_service::{SourceFile, ThumbArt, ThumbProvider};
use bifrost_ui::typeset::{FramePath, Framesetter, RunStyle, TextPainter, Typesetter, TypesetterOptions};
use bifrost_ui::Rect;
use std::io::Read;
use std::rc::Rc;

/// Bytes read from the head of a file.
pub const HEAD_BYTES: usize = 16 * 1024;
/// Characters laid out.
pub const MAX_CHARS: usize = 1500;
/// Page size in points.
pub const PAGE_W: f32 = 200.0;
pub const PAGE_H: f32 = 260.0;
const MARGIN: f32 = 14.0;
const BODY_PT: f32 = 8.5;
const TITLE_PT: f32 = 10.5;

/// How a text page looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    Plain,
    /// A clipping: a coloured tab across the top of the page.
    Clipping,
}

/// The first `max` bytes of the file at `path`.
pub fn read_head(path: &std::path::Path, max: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(max.min(1 << 16));
    std::fs::File::open(path).ok()?.take(max as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Text from bytes that are meant to be text: `None` for binary data (a NUL byte, or many replacement characters).
pub fn text_from_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    // a head cut in the middle of a character is still text
    let cut = match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => bytes.len(),
    };
    let s = String::from_utf8_lossy(&bytes[..cut]).into_owned();
    let bad = s.chars().filter(|&c| c == '\u{FFFD}').count();
    if bad * 50 > s.chars().count().max(1) {
        return None;
    }
    Some(s)
}

/// Text ready to lay out: tabs become spaces, carriage returns and other control characters go, runs of blank lines
/// shrink to one, and the result is at most [`MAX_CHARS`] characters.
pub fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        let line: String = line.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
        if out.chars().count() >= MAX_CHARS {
            break;
        }
    }
    out.chars().take(MAX_CHARS).collect::<String>().trim_end().to_string()
}

fn weight_of(w: u16) -> Weight {
    if w >= 600 {
        Weight::Bold
    } else {
        Weight::Regular
    }
}

struct PagePainter<'a> {
    canvas: &'a mut Canvas,
}

impl TextPainter for PagePainter<'_> {
    fn default_color(&self) -> u32 {
        INK
    }

    fn fill_rect(&mut self, r: Rect, color: u32) {
        self.canvas.rect(r.x, r.y, r.w, r.h, color);
    }

    fn draw_text(&mut self, text: &str, x: f32, baseline: f32, style: &RunStyle) {
        let font = style.effective_font();
        let ascent = self.canvas.ui.font.line_metrics(self.canvas.ui.pt(font.size)).ascent / self.canvas.s;
        let color = style.foreground.unwrap_or(INK);
        // `Canvas::text` places the top of the text box; the baseline sits `ascent` below it
        self.canvas.text(text, font.size, weight_of(font.weight), color, x, baseline - ascent, PAGE_W);
    }

    fn draw_rule(&mut self, x0: f32, x1: f32, y: f32, thickness: f32, color: u32) {
        self.canvas.rect(x0, y - thickness / 2.0, x1 - x0, thickness, color);
    }
}

/// A page with `title` (bold, optional) over `body`, `px` pixels tall. `None` without the UI font or with no text.
pub fn render_page(title: Option<&str>, body: &str, px: u32, look: Look) -> Option<ThumbArt> {
    let body = tidy(body);
    let title = title.map(|t| tidy(t).replace('\n', " ")).filter(|t| !t.is_empty());
    if body.is_empty() && title.is_none() {
        return None;
    }
    let h = px as usize;
    let w = ((px as f32 * PAGE_W / PAGE_H).round() as usize).max(1);
    let mut canvas = Canvas::new(w, h, PAGE_W)?;
    canvas.page(6.0);
    if look == Look::Clipping {
        canvas.rect(0.0, 0.0, PAGE_W, 14.0, RED);
    }
    let top = if look == Look::Clipping { 22.0 } else { MARGIN };

    let text = match &title {
        Some(t) => format!("{t}\n{body}"),
        None => body.clone(),
    };
    let t16 = title.as_ref().map_or(0, |t| t.encode_utf16().count());
    let all16 = text.encode_utf16().count();
    let style = |size: f32, weight: u16| RunStyle { font: FontSpec { family: None, size, weight, italic: false }, foreground: Some(INK), ..Default::default() };
    let mut spans = vec![(0..all16, style(BODY_PT, 400))];
    if title.is_some() {
        spans.push((0..t16, style(TITLE_PT, 700)));
    }

    // a second UI measures: the typesetter owns its glyph source
    let mui = Rc::new(make_ui(canvas.s)?);
    let (m1, m2) = (mui.clone(), mui);
    let s = canvas.s.max(f32::EPSILON);
    let source = FnSource {
        measure: Box::new(move |f, t| m1.measure(t, f.size, weight_of(f.weight)) / s),
        metrics: Box::new(move |f| {
            let lm = m2.font.line_metrics(m2.pt(f.size));
            FontMetrics { ascent: lm.ascent / s, descent: lm.descent / s, leading: lm.leading / s, underline_position: lm.descent / s / 2.0, underline_thickness: f.size / 14.0 }
        }),
    };
    let ts = Typesetter::new(&text, &spans, &[], TypesetterOptions::default(), Rc::new(source));
    let frame = Framesetter::new(ts).create_frame(0..all16, FramePath::Rect(Rect { x: 0.0, y: 0.0, w: PAGE_W - 2.0 * MARGIN, h: PAGE_H - top - MARGIN }));
    // draw offsets by the margins
    frame.draw(&mut PagePainter { canvas: &mut canvas }, MARGIN, top);
    // the body fades into the page where it was cut
    if frame.visible_string_range().end < all16 {
        canvas.rect(0.0, PAGE_H - MARGIN - 6.0, PAGE_W, 6.0, PAPER);
        canvas.text_centered("...", BODY_PT, Weight::Regular, MUTED, PAGE_W / 2.0, PAGE_H - MARGIN - 8.0, 40.0);
    }
    canvas.into_pixels().map(ThumbArt::Pixels)
}

/// Plain text and everything that is text and not something more specific (source, JSON, CSV, markdown).
pub struct TextProvider;

impl ThumbProvider for TextProvider {
    fn name(&self) -> &'static str {
        "text"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.text"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let text = text_from_bytes(&read_head(&file.path, HEAD_BYTES)?)?;
        render_page(None, &text, px, Look::Plain)
    }
}

/// Rich text: the control words are dropped and the text laid out.
pub struct RichTextProvider;

impl ThumbProvider for RichTextProvider {
    fn name(&self) -> &'static str {
        "rich-text"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.rtf"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let bytes = read_head(&file.path, HEAD_BYTES * 4)?;
        render_page(None, &rtf_text(&bytes), px, Look::Plain)
    }
}

/// The text of a rich text document: groups that hold fonts, colours, styles or other tables are skipped, `\par`
/// and `\line` become line breaks, `\'hh` and `\uN` become characters.
pub fn rtf_text(bytes: &[u8]) -> String {
    const SKIP: [&str; 9] = ["fonttbl", "colortbl", "stylesheet", "info", "pict", "header", "footer", "generator", "listtable"];
    let src: String = bytes.iter().map(|&b| b as char).collect();
    let cs: Vec<char> = src.chars().collect();
    let mut out = String::new();
    // for each open group: whether it is skipped
    let mut stack: Vec<bool> = Vec::new();
    let mut skipping = 0usize;
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '{' => {
                let mut skip = false;
                // `{\*` is an optional destination, and a skip-listed first word hides the group
                if cs.get(i + 1) == Some(&'\\') {
                    if cs.get(i + 2) == Some(&'*') {
                        skip = true;
                    } else {
                        let word: String = cs[i + 2..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
                        skip = SKIP.contains(&word.as_str());
                    }
                }
                stack.push(skip);
                if skip {
                    skipping += 1;
                }
                i += 1;
            }
            '}' => {
                if stack.pop() == Some(true) {
                    skipping = skipping.saturating_sub(1);
                }
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&c) = cs.get(i) else { break };
                if c.is_ascii_alphabetic() {
                    let word: String = cs[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
                    i += word.len();
                    let neg = cs.get(i) == Some(&'-');
                    if neg {
                        i += 1;
                    }
                    let digits: String = cs[i..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    i += digits.len();
                    let n: i32 = digits.parse().unwrap_or(0) * if neg { -1 } else { 1 };
                    if cs.get(i) == Some(&' ') {
                        i += 1;
                    }
                    if skipping == 0 {
                        match word.as_str() {
                            "par" | "line" | "sect" | "page" => out.push('\n'),
                            "tab" => out.push(' '),
                            "emdash" => out.push('\u{2014}'),
                            "endash" => out.push('\u{2013}'),
                            "bullet" => out.push('\u{2022}'),
                            "lquote" => out.push('\u{2018}'),
                            "rquote" => out.push('\u{2019}'),
                            "ldblquote" => out.push('\u{201C}'),
                            "rdblquote" => out.push('\u{201D}'),
                            "u" => {
                                let code = if n < 0 { n + 65536 } else { n } as u32;
                                if let Some(ch) = char::from_u32(code) {
                                    out.push(ch);
                                }
                                // the fallback character after \uN is skipped
                                if cs.get(i) == Some(&'?') {
                                    i += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    i += 1;
                    match c {
                        '\'' => {
                            let hex: String = cs[i..].iter().take(2).collect();
                            i += hex.len();
                            if skipping == 0 {
                                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                    out.push(b as char);
                                }
                            }
                        }
                        '\\' | '{' | '}' if skipping == 0 => out.push(c),
                        '~' if skipping == 0 => out.push(' '),
                        _ => {}
                    }
                }
            }
            '\r' | '\n' => i += 1,
            c => {
                if skipping == 0 {
                    out.push(c);
                }
                i += 1;
            }
        }
    }
    out
}

/// Web pages: the title and the text of the body. Scripts, styles and markup are dropped; nothing is fetched and no
/// page is rendered.
pub struct WebProvider;

impl ThumbProvider for WebProvider {
    fn name(&self) -> &'static str {
        "web"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.html"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let text = text_from_bytes(&read_head(&file.path, HEAD_BYTES * 4)?)?;
        let (title, body) = html_text(&text);
        render_page(title.as_deref(), &body, px, Look::Plain)
    }
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "copy" => Some('\u{A9}'),
            "hellip" => Some('\u{2026}'),
            "mdash" => Some('\u{2014}'),
            "ndash" => Some('\u{2013}'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
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

/// The title and the visible text of an HTML document.
pub fn html_text(html: &str) -> (Option<String>, String) {
    const BLOCKS: [&str; 20] = ["p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "section", "article", "header", "footer", "ul", "ol", "table", "blockquote", "pre"];
    let lower = html.to_ascii_lowercase();
    let mut title = None;
    let mut body = String::new();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = html[i..].find('<').map_or(html.len(), |p| i + p);
            body.push_str(&html[i..next]);
            i = next;
            continue;
        }
        // a comment
        if lower[i..].starts_with("<!--") {
            i = lower[i..].find("-->").map_or(html.len(), |p| i + p + 3);
            continue;
        }
        let Some(close) = html[i..].find('>').map(|p| i + p) else { break };
        let inner = lower[i + 1..close].trim_start_matches('/').trim_end_matches('/').trim();
        let name: String = inner.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let closing = lower[i + 1..close].starts_with('/');
        if !closing && matches!(name.as_str(), "script" | "style" | "head" | "svg") {
            let end_tag = format!("</{name}");
            let end = lower[close..].find(&end_tag).map_or(html.len(), |p| close + p);
            if name == "head" {
                if let (Some(a), Some(b)) = (lower[close..end].find("<title"), lower[close..end].find("</title")) {
                    let ts = lower[close + a..].find('>').map(|p| close + a + p + 1);
                    if let Some(ts) = ts.filter(|&ts| ts <= close + b) {
                        let t = decode_entities(&html[ts..close + b]);
                        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
                        if !t.is_empty() {
                            title = Some(t);
                        }
                    }
                }
            }
            i = lower[end..].find('>').map_or(html.len(), |p| end + p + 1);
            continue;
        }
        if BLOCKS.contains(&name.as_str()) {
            body.push('\n');
        }
        i = close + 1;
    }
    let decoded = decode_entities(&body);
    // collapse the spaces inside each line
    let text = decoded.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
    (title, text)
}

/// Clippings: a text clipping shows its text, a web location shows its address.
pub struct ClippingsProvider;

impl ThumbProvider for ClippingsProvider {
    fn name(&self) -> &'static str {
        "clippings"
    }
    fn types(&self) -> &'static [&'static str] {
        &["bifrost.text-clipping", "bifrost.web-location"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let text = text_from_bytes(&read_head(&file.path, HEAD_BYTES)?)?;
        let body = clipping_text(&text);
        let title = (file.type_id == "bifrost.web-location").then_some("Web Location");
        render_page(title, &body, px, Look::Clipping)
    }
}

/// The text of a clipping or web location file: the string after the plain-text key (or the URL key) of a property
/// list, else the file's own text.
pub fn clipping_text(file: &str) -> String {
    if file.contains("<plist") {
        for key in ["public.utf8-plain-text", "URL"] {
            let marker = format!("<key>{key}</key>");
            if let Some(p) = file.find(&marker) {
                let rest = &file[p + marker.len()..];
                if let (Some(a), Some(b)) = (rest.find("<string>"), rest.find("</string>")) {
                    if a + 8 <= b {
                        return decode_entities(&rest[a + 8..b]);
                    }
                }
            }
        }
    }
    file.to_string()
}

#[cfg(test)]
mod tests {
    use super::super::canvas::have_font;
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str, data: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-thumbs-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    fn file(p: PathBuf) -> SourceFile {
        SourceFile::new(p, Default::default())
    }

    #[test]
    fn binary_data_is_not_text() {
        assert!(text_from_bytes(b"hello\nworld").is_some());
        assert!(text_from_bytes(&[b'a', 0, b'b']).is_none());
        assert!(text_from_bytes(&[0xFF; 200]).is_none());
        // a head cut inside a character still reads
        let mut cut = "hé".as_bytes().to_vec();
        cut.pop();
        assert_eq!(text_from_bytes(&cut).as_deref(), Some("h"));
    }

    #[test]
    fn tidy_drops_controls_and_squeezes_blank_lines() {
        assert_eq!(tidy("\n\na\tb\r\n\r\n\r\n\r\nc\u{7}\n"), "a b\n\nc");
        assert_eq!(tidy("   \n"), "");
        assert!(tidy(&"x".repeat(10_000)).chars().count() <= MAX_CHARS);
    }

    #[test]
    fn rich_text_keeps_the_words_and_drops_the_tables() {
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Helvetica;}}{\colortbl;\red0\green0\blue0;}\f0\fs24 Hello \b world\b0\par Second \u233?line\'e9 {\*\generator X;}end}";
        let t = rtf_text(rtf);
        assert!(t.contains("Hello world"), "{t:?}");
        assert!(t.contains("\nSecond \u{e9}line\u{e9}"), "{t:?}");
        assert!(!t.contains("Helvetica") && !t.contains("generator") && !t.contains("red0"), "{t:?}");
        assert!(t.ends_with("end"));
    }

    #[test]
    fn html_gives_the_title_and_the_visible_text() {
        let html = "<html><head><title>My &amp; page</title><style>p{color:red}</style></head><body><h1>Head</h1><p>One  two<br>three</p><script>var x = '<p>no</p>';</script><!-- hidden --><p>a &lt; b &#65;</p></body></html>";
        let (title, body) = html_text(html);
        assert_eq!(title.as_deref(), Some("My & page"));
        let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines, vec!["Head", "One two", "three", "a < b A"]);
    }

    #[test]
    fn entities_that_are_not_entities_stay() {
        assert_eq!(decode_entities("a & b &unknown; &#xZZ; &#66;"), "a & b &unknown; &#xZZ; B");
    }

    #[test]
    fn clippings_read_the_property_list_or_the_plain_file() {
        let plist = "<plist><dict><key>public.utf8-plain-text</key><string>Buy &amp; sell</string></dict></plist>";
        assert_eq!(clipping_text(plist), "Buy & sell");
        let loc = "<plist><dict><key>URL</key><string>https://example.org/</string></dict></plist>";
        assert_eq!(clipping_text(loc), "https://example.org/");
        assert_eq!(clipping_text("just text"), "just text");
    }

    #[test]
    fn empty_text_has_no_page() {
        assert!(render_page(None, " \n\t", 128, Look::Plain).is_none());
    }

    #[test]
    fn a_text_page_is_drawn_with_the_typesetter() {
        if !have_font() {
            return;
        }
        let p = tmp("a.txt", b"Hello world\nThis is the first page of a note.\n");
        let art = TextProvider.render(&file(p), 128).expect("a page");
        let ThumbArt::Pixels(px) = art else { panic!("pixels") };
        assert_eq!((px.w, px.h), (98, 128));
        // some pixel differs from the bare paper: ink was drawn
        assert!(px.px.iter().any(|&v| v != PAPER && v != 0));
    }

    #[test]
    fn a_clipping_has_a_red_tab() {
        if !have_font() {
            return;
        }
        let art = render_page(None, "clip", 130, Look::Clipping).unwrap();
        let ThumbArt::Pixels(px) = art else { panic!("pixels") };
        assert_eq!(px.px[3 * px.w + px.w / 2], RED);
    }

    #[test]
    fn a_binary_file_has_no_text_thumbnail() {
        let p = tmp("b.txt", &[0, 1, 2, 3]);
        assert!(TextProvider.render(&file(p), 64).is_none());
    }
}
