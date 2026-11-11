//! A tolerant RSS 2.0 / Atom reader: no XML crate, just enough scanning to
//! pull items (title, link, date, body) out of the feeds Icelandic news
//! sites publish. Malformed input yields fewer items, never a panic.

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Item {
    pub title: String,
    pub link: String,
    /// when the feed dated it, as `stored_date` keeps it: in UTC whenever the feed wrote its zone
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
    let date = ["pubDate", "published", "updated", "dc:date"].iter().find_map(|t| inner(block, t)).map(|d| stored_date(&clean(&d))).unwrap_or_default();
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

/// The zone a feed wrote after a date, in minutes east of UTC: "+0200", "-05:00", "Z", "GMT" or "UT" at the end of an
/// RFC 822 date, or after the time of an ISO 8601 one. None when the date names no zone this reads (a zone
/// abbreviation such as "EST" is not read).
fn zone_minutes(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let tail = if raw.starts_with(|c: char| c.is_ascii_digit()) && raw.as_bytes().get(4) == Some(&b'-') && raw.contains('T') {
        let time = raw.split_once('T')?.1;
        &time[time.find(['+', '-', 'Z', 'z'])?..]
    } else {
        raw.split_whitespace().last()?
    };
    if ["Z", "GMT", "UT", "UTC"].iter().any(|z| tail.eq_ignore_ascii_case(z)) {
        return Some(0);
    }
    let sign = match tail.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = tail[1..].chars().filter(|c| *c != ':').collect();
    if !(digits.len() == 2 || digits.len() == 4) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = if digits.len() == 4 { digits[2..].parse().ok()? } else { 0 };
    Some(sign * (hours * 60 + minutes))
}

/// The date an item keeps: `short_date`, moved to UTC ("2026-10-05 06:15") when the feed wrote a zone other than
/// UTC, so every kept date is one instant however its feed wrote it. A date with no zone is taken as UTC.
pub fn stored_date(raw: &str) -> String {
    let short = short_date(raw);
    match (zone_minutes(raw), minutes_of(&short)) {
        (Some(zone), Some(minutes)) if zone != 0 && short.contains(':') => {
            let utc = minutes - zone;
            let (y, m, d) = bifrost_app::calendar_system::civil_from_days(utc.div_euclid(1440));
            let minute = utc.rem_euclid(1440);
            format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", minute / 60, minute % 60)
        }
        _ => short,
    }
}

/// An item's date as a row and the article show it: the day and the time in the system's zone, the time as the
/// user's clocks write it ("Today at 08:15", "5 Oct 2026 at 8:15 AM"). `now` is Unix seconds.
pub fn date_text(date: &str, now: i64) -> String {
    // each instant at the zone's offset then: an article from before a daylight saving change keeps its time
    let offset = |t: i64| bifrost_app::time_zone::TimeZone::auto_updating_current().seconds_from_gmt(bifrost_app::coding::Date(t as f64));
    let at = minutes_of(date).map_or(now, |m| m * 60);
    date_text_in(date, offset(at), now, offset(now), bifrost_app::time_zone::uses_24_hour_clock())
}

/// `date_text` with the zone's offset from UTC in seconds at the item's instant and at `now`, and a given clock
/// style. A date that cannot be read, or one with no time of day, is shown as the feed wrote it. The text is the
/// shell's (`springboard::files::fs::date_text_in`), which reads both instants at one offset: the item's time is
/// moved by the difference of the two, so its day and time are the ones of its own offset.
pub fn date_text_in(date: &str, offset: i32, now: i64, now_offset: i32, h24: bool) -> String {
    match minutes_of(date) {
        Some(m) if date.contains(':') => springboard::files::fs::date_text_in(m * 60 + i64::from(offset) - i64::from(now_offset), now, now_offset, h24),
        _ => date.to_string(),
    }
}

/// Minutes since 1970-01-01 of an item's date (`Mon, 26 Oct 2026 03:43` or `2026-10-26 03:43`, in UTC as
/// `stored_date` keeps it), or None when the date is missing or unreadable.
pub fn minutes_of(date: &str) -> Option<i64> {
    use bifrost_app::calendar_system::days_from_civil;
    let (y, m, d, time) = if date.starts_with(|c: char| c.is_ascii_digit()) && date.as_bytes().get(4) == Some(&b'-') {
        let (day, time) = date.split_once(' ').unwrap_or((date, ""));
        let mut p = day.split('-');
        (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?, time)
    } else {
        let parts: Vec<&str> = date.split_whitespace().map(|p| p.trim_end_matches(',')).collect();
        let at = parts.iter().position(|p| p.parse::<i64>().is_ok_and(|n| (1..=31).contains(&n)))?;
        let month = parts.get(at + 1)?;
        let m = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|n| month.to_ascii_lowercase().starts_with(n))? as i64 + 1;
        (parts.get(at + 2)?.parse().ok()?, m, parts[at].parse().ok()?, parts.get(at + 3).copied().unwrap_or(""))
    };
    let mut t = time.split(':');
    let hour: i64 = t.next().and_then(|h| h.parse().ok()).unwrap_or(0);
    let minute: i64 = t.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some(days_from_civil(y, m, d) * 1440 + hour * 60 + minute)
}

