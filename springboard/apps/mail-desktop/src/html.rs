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

/// The result of sanitizing a body: safe HTML, and how many remote images were left out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    pub html: String,
    pub remote_images_blocked: usize,
}

/// Returns `html` with everything unsafe removed. Remote images are dropped unless `allow_remote_images` is set;
/// inline `data:` images in a raster format are kept.
pub fn sanitize(html: &str, allow_remote_images: bool) -> Sanitized {
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
        if name == "img" {
            let src = attr(&tag.attrs, "src").unwrap_or("");
            match image_kind(src) {
                Some(Image::Inline) => {}
                Some(Image::Remote) if allow_remote_images => {}
                Some(Image::Remote) => {
                    blocked += 1;
                    continue;
                }
                None => continue,
            }
        }
        out.push('<');
        out.push_str(name);
        for (key, value) in &tag.attrs {
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
            out.push('\n');
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
fn parse_tag(src: &str, start: usize) -> Option<Tag> {
    let b = src.as_bytes();
    let mut p = start + 1;
    let closing = b.get(p) == Some(&b'/');
    if closing {
        p += 1;
    }
    let name_start = p;
    while p < b.len() && b[p].is_ascii_alphanumeric() {
        p += 1;
    }
    if p == name_start {
        return None;
    }
    let name = src[name_start..p].to_ascii_lowercase();
    let mut attrs = Vec::new();
    loop {
        while p < b.len() && (b[p].is_ascii_whitespace() || b[p] == b'/') {
            p += 1;
        }
        if p >= b.len() {
            return None;
        }
        if b[p] == b'>' {
            return Some(Tag { name, closing, attrs, end: p + 1 });
        }
        let key_start = p;
        while p < b.len() && !b[p].is_ascii_whitespace() && !matches!(b[p], b'=' | b'>' | b'/') {
            p += 1;
        }
        if p == key_start {
            p += 1;
            continue;
        }
        let key = src[key_start..p].to_ascii_lowercase();
        let mut q = p;
        while q < b.len() && b[q].is_ascii_whitespace() {
            q += 1;
        }
        let mut value = String::new();
        if q < b.len() && b[q] == b'=' {
            q += 1;
            while q < b.len() && b[q].is_ascii_whitespace() {
                q += 1;
            }
            if q < b.len() && (b[q] == b'"' || b[q] == b'\'') {
                let quote = b[q];
                let vs = q + 1;
                let off = b[vs..].iter().position(|&c| c == quote)?;
                value = src[vs..vs + off].to_string();
                p = vs + off + 1;
            } else {
                let vs = q;
                while q < b.len() && !b[q].is_ascii_whitespace() && b[q] != b'>' {
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
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// Collapses runs of blanks inside each line, keeps at most one blank line in a row and trims the ends.
fn tidy(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in s.lines() {
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
    fn to_text_collapses_blank_runs() {
        assert_eq!(to_text("<p>a</p>\n\n\n<p>b</p>\n"), "a\n\nb");
        assert_eq!(to_text("<table><tr><td>a</td><td>b</td></tr></table>"), "a b");
    }
}
