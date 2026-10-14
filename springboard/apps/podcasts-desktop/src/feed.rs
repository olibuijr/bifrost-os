//! RSS parsing and the subscription store for Podcasts. The parser is a small
//! tag scanner (feeds are regular enough); the store is a tab-separated file.

use springboard::shell::prefs::write_atomic;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Episode {
    pub title: String,
    pub url: String,
    pub date: String,
    pub duration: String,
    /// Release time, seconds since 1970 (0 when the feed gave none or it did not parse); orders episodes across shows.
    pub ts: i64,
    /// The episode notes as plain text.
    pub notes: String,
    /// Chapter marks from the feed, in start order.
    pub chapters: Vec<Chapter>,
    /// The address of a JSON file holding the chapters, empty when the feed names none (the feed's own marks win).
    pub chapters_url: String,
    /// The address of the transcript file, empty when the feed names none.
    pub transcript: String,
}

/// One chapter mark of an episode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub start_ms: u64,
    pub title: String,
}

/// The chapter playing at `position_ms`: the last one that has started. None before the first.
pub fn current_chapter(chapters: &[Chapter], position_ms: u64) -> Option<usize> {
    chapters.iter().rposition(|c| c.start_ms <= position_ms)
}

#[derive(Clone, Debug, Default)]
pub struct Show {
    pub url: String,
    pub title: String,
    pub episodes: Vec<Episode>,
}

/// FNV-1a over the URL: a stable file stem.
pub fn stem(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Adds a scheme when the user typed a bare host.
pub fn normalize(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let url = if t.starts_with("http://") || t.starts_with("https://") { t.to_string() } else { format!("https://{t}") };
    url.contains('.').then_some(url)
}

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = match s.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")) {
        Some(inner) => return inner.trim().to_string(),
        None => s,
    };
    decode_entities(s)
}

/// Replaces the named and numeric (`&#8217;`, `&#x27;`) entities; `&amp;` goes last
/// of the named ones so `&amp;lt;` stays `&lt;`.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "amp" => Some('&'),
            _ => name
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse().ok(),
                })
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

/// The text of the first `<tag ...>text</tag>` in `xml`.
fn element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = xml[from..].find(&open) {
        let start = from + i + open.len();
        match xml[start..].chars().next() {
            Some('>') | Some(' ') | Some('\n') | Some('\t') => {
                let gt = xml[start..].find('>')? + start;
                if xml[..gt].ends_with('/') {
                    return Some(String::new());
                }
                let end = xml[gt..].find(&format!("</{tag}>"))? + gt;
                return Some(unescape(&xml[gt + 1..end]));
            }
            _ => from = start,
        }
    }
    None
}

/// An attribute of the first `<tag ...>` in `xml`.
pub(crate) fn attribute(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let i = xml.find(&format!("<{tag} "))?;
    let end = xml[i..].find('>')? + i;
    let head = &xml[i..end];
    for quote in ['"', '\''] {
        let key = format!("{attr}={quote}");
        if let Some(k) = head.find(&key) {
            let v = &head[k + key.len()..];
            return v.find(quote).map(|e| unescape(&v[..e]));
        }
    }
    None
}

/// "Mon, 05 Oct 2026 10:00:00 +0000" -> "05 Oct 2026".
fn short_date(raw: &str) -> String {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    match parts.as_slice() {
        [d, day, mon, year, ..] if d.ends_with(',') => format!("{day} {mon} {year}"),
        [day, mon, year, ..] => format!("{day} {mon} {year}"),
        _ => raw.to_string(),
    }
}

/// "3725" or "01:02:05" -> "1:02:05".
fn duration(raw: &str) -> String {
    let secs = if raw.contains(':') {
        raw.split(':').try_fold(0u64, |a, p| p.trim().parse::<u64>().ok().map(|v| a * 60 + v))
    } else {
        raw.trim().parse::<u64>().ok()
    };
    secs.map_or_else(String::new, |s| springboard::shell::music::nowplaying::clock(s as f64 * 1000.0))
}