/// Today's list: every feed's items dated on `day` (days since 1970-01-01, in a zone `offset` minutes east of UTC),
/// newest first across the sources. Items with no readable date follow the dated ones, in feed order.
pub fn today_list(feeds: &[Vec<Item>], day: i64, offset: i64) -> Vec<Item> {
    let mut dated: Vec<(i64, &Item)> = vec![];
    let mut undated: Vec<&Item> = vec![];
    for item in feeds.iter().flatten() {
        match minutes_of(&item.date) {
            Some(m) if (m + offset).div_euclid(1440) == day => dated.push((m, item)),
            Some(_) => {}
            None => undated.push(item),
        }
    }
    dated.sort_by(|a, b| b.0.cmp(&a.0));
    dated.into_iter().map(|(_, i)| i).chain(undated).cloned().collect()
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

    #[test]
    fn minutes_read_both_date_styles() {
        assert_eq!(minutes_of("Mon, 26 Oct 2026 03:43"), minutes_of("2026-10-26 03:43"));
        assert_eq!(minutes_of("2026-10-26 03:43").map(|m| m - minutes_of("2026-10-26 03:42").unwrap()), Some(1));
        assert_eq!(minutes_of(""), None);
        assert_eq!(minutes_of("soon"), None);
    }

    #[test]
    fn today_is_newest_first_and_leaves_out_other_days() {
        let item = |date: &str, source: &str| Item { title: date.into(), date: date.into(), source: source.into(), ..Item::default() };
        let feeds = vec![
            vec![item("Mon, 26 Oct 2026 03:43", "A"), item("Sun, 25 Oct 2026 21:48", "A"), item("", "A")],
            vec![item("Mon, 26 Oct 2026 02:21", "B"), item("2026-10-26 05:00", "B")],
        ];
        let day = minutes_of("2026-10-26 00:00").unwrap() / 1440;
        let titles: Vec<String> = today_list(&feeds, day, 0).into_iter().map(|i| i.title).collect();
        assert_eq!(titles, ["2026-10-26 05:00", "Mon, 26 Oct 2026 03:43", "Mon, 26 Oct 2026 02:21", ""]);
        // three hours east of UTC the late evening before is already that day
        let east: Vec<String> = today_list(&feeds, day, 180).into_iter().map(|i| i.title).collect();
        assert_eq!(east, ["2026-10-26 05:00", "Mon, 26 Oct 2026 03:43", "Mon, 26 Oct 2026 02:21", "Sun, 25 Oct 2026 21:48", ""]);
        // and three hours west the small hours of that day still belong to the day before
        let west: Vec<String> = today_list(&feeds, day - 1, -180).into_iter().map(|i| i.title).collect();
        assert_eq!(west, ["Mon, 26 Oct 2026 02:21", "Sun, 25 Oct 2026 21:48", ""]);
    }

    #[test]
    fn kept_dates_are_in_utc_when_the_feed_wrote_its_zone() {
        assert_eq!(stored_date("Mon, 05 Oct 2026 08:15:00 +0000"), "Mon, 05 Oct 2026 08:15:00");
        assert_eq!(stored_date("Mon, 05 Oct 2026 08:15:00 GMT"), "Mon, 05 Oct 2026 08:15:00");
        assert_eq!(stored_date("Mon, 05 Oct 2026 08:15:00 +0200"), "2026-10-05 06:15");
        assert_eq!(stored_date("Mon, 05 Oct 2026 22:15:00 -0500"), "2026-10-06 03:15");
        assert_eq!(stored_date("2026-10-05T08:15:00Z"), "2026-10-05 08:15");
        assert_eq!(stored_date("2026-10-05T00:15:00+02:00"), "2026-10-04 22:15");
        assert_eq!(stored_date("2026-10-05T08:15:00-0330"), "2026-10-05 11:45");
        // no zone, a zone this does not read, and a date with no time stay as the feed wrote them
        assert_eq!(stored_date("2026-10-05T08:15:00"), "2026-10-05 08:15");
        assert_eq!(stored_date("Mon, 05 Oct 2026 08:15:00 EST"), "Mon, 05 Oct 2026 08:15:00");
        assert_eq!(stored_date("2026-10-05"), "2026-10-05");
        assert_eq!(stored_date("soon"), "soon");
    }

    #[test]
    fn dates_are_shown_in_the_clock_style_and_the_zone() {
        // read on 2026-10-06 at 12:00 UTC
        let now = minutes_of("2026-10-06 12:00").unwrap() * 60;
        let text = |date: &str, h24: bool| date_text_in(date, 0, now, 0, h24);
        // morning, noon, afternoon and midnight, in both styles
        assert_eq!(text("Tue, 06 Oct 2026 08:15:00", true), "Today at 08:15");
        assert_eq!(text("Tue, 06 Oct 2026 08:15:00", false), "Today at 8:15 AM");
        assert_eq!(text("2026-10-06 12:00", true), "Today at 12:00");
        assert_eq!(text("2026-10-06 12:00", false), "Today at 12:00 PM");
        assert_eq!(text("2026-10-05 16:30", true), "Yesterday at 16:30");
        assert_eq!(text("2026-10-05 16:30", false), "Yesterday at 4:30 PM");
        assert_eq!(text("Sat, 03 Oct 2026 00:00:00", true), "3 Oct 2026 at 00:00");
        assert_eq!(text("Sat, 03 Oct 2026 00:00:00", false), "3 Oct 2026 at 12:00 AM");
        // two hours east the late evening is the next day, which is today there
        assert_eq!(date_text_in("2026-10-05 23:30", 7200, now, 7200, true), "Today at 01:30");
        // an article from before a change of the offset keeps the time of its own offset
        assert_eq!(date_text_in("2026-10-03 10:00", 7200, now, 3600, true), "3 Oct 2026 at 12:00");
        // nothing readable, or no time of day: the feed's own text
        assert_eq!(text("", true), "");
        assert_eq!(text("soon", false), "soon");
        assert_eq!(text("2026-10-05", false), "2026-10-05");
    }
}
