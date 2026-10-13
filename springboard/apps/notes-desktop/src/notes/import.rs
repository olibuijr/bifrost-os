//! Import to Notes: turns an HTML or RTF document into the Markdown a note is written in. Both converters are text
//! to text and keep headings, bold, italic, links, lists, quotes and code; everything else (images, tables as
//! grids, colours, fonts) becomes plain text or is dropped. Plain text and Markdown files come in unchanged.

/// The Markdown of a file the open panel picked: HTML and RTF are converted by their extension or their first
/// bytes, anything else is taken as text.
pub fn convert(name: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let lower = name.to_lowercase();
    let head = text.trim_start().chars().take(16).collect::<String>().to_lowercase();
    if lower.ends_with(".rtf") || head.starts_with("{\\rtf") {
        rtf_to_markdown(&text)
    } else if lower.ends_with(".html") || lower.ends_with(".htm") || head.starts_with("<!doctype html") || head.starts_with("<html") {
        html_to_markdown(&text)
    } else {
        text
    }
}

// --- HTML ---

/// The converter's state while it walks the document.
#[derive(Default)]
struct Html {
    out: String,
    /// open lists: `None` is a bulleted one, `Some(n)` a numbered one whose next item is `n`
    lists: Vec<Option<u32>>,
    /// the `href` of each open `<a>` (None when it has none)
    links: Vec<Option<String>>,
    pre: bool,
    /// the element whose content is dropped (script, style, head) until it closes
    skip: Option<String>,
}