/// "90", "01:30" or "00:01:30.500" (also with a comma before the fraction) -> milliseconds.
pub fn clock_ms(raw: &str) -> Option<u64> {
    let raw = raw.trim().replace(',', ".");
    if raw.is_empty() {
        return None;
    }
    let mut secs = 0.0f64;
    for part in raw.split(':') {
        let v: f64 = part.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)?;
        secs = secs * 60.0 + v;
    }
    Some((secs * 1000.0).round() as u64)
}

/// Days from 1970-01-01 to the civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "Mon, 05 Oct 2026 10:00:00 +0000" -> seconds since 1970. Zones as "+0100", "GMT" and "UT" are read.
pub fn rfc2822_ts(raw: &str) -> Option<i64> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let parts = if parts.first().is_some_and(|p| p.ends_with(',')) { &parts[1..] } else { &parts[..] };
    let [day, mon, year, rest @ ..] = parts else { return None };
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| mon.to_ascii_lowercase().starts_with(m))? as i64
        + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let mut clock = rest.first().map_or(Some(0), |t| clock_ms(t).map(|ms| (ms / 1000) as i64))?;
    if let Some(zone) = rest.get(1) {
        if let (Some(sign), true) = (zone.chars().next().filter(|c| *c == '+' || *c == '-'), zone.len() == 5) {
            let n: i64 = zone[1..].parse().ok()?;
            let off = (n / 100) * 3600 + (n % 100) * 60;
            clock -= if sign == '+' { off } else { -off };
        }
    }
    Some(days_from_civil(year, month, day) * 86_400 + clock)
}

/// Turns the HTML of a feed description into plain text: tags go, paragraph and line breaks become line breaks,
/// entities are decoded, and runs of blank lines collapse to one. A link keeps its text and its address in
/// parentheses, unless the text is the address itself.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    // the link being read: where its text starts in `out`, and its address
    let mut link: Option<(usize, String)> = None;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(end) = rest[i..].find('>') else {
            rest = "";
            break;
        };
        let tag = &rest[i..i + end + 1];
        let closing = rest[i + 1..].starts_with('/');
        let name: String = rest[i + 1..i + end].trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if name == "br" || (closing && matches!(name.as_str(), "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4")) {
            out.push('\n');
        }
        if name == "a" {
            if closing {
                if let Some((from, href)) = link.take() {
                    let text = out[from..].trim();
                    if !href.is_empty() && text != href {
                        out.push_str(&format!(" ({href})"));
                    }
                }
            } else {
                link = Some((out.len(), attribute(tag, "a", "href").unwrap_or_default()));
            }
        }
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    let text = decode_entities(&out);
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// The `<psc:chapter start=".." title=".."/>` marks of one item.
fn chapters(item: &str) -> Vec<Chapter> {
    let mut out = Vec::new();
    let mut rest = item;
    while let Some(i) = rest.find("<psc:chapter ") {
        let Some(end) = rest[i..].find('>') else { break };
        let tag = &rest[i..i + end + 1];
        rest = &rest[i + end + 1..];
        let (Some(start), title) = (attribute(tag, "psc:chapter", "start").and_then(|s| clock_ms(&s)), attribute(tag, "psc:chapter", "title")) else { continue };
        out.push(Chapter { start_ms: start, title: title.unwrap_or_default() });
    }
    out.sort_by_key(|c| c.start_ms);
    out
}

