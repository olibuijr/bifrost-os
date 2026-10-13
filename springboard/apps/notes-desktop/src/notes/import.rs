//! Import to Notes: turns a chosen file (plain text, Markdown, HTML or RTF) into the Markdown a note holds, and
//! summarises a batch with the files that were skipped. The converters are small and forgiving: they keep the text and
//! the structure people notice (headings, emphasis, lists, paragraphs) and drop the rest.

/// What kind of file the bytes are, from the name's extension and a peek at the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Html,
    Rtf,
}

/// The uniform type identifiers the open panel offers.
pub const TYPES: [&str; 3] = ["public.text", "public.html", "public.rtf"];

/// The kind of file `name` with these `bytes` is.
pub fn kind_of(name: &str, bytes: &[u8]) -> Kind {
    let lower = name.to_lowercase();
    let head: String = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).trim_start().to_lowercase();
    if lower.ends_with(".rtf") || head.starts_with("{\\rtf") {
        Kind::Rtf
    } else if lower.ends_with(".html") || lower.ends_with(".htm") || head.starts_with("<!doctype html") || head.starts_with("<html") {
        Kind::Html
    } else {
        Kind::Text
    }
}

/// The note text for an imported file.
pub fn to_markdown(name: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let md = match kind_of(name, bytes) {
        Kind::Text => text.replace("\r\n", "\n"),
        Kind::Html => from_html(&text),
        Kind::Rtf => from_rtf(&text),
    };
    md.trim_matches('\n').to_string()
}

fn entity(name: &str) -> Option<String> {
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some(c.to_string())
}

/// HTML to Markdown: headings, bold, italic, lists, line and paragraph breaks; scripts and styles are dropped.
pub fn from_html(html: &str) -> String {
    let chars: Vec<char> = html.chars().collect();
    let mut out = String::new();
    let mut lists: Vec<Option<usize>> = Vec::new();
    let mut skip: Option<String> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '<' {
            let end = chars[i..].iter().position(|&x| x == '>').map(|p| i + p);
            let Some(end) = end else { break };
            let tag: String = chars[i + 1..end].iter().collect();
            i = end + 1;
            let closing = tag.starts_with('/');
            let name: String = tag.trim_start_matches('/').chars().take_while(|ch| ch.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
            if let Some(s) = &skip {
                if closing && *s == name {
                    skip = None;
                }
                continue;
            }
            match (name.as_str(), closing) {
                ("script" | "style" | "head", false) => skip = Some(name.clone()),
                ("b" | "strong", _) => out.push_str("**"),
                ("i" | "em", _) => out.push('*'),
                ("code", _) => out.push('`'),
                ("br", _) => out.push('\n'),
                ("h1", false) => block_start(&mut out, "# "),
                ("h2", false) => block_start(&mut out, "## "),
                ("h3" | "h4" | "h5" | "h6", false) => block_start(&mut out, "### "),
                ("h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "tr" | "ul" | "ol", true) => {
                    if name == "ul" || name == "ol" {
                        lists.pop();
                    }
                    block_end(&mut out);
                }
                ("p" | "div" | "tr", false) => block_start(&mut out, ""),
                ("ul" | "ol", false) => {
                    if lists.is_empty() {
                        block_start(&mut out, "");
                    }
                    lists.push(if name == "ol" { Some(0) } else { None });
                }
                ("li", false) => {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                    match lists.last_mut() {
                        Some(Some(n)) => {
                            *n += 1;
                            out.push_str(&format!("{n}. "));
                        }
                        _ => out.push_str("- "),
                    }
                }
                ("li", true) => block_end(&mut out),
                ("td" | "th", true) => out.push('\t'),
                _ => {}
            }
        } else if skip.is_some() {
            i += 1;
        } else if c == '&' {
            let semi = chars[i..].iter().take(12).position(|&x| x == ';');
            match semi.and_then(|p| entity(&chars[i + 1..i + p].iter().collect::<String>())) {
                Some(s) => {
                    out.push_str(&s);
                    i += semi.unwrap() + 1;
                }
                None => {
                    out.push('&');
                    i += 1;
                }
            }
        } else if c == '\n' || c == '\r' || c == '\t' {
            // source whitespace collapses to one space
            if !out.ends_with([' ', '\n']) && !out.is_empty() {
                out.push(' ');
            }
            i += 1;
        } else {
            if c == ' ' && out.ends_with([' ', '\n']) {
                i += 1;
                continue;
            }
            out.push(c);
            i += 1;
        }
    }
    squeeze(&out)
}

fn block_start(out: &mut String, marker: &str) {
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
    }
    out.push_str(marker);
}

