//! Safe reading of HTML mail. A mail body is untrusted: scripts, frames, style sheets, event handlers and script
//! URLs are removed, and remote images are not fetched (they are counted, so the view can offer to load them).
//! Only a fixed set of tags and attributes survives; unknown tags are dropped and their text is kept. `to_text`
//! gives the plain text of a body for previews and search.

/// Elements that are removed together with everything inside them.
const DROP_WITH_CONTENT: &[&str] = &[
    "script", "style", "iframe", "frame", "frameset", "object", "applet", "noscript", "template", "title", "head", "svg",
    "math", "xml",
];

/// Elements that survive sanitizing.
const ALLOWED: &[&str] = &[
    "a", "abbr", "b", "blockquote", "br", "code", "dd", "div", "dl", "dt", "em", "h1", "h2", "h3", "h4", "h5", "h6", "hr",
    "i", "img", "li", "ol", "p", "pre", "s", "small", "span", "strong", "sub", "sup", "table", "tbody", "td", "tfoot",
    "th", "thead", "tr", "u", "ul",
];

/// Elements with no closing tag.
const VOID: &[&str] = &["br", "hr", "img"];

/// Elements that end a line in plain text.
const BLOCK: &[&str] = &[
    "p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre", "table", "ul", "ol", "hr", "dt",
    "dd", "br",
];

/// The longest tag, in bytes, that is parsed. A longer run of attributes is not a tag and is shown as text, so a
/// hostile body cannot make every `<` rescan the rest of the input. A quoted `data:` value (an inline image) is not
/// counted against the cap, so a large inline image is still one tag.
const MAX_TAG_BYTES: usize = 4096;

/// The most input, in bytes, that `sanitize` and `to_text` read. The rest of a larger body is ignored.
const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;

/// `s` cut to at most `max` bytes, on a character boundary.
fn clamp(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The result of sanitizing a body: safe HTML, and how many remote images were left out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    pub html: String,
    pub remote_images_blocked: usize,
}

/// Returns `html` with everything unsafe removed. Remote images are dropped unless `allow_remote_images` is set;
/// inline `data:` images in a raster format are kept.
pub fn sanitize(html: &str, allow_remote_images: bool) -> Sanitized {
    sanitize_with_parts(html, allow_remote_images, &[])
}

