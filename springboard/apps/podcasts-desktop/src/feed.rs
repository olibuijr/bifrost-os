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
fn attribute(xml: &str, tag: &str, attr: &str) -> Option<String> {
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