fn block_end(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// Trailing spaces off every line and no run of more than one blank line.
fn squeeze(s: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for l in s.split('\n') {
        let l = l.trim_end();
        if l.is_empty() && out.last().is_some_and(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    out.join("\n").trim_matches('\n').to_string()
}

/// RTF to plain paragraphs: control words are dropped except paragraph and line breaks, tabs, `\'xx` bytes and `\uN`
/// characters; font, colour, style and info tables and `{\*...}` destinations are skipped.
pub fn from_rtf(rtf: &str) -> String {
    let chars: Vec<char> = rtf.chars().collect();
    let mut out = String::new();
    // per open group: whether its text is dropped
    let mut stack: Vec<bool> = Vec::new();
    let mut skipping = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                stack.push(skipping);
                i += 1;
                // a destination group starts with \* or a table control word
                if chars.get(i) == Some(&'\\') {
                    let word: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphabetic() || **c == '*').collect();
                    if matches!(word.as_str(), "*" | "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header" | "footer") {
                        skipping = true;
                    }
                }
            }
            '}' => {
                skipping = stack.pop().unwrap_or(false);
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&n) = chars.get(i) else { break };
                if n == '\'' {
                    let hex: String = chars[i + 1..].iter().take(2).collect();
                    if let Ok(b) = u8::from_str_radix(&hex, 16) {
                        if !skipping {
                            // the single-byte code page is Latin-1 compatible for the common range
                            out.push(b as char);
                        }
                    }
                    i += 3;
                } else if n.is_ascii_alphabetic() {
                    let word: String = chars[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
                    i += word.len();
                    let sign = if chars.get(i) == Some(&'-') { i += 1; -1 } else { 1 };
                    let digits: String = chars[i..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    i += digits.len();
                    if chars.get(i) == Some(&' ') {
                        i += 1;
                    }
                    if skipping {
                        continue;
                    }
                    match word.as_str() {
                        "par" | "line" => out.push('\n'),
                        "tab" => out.push('\t'),
                        "u" => {
                            if let Ok(v) = digits.parse::<i32>() {
                                let code = (if sign < 0 { 65536 - v } else { v }) as u32;
                                if let Some(ch) = char::from_u32(code) {
                                    out.push(ch);
                                }
                                // the ANSI fallback char that follows \uN is not text
                                if chars.get(i) == Some(&'?') {
                                    i += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    if !skipping && matches!(n, '\\' | '{' | '}') {
                        out.push(n);
                    }
                    i += 1;
                }
            }
            '\r' | '\n' => i += 1,
            c => {
                if !skipping {
                    out.push(c);
                }
                i += 1;
            }
        }
    }
    squeeze(&out)
}

/// The outcome of importing several files: how many became notes and which were skipped, with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub imported: usize,
    pub skipped: Vec<(String, String)>,
}

impl Summary {
    pub fn skip(&mut self, name: &str, why: &str) {
        self.skipped.push((name.to_string(), why.to_string()));
    }

    /// The sheet's text: "3 notes imported" and one line per skipped file.
    pub fn message(&self) -> String {
        let head = if self.imported == 1 { "1 note imported".to_string() } else { format!("{} notes imported", self.imported) };
        if self.skipped.is_empty() {
            return head;
        }
        let mut s = format!("{head}, {} skipped", self.skipped.len());
        for (name, why) in &self.skipped {
            s.push_str(&format!("\n{name}: {why}"));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_kind_by_name_then_content() {
        assert_eq!(kind_of("a.html", b""), Kind::Html);
        assert_eq!(kind_of("a.RTF", b""), Kind::Rtf);
        assert_eq!(kind_of("a", b"{\\rtf1 hi}"), Kind::Rtf);
        assert_eq!(kind_of("a", b"<!DOCTYPE html><p>x"), Kind::Html);
        assert_eq!(kind_of("a.md", b"# t"), Kind::Text);
    }

    #[test]
    fn html_maps_structure_to_markdown() {
        let html = "<html><head><style>p{}</style></head><body><h1>Trip</h1><p>Pack <b>warm</b> &amp; <i>dry</i> things.</p><ul><li>tent</li><li>stove</li></ul><ol><li>go</li><li>eat</li></ol></body></html>";
        assert_eq!(from_html(html), "# Trip\n\nPack **warm** & *dry* things.\n\n- tent\n- stove\n\n1. go\n2. eat");
    }

    #[test]
    fn html_entities_and_whitespace() {
        assert_eq!(from_html("a\n   b&#65;&#x42;&nbsp;c&bogus;<br>d"), "a bAB c&bogus;\nd");
    }

    #[test]
    fn rtf_keeps_text_and_paragraphs() {
        let rtf = "{\\rtf1\\ansi{\\fonttbl{\\f0 Helvetica;}}{\\colortbl;\\red0\\green0\\blue0;}\\f0\\fs24 Hello \\b world\\b0\\par Caf\\'e9 \\u8364?\\par}";
        assert_eq!(from_rtf(rtf), "Hello world\nCaf\u{e9} \u{20ac}");
    }

    #[test]
    fn to_markdown_dispatches_and_trims() {
        assert_eq!(to_markdown("n.txt", b"\r\nline\r\n\r\n"), "line");
        assert_eq!(to_markdown("n.html", b"<h2>Hi</h2>"), "## Hi");
    }

    #[test]
    fn summary_lists_skipped_files() {
        let mut s = Summary { imported: 2, ..Summary::default() };
        assert_eq!(s.message(), "2 notes imported");
        s.skip("big.txt", "too large");
        assert_eq!(s.message(), "2 notes imported, 1 skipped\nbig.txt: too large");
    }
}