/// Parses an RSS document into its title and episodes with audio enclosures.
pub fn parse(xml: &str) -> (String, Vec<Episode>) {
    let first_item = xml.find("<item").unwrap_or(xml.len());
    let title = element(&xml[..first_item], "title").unwrap_or_default();
    let mut episodes = Vec::new();
    let mut rest = &xml[first_item..];
    while let Some(i) = rest.find("<item") {
        let Some(j) = rest[i..].find("</item>") else { break };
        let item = &rest[i..i + j];
        rest = &rest[i + j + 7..];
        let Some(url) = attribute(item, "enclosure", "url") else { continue };
        if url.is_empty() {
            continue;
        }
        episodes.push(Episode {
            title: element(item, "title").unwrap_or_else(|| url.clone()),
            url,
            date: element(item, "pubDate").map(|d| short_date(&d)).unwrap_or_default(),
            duration: element(item, "itunes:duration").map(|d| duration(&d)).unwrap_or_default(),
            ts: element(item, "pubDate").and_then(|d| rfc2822_ts(&d)).unwrap_or(0),
            notes: ["content:encoded", "itunes:summary", "description"]
                .iter()
                .find_map(|tag| element(item, tag).filter(|t| !t.is_empty()))
                .map(|t| html_to_text(&t))
                .unwrap_or_default(),
            chapters: chapters(item),
            chapters_url: attribute(item, "podcast:chapters", "url").unwrap_or_default(),
            transcript: attribute(item, "podcast:transcript", "url").unwrap_or_default(),
        });
    }
    (title, episodes)
}

