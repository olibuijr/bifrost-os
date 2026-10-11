//! A tolerant RSS 2.0 / Atom reader: no XML crate, just enough scanning to
//! pull items (title, link, date, body) out of the feeds Icelandic news
//! sites publish. Malformed input yields fewer items, never a panic.

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub date: String,
    pub body: String,
    /// the feed's display name
    pub source: String,
    /// the article's picture, when the feed names one (enclosure, media:*, or an <img> in the text)
    pub image: String,
}

/// Parses an RSS or Atom document into items (at most `MAX_ITEMS`).
pub fn parse(xml: &str, source: &str) -> Vec<Item> {
    let mut items = vec![];
    for tag in ["item", "entry"] {
        let mut rest = xml;
        while let Some((block, after)) = next_block(rest, tag) {
            rest = after;
            if let Some(item) = parse_item(block, source) {
                items.push(item);
                if items.len() >= MAX_ITEMS {
                    return items;
                }
            }
        }
        if !items.is_empty() {
            break;
        }
    }
    items
}

pub const MAX_ITEMS: usize = 60;

fn parse_item(block: &str, source: &str) -> Option<Item> {
    let title = clean(&inner(block, "title")?);
    if title.is_empty() {
        return None;
    }
    let link = link_of(block);
    let date = ["pubDate", "published", "updated", "dc:date"].iter().find_map(|t| inner(block, t)).map(|d| short_date(&clean(&d))).unwrap_or_default();
    let body = ["content:encoded", "content", "description", "summary"].iter().find_map(|t| inner(block, t).map(|b| clean(&b)).filter(|b| !b.is_empty())).unwrap_or_default();
    Some(Item { title, link, date, body, source: source.to_string(), image: image_of(block) })
}

/// The first picture an item names: `<enclosure type="image/..." url=...>`, `<media:content|thumbnail url=...>`,
/// or the first `<img src=...>` inside its (entity-encoded or CDATA) text.
fn image_of(block: &str) -> String {
    for tag in ["<enclosure", "<media:content", "<media:thumbnail"] {
        let mut from = 0;
        while let Some(i) = block[from..].find(tag) {
            let at = from + i;
            let Some(gt) = block[at..].find('>') else { break };
            let t = &block[at..at + gt];
            if let Some(u) = attr(t, "url") {
                let is_image = attr(t, "type").is_none_or(|ty| ty.starts_with("image")) && attr(t, "medium").is_none_or(|m| m == "image");
                if is_image && !u.is_empty() {
                    return decode(&u);
                }
            }
            from = at + gt;
        }
    }
    let text = decode(block);
    let mut from = 0;
    while let Some(i) = text[from..].find("<img") {
        let at = from + i;
        let Some(gt) = text[at..].find('>') else { break };
        if let Some(u) = attr(&text[at..at + gt], "src").filter(|u| u.starts_with("http")) {
            return decode(&u);
        }
        from = at + gt;
    }
    String::new()
}

/// The next `<tag ...>...</tag>` block and what follows it.
fn next_block<'a>(s: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let (_start, open_end) = find_open(s, tag)?;
    let close = format!("</{tag}>");
    let end = s[open_end..].find(&close)? + open_end;
    Some((&s[open_end..end], &s[end + close.len()..]))
}

/// Finds `<tag` followed by `>`, whitespace or `/`; returns (start, index after the `>`).
fn find_open(s: &str, tag: &str) -> Option<(usize, usize)> {
    let pat = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = s[from..].find(&pat) {
        let at = from + i;
        let next = s[at + pat.len()..].chars().next()?;
        if next == '>' || next == '/' || next.is_whitespace() {
            let gt = s[at..].find('>')? + at;
            if s[..gt].ends_with('/') {
                // a self-closing tag has no body
                from = gt + 1;
                continue;
            }
            return Some((at, gt + 1));
        }
        from = at + pat.len();
    }
    None
}

/// The raw text between `<tag>` and `</tag>`.
fn inner(s: &str, tag: &str) -> Option<String> {
    let (_, open_end) = find_open(s, tag)?;
    let close = format!("</{tag}>");
    let end = s[open_end..].find(&close)? + open_end;
    Some(s[open_end..end].to_string())
}