impl Html {
    /// Ends the current line.
    fn newline(&mut self) {
        let kept = self.out.trim_end_matches(' ').len();
        self.out.truncate(kept);
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    /// Ends the current line and leaves an empty one after it.
    fn blank(&mut self) {
        self.newline();
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    fn text(&mut self, c: char) {
        if self.pre {
            if c == '\n' {
                self.out.push_str("\n    ");
            } else if c != '\r' {
                self.out.push(c);
            }
        } else if c.is_whitespace() {
            if !self.out.is_empty() && !self.out.ends_with(' ') && !self.out.ends_with('\n') {
                self.out.push(' ');
            }
        } else {
            self.out.push(c);
        }
    }

    fn tag(&mut self, raw: &str) {
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/');
        let name: String = body.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        if let Some(skipping) = &self.skip {
            if closing && *skipping == name {
                self.skip = None;
            }
            return;
        }
        match name.as_str() {
            "script" | "style" | "head" if !closing => self.skip = Some(name.clone()),
            "br" => self.newline(),
            "p" | "table" | "figure" => self.blank(),
            "div" | "section" | "article" | "tr" | "header" | "footer" => self.newline(),
            "td" | "th" => {
                if closing {
                    self.text(' ');
                }
            }
            "blockquote" => {
                self.blank();
                if !closing {
                    self.out.push_str("> ");
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.blank();
                if !closing {
                    self.out.push_str(match name.as_str() {
                        "h1" => "# ",
                        "h2" => "## ",
                        _ => "### ",
                    });
                }
            }
            "ul" | "ol" => {
                if closing {
                    self.lists.pop();
                    if self.lists.is_empty() {
                        self.blank();
                    } else {
                        self.newline();
                    }
                } else {
                    self.newline();
                    self.lists.push(if name == "ol" { Some(1) } else { None });
                }
            }
            "li" => {
                self.newline();
                if !closing {
                    self.out.push_str(&"  ".repeat(self.lists.len().saturating_sub(1)));
                    match self.lists.last_mut() {
                        Some(Some(n)) => {
                            self.out.push_str(&format!("{n}. "));
                            *n += 1;
                        }
                        _ => self.out.push_str("- "),
                    }
                }
            }
            "b" | "strong" => self.out.push_str("**"),
            "i" | "em" => self.out.push('*'),
            "s" | "strike" | "del" => self.out.push_str("~~"),
            "u" | "ins" => self.out.push_str("++"),
            "mark" => self.out.push_str("=="),
            "code" if !self.pre => self.out.push('`'),
            "pre" => {
                self.blank();
                if closing {
                    self.pre = false;
                } else {
                    self.out.push_str("    ");
                    self.pre = true;
                }
            }
            "a" => {
                if closing {
                    if let Some(Some(href)) = self.links.pop() {
                        self.out.push_str(&format!("]({href})"));
                    }
                } else {
                    let href = attribute(body, "href").filter(|h| !h.is_empty() && !h.starts_with('#') && !h.to_lowercase().starts_with("javascript:"));
                    if href.is_some() {
                        self.out.push('[');
                    }
                    self.links.push(href);
                }
            }
            _ => {}
        }
    }
}

/// The value of attribute `name` in the text of a start tag (quoted or bare), compared without case.
fn attribute(tag: &str, name: &str) -> Option<String> {
    // ASCII lowercase keeps every byte offset the same in both strings
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        let before_ok = start == 0 || lower[..start].ends_with(char::is_whitespace);
        let rest = tag[start + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let value = rest[1..].trim_start();
            let end = match value.chars().next() {
                Some(q @ ('"' | '\'')) => return value[1..].split(q).next().map(|v| decode_entities(v)),
                _ => value.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(value.len()),
            };
            return Some(decode_entities(&value[..end]));
        }
        from = start + name.len();
    }
    None
}

/// Replaces the character references of `s` (`&amp;`, `&#39;`, `&#x1F600;`, `&nbsp;`); unknown ones stay as written.
fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('\u{2013}'),
            "mdash" => Some('\u{2014}'),
            "hellip" => Some('\u{2026}'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
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

/// An HTML document as Markdown.
pub fn html_to_markdown(html: &str) -> String {
    let mut h = Html::default();
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |e| &after[e + 3..]);
        } else if rest.starts_with('<') && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/') {
            let Some(end) = rest.find('>') else { break };
            let tag = rest[1..end].to_string();
            h.tag(tag.trim_end_matches('/'));
            rest = &rest[end + 1..];
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
        } else {
            let first = rest.chars().next().map_or(1, char::len_utf8);
            let next = rest[first..].find(|c: char| c == '<' || c == '&').map_or(rest.len(), |e| e + first);
            let chunk = &rest[..next];
            let text = if chunk.starts_with('&') { decode_entities(chunk) } else { chunk.to_string() };
            if h.skip.is_none() {
                for c in text.chars() {
                    h.text(c);
                }
            }
            rest = &rest[next..];
        }
    }
    h.newline();
    // at most one empty line in a row, and none at the ends
    let mut lines: Vec<&str> = vec![];
    for line in h.out.split('\n') {
        let line = line.trim_end();
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

// --- RTF ---

/// The formatting of one RTF group.
#[derive(Clone, Copy, Default)]
struct Group {
    bold: bool,
    italic: bool,
    /// a destination whose content is not text (font table, colour table, pictures, ...)
    skip: bool,
}

/// Writes RTF text with `**` and `*` marks opened and closed to follow the groups' formatting.
#[derive(Default)]
struct Rtf {
    out: String,
    bold_open: bool,
    italic_open: bool,
}

impl Rtf {
    fn text(&mut self, g: Group, c: char) {
        if g.skip {
            return;
        }
        if g.bold != self.bold_open {
            self.out.push_str("**");
            self.bold_open = g.bold;
        }
        if g.italic != self.italic_open {
            self.out.push('*');
            self.italic_open = g.italic;
        }
        self.out.push(c);
    }

    fn close(&mut self) {
        if self.bold_open {
            self.out.push_str("**");
            self.bold_open = false;
        }
        if self.italic_open {
            self.out.push('*');
            self.italic_open = false;
        }
    }

    fn newline(&mut self, g: Group) {
        if !g.skip {
            self.close();
            self.out.push('\n');
        }
    }
}

/// The char of a Windows-1252 byte (the `\'hh` escape); the 0x80 to 0x9f block is the few that RTF writers use.
fn cp1252(b: u8) -> char {
    match b {
        0x80 => '\u{20ac}',
        0x85 => '\u{2026}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        b => char::from(b),
    }
}

/// An RTF document as Markdown: paragraphs, bold and italic, the usual escapes and Unicode; font and colour
/// tables, pictures and other destinations are dropped.
pub fn rtf_to_markdown(rtf: &str) -> String {
    let chars: Vec<char> = rtf.chars().collect();
    let (mut stack, mut cur, mut w) = (Vec::<Group>::new(), Group::default(), Rtf::default());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                stack.push(cur);
                i += 1;
            }
            '}' => {
                if let Some(g) = stack.pop() {
                    cur = g;
                }
                i += 1;
            }
            '\r' | '\n' => i += 1,
            '\\' => {
                i += 1;
                let Some(&c) = chars.get(i) else { break };
                match c {
                    '\\' | '{' | '}' => {
                        w.text(cur, c);
                        i += 1;
                    }
                    '\'' => {
                        let hex: String = chars.iter().skip(i + 1).take(2).collect();
                        if let Ok(b) = u8::from_str_radix(&hex, 16) {
                            w.text(cur, cp1252(b));
                        }
                        i += 3;
                    }
                    '*' => {
                        cur.skip = true;
                        i += 1;
                    }
                    '~' => {
                        w.text(cur, ' ');
                        i += 1;
                    }
                    '\n' | '\r' => {
                        w.newline(cur);
                        i += 1;
                    }
                    c if c.is_ascii_alphabetic() => {
                        let start = i;
                        while chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                            i += 1;
                        }
                        let word: String = chars[start..i].iter().collect();
                        let sign = if chars.get(i) == Some(&'-') { i += 1; -1 } else { 1 };
                        let digits = i;
                        while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                            i += 1;
                        }
                        let param: Option<i32> = chars[digits..i].iter().collect::<String>().parse::<i32>().ok().map(|n| n * sign);
                        // one space ends a control word
                        if chars.get(i) == Some(&' ') {
                            i += 1;
                        }
                        match word.as_str() {
                            "par" | "line" | "sect" | "page" => w.newline(cur),
                            "tab" => w.text(cur, '\t'),
                            "b" => cur.bold = param != Some(0),
                            "i" => cur.italic = param != Some(0),
                            "plain" => {
                                cur.bold = false;
                                cur.italic = false;
                            }
                            "bullet" => w.text(cur, '\u{2022}'),
                            "endash" => w.text(cur, '\u{2013}'),
                            "emdash" => w.text(cur, '\u{2014}'),
                            "lquote" => w.text(cur, '\u{2018}'),
                            "rquote" => w.text(cur, '\u{2019}'),
                            "ldblquote" => w.text(cur, '\u{201c}'),
                            "rdblquote" => w.text(cur, '\u{201d}'),
                            "u" => {
                                if let Some(n) = param {
                                    if let Some(ch) = char::from_u32(n as i16 as u16 as u32) {
                                        w.text(cur, ch);
                                    }
                                }
                                // the one-char fallback that follows is not text
                                match chars.get(i) {
                                    Some('\\') if chars.get(i + 1) == Some(&'\'') => i += 4,
                                    Some(c) if !matches!(c, '\\' | '{' | '}') => i += 1,
                                    _ => {}
                                }
                            }
                            "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header" | "footer" | "headerl" | "headerr" | "footerl" | "footerr" | "filetbl"
                            | "listtable" | "listoverridetable" | "revtbl" | "generator" | "object" | "fldinst" | "themedata" | "colorschememapping" | "datastore" => cur.skip = true,
                            _ => {}
                        }
                    }
                    _ => i += 1,
                }
            }
            c => {
                w.text(cur, c);
                i += 1;
            }
        }
    }
    w.close();
    let lines: Vec<&str> = w.out.split('\n').map(str::trim_end).collect();
    let (mut start, mut end) = (0, lines.len());
    while start < end && lines[start].is_empty() {
        start += 1;
    }
    while end > start && lines[end - 1].is_empty() {
        end -= 1;
    }
    lines[start..end].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_keeps_headings_emphasis_links_and_paragraphs() {
        let html = "<html><head><title>Ignored</title><style>p{color:red}</style></head><body>\
            <h1>Trip &amp; Plans</h1><p>Pack <b>light</b> and <em>early</em>, see <a href=\"https://x.org/a?b=1&amp;c=2\">the guide</a>.</p>\
            <h2>Gear</h2><p>Line one<br>line two</p><script>alert(1)</script></body></html>";
        assert_eq!(
            html_to_markdown(html),
            "# Trip & Plans\n\nPack **light** and *early*, see [the guide](https://x.org/a?b=1&c=2).\n\n## Gear\n\nLine one\nline two"
        );
    }

    #[test]
    fn html_lists_nest_and_number() {
        let html = "<ul><li>one</li><li>two<ul><li>inner</li></ul></li></ul><ol><li>first</li><li>second</li></ol>";
        assert_eq!(html_to_markdown(html), "- one\n- two\n  - inner\n\n1. first\n2. second");
    }

    #[test]
    fn html_code_quotes_and_entities() {
        let html = "<p>Use <code>cargo test</code> &lt;now&gt; &#169; &#x41; &nbsp;done &bogus; &</p>\
            <blockquote>said so</blockquote><pre>let a = 1;\nlet b = 2;</pre><p>~~ <s>old</s> <u>new</u> <mark>key</mark></p>";
        let md = html_to_markdown(html);
        assert_eq!(
            md,
            "Use `cargo test` <now> \u{a9} A done &bogus; &\n\n> said so\n\n    let a = 1;\n    let b = 2;\n\n~~ ~~old~~ ++new++ ==key=="
        );
    }

    #[test]
    fn html_drops_comments_scripts_and_unsafe_links() {
        let html = "<!-- note --><p>a<!-- hidden -->b</p><p><a href=\"javascript:x()\">bad</a> <a href=\"#top\">top</a> <a>plain</a></p><style>x</style>tail";
        assert_eq!(html_to_markdown(html), "ab\n\nbad top plain\n\ntail");
        assert_eq!(html_to_markdown("   "), "");
        assert_eq!(html_to_markdown("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2", "a lone < is text");
    }

    #[test]
    fn attributes_are_found_by_name_with_any_quoting() {
        assert_eq!(attribute("a href=\"x y\" class=z", "href").as_deref(), Some("x y"));
        assert_eq!(attribute("a HREF='q&amp;r'", "href").as_deref(), Some("q&r"));
        assert_eq!(attribute("a data-href=\"no\" href=bare>", "href").as_deref(), Some("bare"));
        assert_eq!(attribute("a class=x", "href"), None);
    }

    #[test]
    fn rtf_keeps_paragraphs_bold_italic_and_unicode() {
        let rtf = "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Helvetica;}}{\\colortbl;\\red0\\green0\\blue0;}\\f0\\fs24 \\b Hello\\b0  \\i world\\i0\\par Caf\\u233? \\'e9 {\\*\\generator Writer;}end\\par}";
        assert_eq!(rtf_to_markdown(rtf), "**Hello** *world*\nCaf\u{e9} \u{e9} end");
    }

    #[test]
    fn rtf_escapes_groups_and_destinations() {
        assert_eq!(rtf_to_markdown("{\\rtf1 a \\{b\\} \\\\ c\\tab d\\line e}"), "a {b} \\ c\td\ne");
        assert_eq!(rtf_to_markdown("{\\rtf1 {\\info {\\title Secret}}shown}"), "shown");
        assert_eq!(rtf_to_markdown("{\\rtf1 {\\b bold in group} plain}"), "**bold in group** plain");
        assert_eq!(rtf_to_markdown("{\\rtf1 \\b open at end"), "**open at end**", "marks are closed at the end");
        assert_eq!(rtf_to_markdown("{\\rtf1 \\ldblquote q\\rdblquote \\endash \\bullet }"), "\u{201c}q\u{201d}\u{2013}\u{2022}");
        assert_eq!(rtf_to_markdown(""), "");
    }

    #[test]
    fn the_copy_made_by_notes_comes_back_as_the_same_markdown() {
        let md = "**bold** and *it*\n# Head";
        let back = rtf_to_markdown(&super::super::system::rtf(md));
        assert!(back.contains("**bold** and *it*"), "{back:?}");
        assert!(back.contains("Head"), "{back:?}");
    }

    #[test]
    fn files_are_converted_by_extension_or_first_bytes_and_text_stays_as_it_is() {
        assert_eq!(convert("a.html", b"<p>x</p>"), "x");
        assert_eq!(convert("page", b"  <!DOCTYPE html><p>y</p>"), "y");
        assert_eq!(convert("a.rtf", b"{\\rtf1 z}"), "z");
        assert_eq!(convert("noext", b"{\\rtf1 w}"), "w");
        assert_eq!(convert("a.md", b"# T\n- [ ] <b>"), "# T\n- [ ] <b>");
        assert_eq!(convert("a.txt", "caf\u{e9}".as_bytes()), "caf\u{e9}");
        assert_eq!(convert("a.txt", &[0x61, 0xff]), "a\u{fffd}", "bytes that are not text are replaced");
    }
}