/// Like [`sanitize`], and `cids` maps the Content-IDs of the message's inline parts (without the angle brackets) to
/// their `data:image/...;base64,` URIs. An image that refers to `cid:ID` takes that URI when it is listed, and is
/// dropped otherwise.
pub fn sanitize_with_parts(html: &str, allow_remote_images: bool, cids: &[(String, String)]) -> Sanitized {
    let html = clamp(html, MAX_HTML_BYTES);
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut blocked = 0;
    let mut i = 0;
    while i < html.len() {
        let Some(rel) = html[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        let lt = i + rel;
        out.push_str(&html[i..lt]);
        if let Some(end) = skip_markup(html, lt) {
            i = end;
            continue;
        }
        let Some(tag) = parse_tag(html, lt) else {
            out.push_str("&lt;");
            i = lt + 1;
            continue;
        };
        i = tag.end;
        let name = tag.name.as_str();
        if !tag.closing && DROP_WITH_CONTENT.contains(&name) {
            i = skip_element(&lower, i, name);
            continue;
        }
        if tag.closing {
            if ALLOWED.contains(&name) && !VOID.contains(&name) {
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
            continue;
        }
        if !ALLOWED.contains(&name) {
            continue;
        }
        let mut resolved: Option<String> = None;
        if name == "img" {
            let src = resolve_cid(attr(&tag.attrs, "src").unwrap_or(""), cids);
            match image_kind(&src) {
                Some(Image::Inline) => {}
                Some(Image::Remote) if allow_remote_images => {}
                Some(Image::Remote) => {
                    blocked += 1;
                    continue;
                }
                None => continue,
            }
            resolved = Some(src);
        }
        out.push('<');
        out.push_str(name);
        for (key, value) in &tag.attrs {
            let value = match resolved.as_deref() {
                Some(src) if key == "src" => src,
                _ => value.as_str(),
            };
            if keep_attr(name, key, value) {
                out.push(' ');
                out.push_str(key);
                out.push_str("=\"");
                out.push_str(&value.replace('"', "&quot;"));
                out.push('"');
            }
        }
        out.push_str(if VOID.contains(&name) { " />" } else { ">" });
    }
    Sanitized { html: out, remote_images_blocked: blocked }
}

/// The plain text of an HTML body: tags and dropped elements are removed, line breaks follow block elements,
/// entities are decoded and runs of blank space are collapsed.
pub fn to_text(html: &str) -> String {
    let html = clamp(html, MAX_HTML_BYTES);
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < html.len() {
        let Some(rel) = html[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        let lt = i + rel;
        out.push_str(&html[i..lt]);
        if let Some(end) = skip_markup(html, lt) {
            i = end;
            continue;
        }
        let Some(tag) = parse_tag(html, lt) else {
            out.push('<');
            i = lt + 1;
            continue;
        };
        i = tag.end;
        let name = tag.name.as_str();
        if !tag.closing && DROP_WITH_CONTENT.contains(&name) {
            i = skip_element(&lower, i, name);
            continue;
        }
        if BLOCK.contains(&name) {
            // A block edge is one line break: `</div><p>` must not add a second one. Blank lines come from the
            // source's own blank runs, which `tidy` keeps to one.
            if !out.ends_with('\n') {
                out.push('\n');
            }
        } else if name == "td" || name == "th" {
            out.push('\t');
        }
    }
    tidy(&decode_entities(&out))
}

enum Image {
    Remote,
    Inline,
}

struct Tag {
    name: String,
    closing: bool,
    attrs: Vec<(String, String)>,
    /// Byte index just after the tag's closing `>`.
    end: usize,
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn keep_attr(tag: &str, key: &str, value: &str) -> bool {
    let digits = || {
        let v = value.trim();
        !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit())
    };
    match (tag, key) {
        ("a", "href") => safe_link(value),
        ("a", "title") | ("img", "alt") => true,
        ("img", "src") => image_kind(value).is_some(),
        ("img", "width") | ("img", "height") => digits(),
        ("td" | "th", "colspan" | "rowspan") => digits(),
        _ => false,
    }
}

/// Lowercased value with the whitespace and control characters that browsers ignore inside a scheme removed.
fn clean_url(value: &str) -> String {
    value.chars().filter(|c| !c.is_ascii_whitespace() && !c.is_ascii_control()).collect::<String>().to_ascii_lowercase()
}

/// Only absolute web and mail links are kept. A value with no colon is relative and has no meaning in a mail, so it
/// is dropped too, which also drops entity-encoded schemes.
fn safe_link(value: &str) -> bool {
    match clean_url(value).split_once(':') {
        Some((scheme, _)) => matches!(scheme, "http" | "https" | "mailto"),
        None => false,
    }
}

/// The source an image really loads from: a `cid:` reference becomes the data URI of its inline part when `cids`
/// lists it. Any other value is returned as it is, and an unknown `cid:` becomes empty (so the image is dropped).
fn resolve_cid(value: &str, cids: &[(String, String)]) -> String {
    let v = value.trim();
    if !v.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("cid:")) {
        return v.to_string();
    }
    let id = v[4..].trim().trim_start_matches('<').trim_end_matches('>');
    cids.iter().find(|(k, _)| k == id).map(|(_, uri)| uri.clone()).unwrap_or_default()
}

fn image_kind(value: &str) -> Option<Image> {
    let v = clean_url(value);
    if v.starts_with("https://") || v.starts_with("http://") {
        return Some(Image::Remote);
    }
    const RASTER: &[&str] = &["png", "jpeg", "jpg", "gif", "webp"];
    let inline = RASTER.iter().any(|t| v.starts_with(&format!("data:image/{t};base64,")));
    if inline {
        Some(Image::Inline)
    } else {
        None
    }
}

/// Skips a comment or a `<!...>` / `<?...>` declaration starting at `lt`, returning the index after it.
fn skip_markup(html: &str, lt: usize) -> Option<usize> {
    let rest = &html[lt..];
    if rest.starts_with("<!--") {
        return Some(match html[lt + 4..].find("-->") {
            Some(p) => lt + 4 + p + 3,
            None => html.len(),
        });
    }
    if rest.starts_with("<!") || rest.starts_with("<?") {
        return Some(html[lt..].find('>').map_or(html.len(), |p| lt + p + 1));
    }
    None
}

/// Index of the closing tag of `name` at or after `from`, or the end of input when there is none.
fn skip_element(lower: &str, from: usize, name: &str) -> usize {
    let pattern = format!("</{name}");
    lower[from..].find(&pattern).map_or(lower.len(), |p| from + p)
}

/// Parses the tag that starts with `<` at `start`. Returns `None` when the text there is not a tag.
///
/// Parsing is linear: a tag is at most `MAX_TAG_BYTES` long, and a `<` where an attribute name or an unquoted value
/// should start ends the attempt, so an unfinished tag never swallows the tags that follow it.
fn parse_tag(src: &str, start: usize) -> Option<Tag> {
    let b = src.as_bytes();
    // The end of what this tag may use. It starts `MAX_TAG_BYTES` after the `<` and grows by the length of every quoted
    // `data:` value (an inline image), which is the one legitimate reason for a tag to be long.
    let mut limit = start.saturating_add(MAX_TAG_BYTES).min(b.len());
    let mut p = start + 1;
    let closing = b.get(p) == Some(&b'/');
    if closing {
        p += 1;
    }
    let name_start = p;
    while p < limit && b[p].is_ascii_alphanumeric() {
        p += 1;
    }
    if p == name_start {
        return None;
    }
    let name = src[name_start..p].to_ascii_lowercase();
    let mut attrs = Vec::new();
    loop {
        while p < limit && (b[p].is_ascii_whitespace() || b[p] == b'/') {
            p += 1;
        }
        if p >= limit {
            return None;
        }
        if b[p] == b'>' {
            return Some(Tag { name, closing, attrs, end: p + 1 });
        }
        if b[p] == b'<' {
            return None;
        }
        let key_start = p;
        while p < limit && !b[p].is_ascii_whitespace() && !matches!(b[p], b'=' | b'>' | b'/' | b'<') {
            p += 1;
        }
        if p == key_start {
            p += 1;
            continue;
        }
        let key = src[key_start..p].to_ascii_lowercase();
        let mut q = p;
        while q < limit && b[q].is_ascii_whitespace() {
            q += 1;
        }
        let mut value = String::new();
        if q < limit && b[q] == b'=' {
            q += 1;
            while q < limit && b[q].is_ascii_whitespace() {
                q += 1;
            }
            if q < limit && (b[q] == b'"' || b[q] == b'\'') {
                let quote = b[q];
                let vs = q + 1;
                // A `data:` value may run past the cap: its closing quote is looked for in the whole input. The search
                // stops at the nearest quote of the same kind, so hostile input cannot make it rescan.
                let data = src.get(vs..vs + 5).is_some_and(|v| v.eq_ignore_ascii_case("data:"));
                let off = if data {
                    let off = b[vs..].iter().position(|&c| c == quote)?;
                    limit = limit.max(vs + off + 1).saturating_add(MAX_TAG_BYTES / 4).min(b.len());
                    off
                } else {
                    b[vs..limit].iter().position(|&c| c == quote)?
                };
                value = src[vs..vs + off].to_string();
                p = vs + off + 1;
            } else {
                let vs = q;
                while q < limit && !b[q].is_ascii_whitespace() && b[q] != b'>' {
                    if b[q] == b'<' {
                        return None;
                    }
                    q += 1;
                }
                value = src[vs..q].to_string();
                p = q;
            }
        }
        attrs.push((key, value));
    }
}

fn decode_entities(s: &str) -> String {
    crate::entities::decode(s)
}

/// Characters that take no room and have no shape: senders pad the preview line of a mail with them. A font draws them as
/// empty boxes, so the text view drops them.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}')
}

/// `line` without its invisible characters. A joiner (U+200C, U+200D) between two visible characters stays: it changes how
/// the letters around it join.
fn strip_invisible(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let visible = |c: Option<&char>| c.is_some_and(|c| !c.is_whitespace() && !is_invisible(*c));
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| {
            !is_invisible(c) || (matches!(c, '\u{200c}' | '\u{200d}') && visible(i.checked_sub(1).and_then(|j| chars.get(j))) && visible(chars.get(i + 1)))
        })
        .map(|(_, &c)| c)
        .collect()
}