#[derive(Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn subs_file(&self) -> PathBuf {
        self.dir.join("subscriptions.tsv")
    }

    pub fn feed_file(&self, url: &str) -> PathBuf {
        self.dir.join("feeds").join(format!("{}.xml", stem(url)))
    }

    pub fn episode_file(&self, url: &str) -> PathBuf {
        self.dir.join("episodes").join(format!("{}.mp3", stem(url)))
    }

    pub fn load(&self) -> Vec<Show> {
        let text = std::fs::read_to_string(self.subs_file()).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let (url, title) = l.split_once('\t')?;
                Some(Show { url: url.to_string(), title: title.to_string(), episodes: Vec::new() })
            })
            .collect()
    }

    pub fn save(&self, shows: &[Show]) {
        let text: String = shows.iter().map(|s| format!("{}\t{}\n", s.url, s.title.replace(['\t', '\n'], " "))).collect();
        let _ = write_atomic(&self.subs_file(), text.as_bytes());
    }

    fn pos_file(&self) -> PathBuf {
        self.dir.join("positions.tsv")
    }

    /// Resume positions (ms) per episode URL.
    pub fn positions(&self) -> Vec<(String, u64)> {
        let text = std::fs::read_to_string(self.pos_file()).unwrap_or_default();
        text.lines().filter_map(|l| l.split_once('\t').and_then(|(u, p)| Some((u.to_string(), p.parse().ok()?)))).collect()
    }

    pub fn save_positions(&self, pos: &[(String, u64)]) {
        let text: String = pos.iter().map(|(u, p)| format!("{u}\t{p}\n")).collect();
        let _ = write_atomic(&self.pos_file(), text.as_bytes());
    }

    pub fn cached(&self, url: &str) -> Option<(String, Vec<Episode>)> {
        let xml = std::fs::read_to_string(self.feed_file(url)).ok()?;
        Some(parse(&xml))
    }

    pub fn cache(&self, url: &str, xml: &str) {
        let _ = write_atomic(&self.feed_file(url), xml.as_bytes());
    }

    pub fn forget(&self, url: &str) {
        let _ = std::fs::remove_file(self.feed_file(url));
    }

    pub fn downloaded(&self, url: &str) -> bool {
        self.episode_file(url).is_file()
    }

    /// The partial file an episode download streams into.
    pub fn part_file(&self, url: &str) -> PathBuf {
        self.episode_file(url).with_extension("part")
    }

    /// Deletes the downloaded files of `urls` (and any half-finished download).
    pub fn forget_episodes<'a>(&self, urls: impl IntoIterator<Item = &'a str>) {
        for url in urls {
            let _ = std::fs::remove_file(self.episode_file(url));
            let _ = std::fs::remove_file(self.part_file(url));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_items_with_enclosures() {
        let xml = r#"<rss><channel><title>Show &amp; Tell</title>
<item><title><![CDATA[Ep 1]]></title><pubDate>Mon, 05 Oct 2026 10:00:00 +0000</pubDate><itunes:duration>3725</itunes:duration><enclosure url="https://x.test/1.mp3" type="audio/mpeg"/></item>
<item><title>No audio</title></item></channel></rss>"#;
        let (title, eps) = parse(xml);
        assert_eq!(title, "Show & Tell");
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].title, "Ep 1");
        assert_eq!(eps[0].date, "05 Oct 2026");
        assert_eq!(eps[0].duration, "1:02:05");
    }

    #[test]
    fn clocks_and_dates_become_numbers() {
        assert_eq!(clock_ms("90"), Some(90_000));
        assert_eq!(clock_ms("01:30"), Some(90_000));
        assert_eq!(clock_ms("00:01:30.500"), Some(90_500));
        assert_eq!(clock_ms("00:00:01,250"), Some(1_250));
        assert_eq!(clock_ms("x"), None);
        assert_eq!(clock_ms(""), None);
        assert_eq!(rfc2822_ts("Thu, 01 Jan 1970 00:00:01 +0000"), Some(1));
        assert_eq!(rfc2822_ts("Sat, 01 Jan 2000 00:00:00 GMT"), Some(946_684_800));
        assert_eq!(rfc2822_ts("Sat, 01 Jan 2000 01:00:00 +0100"), Some(946_684_800));
        assert_eq!(rfc2822_ts("Fri, 31 Dec 1999 19:00:00 -0500"), Some(946_684_800));
        let (a, b) = (rfc2822_ts("Mon, 05 Oct 2026 10:00:00 +0000").unwrap(), rfc2822_ts("Sun, 04 Oct 2026 10:00:00 +0000").unwrap());
        assert_eq!(a - b, 86_400);
        assert_eq!(rfc2822_ts("sometime"), None);
    }

    #[test]
    fn notes_chapters_and_transcripts_are_read_from_items() {
        let xml = r#"<rss><channel><title>S</title>
<item><title>E</title><pubDate>Sat, 01 Jan 2000 00:00:00 GMT</pubDate>
<description><![CDATA[<p>First &amp; second</p><p>Third<br/>line</p>]]></description>
<podcast:transcript url="https://x.test/t.vtt" type="text/vtt"/>
<podcast:chapters url="https://x.test/c.json" type="application/json+chapters"/>
<psc:chapter start="00:01:00" title="Later"/><psc:chapter start="0" title="Intro"/>
<enclosure url="https://x.test/1.mp3"/></item></channel></rss>"#;
        let (_, eps) = parse(xml);
        assert_eq!(eps[0].ts, 946_684_800);
        assert_eq!(eps[0].notes, "First & second\nThird\nline");
        assert_eq!(eps[0].transcript, "https://x.test/t.vtt");
        assert_eq!(eps[0].chapters_url, "https://x.test/c.json");
        assert_eq!(eps[0].chapters, [Chapter { start_ms: 0, title: "Intro".into() }, Chapter { start_ms: 60_000, title: "Later".into() }]);
    }

    #[test]
    fn the_current_chapter_is_the_last_one_started() {
        let ch = |ms| Chapter { start_ms: ms, title: String::new() };
        let list = [ch(0), ch(60_000), ch(120_000)];
        assert_eq!(current_chapter(&list, 0), Some(0));
        assert_eq!(current_chapter(&list, 59_999), Some(0));
        assert_eq!(current_chapter(&list, 60_000), Some(1));
        assert_eq!(current_chapter(&list, 10_000_000), Some(2));
        assert_eq!(current_chapter(&[ch(5_000)], 1_000), None);
        assert_eq!(current_chapter(&[], 1_000), None);
    }

    #[test]
    fn html_becomes_plain_text() {
        assert_eq!(html_to_text("<b>Hi</b> &lt;you&gt;<br>\n\n\n<ul><li>a</li><li>b</li></ul>"), "Hi <you>\n\na\nb");
        assert_eq!(html_to_text("plain"), "plain");
    }

    #[test]
    fn links_keep_their_address_unless_the_text_is_the_address() {
        assert_eq!(html_to_text(r#"Read <a href="https://x.test/a?b=1&amp;c=2">the guide</a> now"#), "Read the guide (https://x.test/a?b=1&c=2) now");
        assert_eq!(html_to_text(r#"<a href="https://x.test">https://x.test</a>"#), "https://x.test");
        assert_eq!(html_to_text(r#"<a href='https://y.test/p'>Y</a>, <abbr>ok</abbr>"#), "Y (https://y.test/p), ok");
        assert_eq!(html_to_text("<a>bare</a>"), "bare");
    }

    #[test]
    fn numeric_and_named_entities_are_decoded() {
        assert_eq!(unescape("It&#8217;s &#x27;fine&#X27; &amp; &lt;b&gt; &quot;q&quot; &apos;a&apos; &#39;"), "It\u{2019}s 'fine' & <b> \"q\" 'a' '");
        assert_eq!(unescape("&amp;lt; stays"), "&lt; stays", "one pass only");
        assert_eq!(unescape("AT&T &#xZZ; &#99999999; &nosuch; tail &"), "AT&T &#xZZ; &#99999999; &nosuch; tail &");
    }

    #[test]
    fn the_store_writes_through_part_files_and_forgets_episodes() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("test-store-{}", std::process::id()));
        let store = Store::new(dir.clone());
        store.save(&[Show { url: "https://x.test/rss".into(), title: "T".into(), episodes: vec![] }]);
        store.save_positions(&[("https://x.test/1.mp3".into(), 9_000)]);
        store.cache("https://x.test/rss", "<rss/>");
        assert_eq!(store.load().len(), 1);
        assert_eq!(store.positions(), [("https://x.test/1.mp3".to_string(), 9_000)]);
        assert!(store.cached("https://x.test/rss").is_some());
        assert!(!dir.join("subscriptions.part").exists() && !dir.join("positions.part").exists());
        let ep = "https://x.test/1.mp3";
        write_atomic(&store.episode_file(ep), b"mp3").unwrap();
        std::fs::write(store.part_file(ep), b"half").unwrap();
        assert!(store.downloaded(ep));
        store.forget_episodes([ep]);
        assert!(!store.downloaded(ep) && !store.part_file(ep).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize("feeds.x.test/a").as_deref(), Some("https://feeds.x.test/a"));
        assert_eq!(normalize("nonsense"), None);
    }

    #[test]
    fn stems_are_stable_sixteen_hex_digits_per_url() {
        let a = stem("https://x.test/a.xml");
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, stem("https://x.test/a.xml"));
        assert_ne!(a, stem("https://x.test/b.xml"));
    }

    #[test]
    fn enclosure_attributes_accept_single_quotes() {
        let xml = "<enclosure url='https://x.test/a.mp3' length='1'/>";
        assert_eq!(attribute(xml, "enclosure", "url").as_deref(), Some("https://x.test/a.mp3"));
        assert_eq!(attribute(xml, "enclosure", "type"), None);
    }

    #[test]
    fn durations_accept_seconds_or_clock_and_reject_junk() {
        assert_eq!(duration("01:02"), duration("62"));
        assert_eq!(duration("3725"), "1:02:05");
        assert_eq!(duration("not a time"), "");
        assert_eq!(duration("1:x"), "");
    }

    #[test]
    fn unparseable_dates_are_kept_as_they_are() {
        assert_eq!(short_date("05 Oct 2026 10:00:00"), "05 Oct 2026");
        assert_eq!(short_date("sometime"), "sometime");
    }
}