/// RSS `<link>text</link>` or Atom `<link href="..." rel="alternate"/>`.
fn link_of(block: &str) -> String {
    if let Some(l) = inner(block, "link") {
        let l = clean(&l);
        if !l.is_empty() {
            return l;
        }
    }
    let mut from = 0;
    while let Some(i) = block[from..].find("<link") {
        let at = from + i;
        let Some(gt) = block[at..].find('>') else { break };
        let tag = &block[at..at + gt];
        if let Some(h) = attr(tag, "href") {
            if !tag.contains("rel=\"self\"") && !tag.contains("rel=\"enclosure\"") {
                return decode(&h);
            }
        }
        from = at + gt;
    }
    String::new()
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let pat = format!("{name}=");
    let i = tag.find(&pat)? + pat.len();
    let rest = &tag[i..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = rest[1..].find(quote)?;
    Some(rest[1..1 + end].to_string())
}

/// Unwraps CDATA, decodes entities, strips markup and collapses whitespace.
pub fn clean(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    while let Some(i) = text.find("<![CDATA[") {
        let end = text[i..].find("]]>").map_or(text.len(), |e| e + i);
        let body = text[i + 9..end].to_string();
        text.replace_range(i..(end + 3).min(text.len()), &body);
    }
    let text = decode(&text);
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    // soft hyphens (Visir hyphenates long words with them) and zero-width marks have no glyph to draw
    let out: String = decode(&out).chars().filter(|c| !matches!(c, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{2060}' | '\u{feff}')).collect();
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decodes the XML and common HTML entities.
pub fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match rest.find(';').filter(|&e| e <= 10) {
            Some(e) => {
                let name = &rest[1..e];
                let ch = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "shy" => Some('\u{ad}'),
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
                match ch {
                    Some(c) => {
                        out.push(c);
                        rest = &rest[e + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
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

/// "Mon, 05 Oct 2026 08:15:00 +0000" becomes "Mon, 05 Oct 2026 08:15";
/// ISO dates keep their date and minute.
pub fn short_date(raw: &str) -> String {
    // ISO 8601 (2026-10-05T08:15:00Z) starts with the year; "Tue, 06 Oct ..." only contains a T
    if let Some((date, time)) = raw.split_once('T').filter(|_| raw.starts_with(|c: char| c.is_ascii_digit())) {
        return format!("{date} {}", time.chars().take(5).collect::<String>());
    }
    let mut parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() >= 6 {
        parts.pop();
    }
    let mut s = parts.join(" ");
    if s.len() > 3 && s.as_bytes()[s.len() - 3] == b':' {
        // already HH:MM
    } else if let Some(i) = s.rfind(':') {
        if s.len() >= i + 3 {
            s.truncate(i);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_items_parse() {
        let xml = r#"<rss><channel><title>RUV</title><item><title><![CDATA[Fr&eacute;tt &amp; fleira]]></title><link>https://ruv.is/a</link>
            <pubDate>Mon, 05 Oct 2026 08:15:00 +0000</pubDate><description>&lt;p&gt;Texti &#243;l&lt;/p&gt;</description></item></channel></rss>"#;
        let items = parse(xml, "RUV");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].link, "https://ruv.is/a");
        assert_eq!(items[0].date, "Mon, 05 Oct 2026 08:15:00");
        assert_eq!(items[0].body, "Texti \u{f3}l");
        assert_eq!(items[0].source, "RUV");
    }

    #[test]
    fn atom_entries_parse() {
        let xml = r#"<feed><title>x</title><entry><title>Hello</title><link rel="alternate" href="https://e.is/1"/><updated>2026-10-05T08:15:00Z</updated><summary>Sum</summary></entry></feed>"#;
        let items = parse(xml, "E");
        assert_eq!(items[0].link, "https://e.is/1");
        assert_eq!(items[0].date, "2026-10-05 08:15");
        assert_eq!(items[0].body, "Sum");
    }

    #[test]
    fn images_come_from_enclosures_media_and_img_tags() {
        let rss = |inner: &str| format!("<rss><item><title>T</title>{inner}</item></rss>");
        let img = |inner: &str| parse(&rss(inner), "S")[0].image.clone();
        assert_eq!(img(r#"<enclosure length="1" type="image/jpeg" url="https://myndir.ruv.is/abc="/>"#), "https://myndir.ruv.is/abc=");
        assert_eq!(img(r#"<enclosure type="audio/mpeg" url="https://x/a.mp3"/><media:thumbnail url="https://x/t.jpg"/>"#), "https://x/t.jpg");
        assert_eq!(img(r#"<description>&lt;img style="m" align="left" alt="K."     src="https://c.arvakur.is/1.jpg"/&gt; Texti</description>"#), "https://c.arvakur.is/1.jpg");
        assert_eq!(img("<description>Texti</description>"), "");
        assert_eq!(parse(&rss(r#"<description>&lt;img src="https://c/1.jpg"/&gt; Texti</description>"#), "S")[0].body, "Texti");
    }

    #[test]
    fn soft_hyphens_and_zero_width_marks_are_dropped() {
        assert_eq!(clean("raf\u{ad}magn hand&shy;t\u{f6}ku flug&#173;st\u{f6}\u{f0}inni\u{200b}"), "rafmagn handt\u{f6}ku flugst\u{f6}\u{f0}inni");
    }

    #[test]
    fn rfc822_dates_on_tuesdays_and_thursdays_keep_their_day() {
        assert_eq!(short_date("Tue, 06 Oct 2026 00:05:12 Z"), "Tue, 06 Oct 2026 00:05:12");
        assert_eq!(short_date("Thu, 08 Oct 2026 10:00:00 +0000"), "Thu, 08 Oct 2026 10:00:00");
    }

    #[test]
    fn garbage_gives_nothing() {
        assert!(parse("<item><title>", "x").is_empty());
        assert!(parse("", "x").is_empty());
    }
}