/// Collapses runs of blanks inside each line, keeps at most one blank line in a row and trims the ends.
fn tidy(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in s.lines() {
        let line = strip_invisible(line);
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() && lines.last().map_or(true, |p| p.is_empty()) {
            continue;
        }
        lines.push(l);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        sanitize(html, false).html
    }

    #[test]
    fn unfinished_tags_are_linear_and_keep_the_text() {
        let hostile = "<a ".repeat(200_000);
        let start = std::time::Instant::now();
        let _ = to_text(&hostile);
        let _ = sanitize(&hostile, false);
        let quoted = "<a b=\"x ".repeat(100_000);
        let _ = to_text(&quoted);
        assert!(start.elapsed().as_secs() < 5);
        assert_eq!(to_text("<a <b>hi</b>"), "<a hi");
        assert_eq!(clean("<a href=<b>x"), "&lt;a href=<b>x");
    }

    #[test]
    fn a_tag_longer_than_the_cap_is_text() {
        let long = format!("<p {}>x", "a ".repeat(MAX_TAG_BYTES));
        assert!(to_text(&long).starts_with("<p"));
        assert_eq!(to_text("<p>é</p>"), "é");
    }

    #[test]
    fn scripts_and_their_content_are_removed() {
        assert_eq!(clean("<p>hi</p><script>alert(1)</script>!"), "<p>hi</p>!");
        assert_eq!(clean("a<STYLE>p{color:red}</STYLE>b"), "ab");
        assert_eq!(clean("<iframe src=\"https://x\">frame</iframe>ok"), "ok");
    }

    #[test]
    fn event_handlers_and_styles_are_dropped() {
        assert_eq!(clean("<p onclick=\"x()\" style=\"color:red\">t</p>"), "<p>t</p>");
        assert_eq!(clean("<div class=\"a\">t</div>"), "<div>t</div>");
    }

    #[test]
    fn script_urls_are_dropped_and_web_links_kept() {
        assert_eq!(clean("<a href=\"javascript:alert(1)\">x</a>"), "<a>x</a>");
        assert_eq!(clean("<a href=\"java\tscript:alert(1)\">x</a>"), "<a>x</a>");
        assert_eq!(clean("<a href=\"&#106;avascript:x\">x</a>"), "<a>x</a>");
        assert_eq!(clean("<a href=\"https://example.com/a?b=1\">x</a>"), "<a href=\"https://example.com/a?b=1\">x</a>");
        assert_eq!(clean("<a href=\"mailto:a@b.is\">m</a>"), "<a href=\"mailto:a@b.is\">m</a>");
    }

    #[test]
    fn remote_images_are_blocked_and_counted() {
        let s = sanitize("<p><img src=\"https://t.example/px.gif\" alt=\"x\"></p><img src=\"http://a/b.png\">", false);
        assert_eq!(s.html, "<p></p>");
        assert_eq!(s.remote_images_blocked, 2);
        let s = sanitize("<img src=\"https://t.example/px.gif\" alt=\"x\">", true);
        assert_eq!(s.html, "<img src=\"https://t.example/px.gif\" alt=\"x\" />");
        assert_eq!(s.remote_images_blocked, 0);
    }

    #[test]
    fn inline_raster_images_are_kept_and_svg_is_not() {
        let kept = "<img src=\"data:image/png;base64,iVBORw0=\" width=\"16\">";
        assert_eq!(clean(kept), "<img src=\"data:image/png;base64,iVBORw0=\" width=\"16\" />");
        assert_eq!(clean("<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">"), "");
        assert_eq!(clean("<img src=\"cid:part1\">"), "");
    }

    #[test]
    fn dimensions_must_be_plain_numbers() {
        assert_eq!(clean("<td colspan=\"2\" rowspan=\"x\">a</td>"), "<td colspan=\"2\">a</td>");
        assert_eq!(clean("<img src=\"data:image/gif;base64,R0lG\" width=\"100%\">"), "<img src=\"data:image/gif;base64,R0lG\" />");
    }

    #[test]
    fn comments_and_declarations_are_removed() {
        assert_eq!(clean("<!DOCTYPE html><!-- <b>secret</b> -->x"), "x");
        assert_eq!(clean("<!-- unterminated"), "");
    }

    #[test]
    fn unknown_tags_are_unwrapped_and_text_kept() {
        assert_eq!(clean("<font color=\"red\">hi <center>there</center></font>"), "hi there");
    }

    #[test]
    fn stray_and_unterminated_markup_does_not_panic() {
        assert_eq!(clean("1 < 2 and <b"), "1 &lt; 2 and &lt;b");
        assert_eq!(clean("<a href=\"x"), "&lt;a href=\"x");
        assert_eq!(clean("<script>never closed"), "");
    }

    #[test]
    fn icelandic_text_survives() {
        assert_eq!(clean("<p>Þórður fær ÆÐI ð á</p>"), "<p>Þórður fær ÆÐI ð á</p>");
        assert_eq!(to_text("<p>Þórður</p>"), "Þórður");
    }

    #[test]
    fn to_text_breaks_lines_and_decodes_entities() {
        let text = to_text("<head><title>T</title></head><div>One &amp; two</div><p>Three&nbsp;four</p><script>x</script>");
        assert_eq!(text, "One & two\nThree four");
        assert_eq!(to_text("a<br>b"), "a\nb");
        assert_eq!(to_text("&lt;b&gt; &amp;lt;"), "<b> &lt;");
    }

    #[test]
    fn to_text_decodes_named_and_numeric_entities() {
        assert_eq!(to_text("<p>&bull; Sale &#8226; 20&percnt; off &hellip; &euro;5 &#x2713; caf&eacute;</p>"), "\u{2022} Sale \u{2022} 20% off \u{2026} \u{20ac}5 \u{2713} caf\u{e9}");
        assert_eq!(to_text("<p>Ap&#243;tekarinn &amp; &THORN;&oacute;r</p>"), "Ap\u{f3}tekarinn & \u{de}\u{f3}r");
        // names are case-sensitive and the standard has THORN and thorn only: "Thorn" is no reference and stays as written
        assert_eq!(to_text("<p>&Thorn; &thorn;</p>"), "&Thorn; \u{fe}");
        assert_eq!(to_text("<p>&unknown; &amp</p>"), "&unknown; &");
    }

    #[test]
    fn the_padding_senders_put_in_the_preview_line_is_dropped() {
        // hair spaces collapse like any space; zero-width characters leave nothing behind
        assert_eq!(to_text("<div>Hello&#8202;&#8202;&nbsp;&zwnj;&#847;&shy;&#65279; world</div>"), "Hello world");
        assert_eq!(to_text("<div>&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;</div><p>Body</p>"), "Body");
        // a joiner between two letters is part of the word
        assert_eq!(to_text("<p>a&zwj;b</p>"), "a\u{200d}b");
    }

    #[test]
    fn forms_and_controls_are_unwrapped_without_their_actions() {
        let s = clean("<form action=\"https://x/steal\"><input name=\"pw\"><button onclick=\"go()\">Go</button>ok</form>");
        assert_eq!(s, "Gook");
        assert!(!s.contains("form") && !s.contains("input") && !s.contains("onclick"));
        assert_eq!(clean("<img src=\"x.png\" onerror=\"alert(1)\">"), "");
    }

    #[test]
    fn cid_images_take_their_inline_part_only_when_listed() {
        let cids = vec![("logo1".to_string(), "data:image/png;base64,iVBORw0=".to_string())];
        let s = sanitize_with_parts("<p><img src=\"cid:logo1\" alt=\"L\"></p>", false, &cids);
        assert_eq!(s.html, "<p><img src=\"data:image/png;base64,iVBORw0=\" alt=\"L\" /></p>");
        assert_eq!(s.remote_images_blocked, 0);
        let s = sanitize_with_parts("<img src=\"CID:logo1\"><img src=\"cid:missing\">", false, &cids);
        assert_eq!(s.html, "<img src=\"data:image/png;base64,iVBORw0=\" />");
        // a cid part that is not a raster image is never shown
        let svg = vec![("s".to_string(), "data:image/svg+xml;base64,PHN2Zz4=".to_string())];
        assert_eq!(sanitize_with_parts("<img src=\"cid:s\">", true, &svg).html, "");
    }

    #[test]
    fn remote_images_stay_blocked_even_beside_inline_parts() {
        let cids = vec![("a".to_string(), "data:image/gif;base64,R0lG".to_string())];
        let s = sanitize_with_parts("<img src=\"https://t.example/p.gif\"><img src=\"cid:a\">", false, &cids);
        assert_eq!(s.html, "<img src=\"data:image/gif;base64,R0lG\" />");
        assert_eq!(s.remote_images_blocked, 1);
    }

    #[test]
    fn non_ascii_sources_do_not_panic() {
        assert_eq!(clean("<img src=\"Þ\">"), "");
        assert_eq!(sanitize_with_parts("<img src=\"ÞÞ\">", false, &[]).html, "");
    }

    /// A `data:` PNG tag well past `MAX_TAG_BYTES`, with filler that is not a real image.
    fn big_png_tag() -> String {
        format!("<img src=\"data:image/png;base64,iVBORw0KGgo{}\" width=\"600\" alt=\"Welcome\">", "QUFB".repeat(50_000))
    }

    #[test]
    fn a_large_inline_image_tag_is_a_tag_and_never_text() {
        let tag = big_png_tag();
        assert!(tag.len() > 10 * MAX_TAG_BYTES);
        let page = format!("{tag}<p>Welcome aboard</p>");
        assert_eq!(to_text(&page), "Welcome aboard");
        let s = sanitize(&page, false);
        assert!(s.html.starts_with("<img src=\"data:image/png;base64,iVBORw0KGgo"), "the inline image is kept");
        assert!(s.html.ends_with(" width=\"600\" alt=\"Welcome\" /><p>Welcome aboard</p>"));
        assert_eq!(s.remote_images_blocked, 0);
        // a data image that is not a raster format is dropped, not printed
        let svg = format!("<img src=\"data:image/svg+xml;base64,{}\">ok", "QUFB".repeat(50_000));
        assert_eq!(to_text(&svg), "ok");
        assert_eq!(clean(&svg), "ok");
    }

    #[test]
    fn a_remote_image_after_a_large_inline_image_is_still_blocked() {
        let page = format!("{}<img src=\"https://t.example/px.gif\"><p>hi</p>", big_png_tag());
        let s = sanitize(&page, false);
        assert_eq!(s.remote_images_blocked, 1);
        assert!(!s.html.contains("t.example"));
        assert_eq!(to_text(&page), "hi");
    }

    #[test]
    fn an_unclosed_data_value_stays_linear() {
        let hostile = "<img src=\"data:".repeat(50_000);
        let start = std::time::Instant::now();
        let _ = to_text(&hostile);
        let _ = sanitize(&hostile, false);
        let one_open = format!("<img src=\"data:{}", "x".repeat(MAX_HTML_BYTES / 2));
        let _ = to_text(&one_open);
        assert!(start.elapsed().as_secs() < 5);
    }

    #[test]
    fn to_text_collapses_blank_runs() {
        assert_eq!(to_text("<p>a</p>\n\n\n<p>b</p>\n"), "a\n\nb");
        assert_eq!(to_text("<table><tr><td>a</td><td>b</td></tr></table>"), "a b");
    }
}
