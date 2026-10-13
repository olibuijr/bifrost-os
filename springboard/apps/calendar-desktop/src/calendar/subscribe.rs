//! Calendar subscriptions: a calendar published at an address (`webcal://` or `https://`), read on a schedule into a
//! read-only calendar beside the saved events. The list of subscriptions is kept beside the events file; each one's
//! last answer is cached next to it, so the calendar shows at once when the app starts and stays after a failed update.
use super::ics;
use super::Calendar;
use bifrost_app::dav_client::session_send;
use bifrost_app::url::URLRequest;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How often a subscription is read while the app runs (milliseconds of the app clock).
pub const EVERY_MS: f64 = 60.0 * 60.0 * 1000.0;
/// Most subscriptions the list keeps, and the longest address and name.
pub const MAX_SUBSCRIPTIONS: usize = 20;
const MAX_URL: usize = 1000;
const MAX_NAME: usize = 80;
const HEADER: &str = "springboard-calendar-subscriptions-v1";

#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    /// The address read, always `https://` or `http://`.
    pub url: String,
    /// The name in the calendar list: what the calendar calls itself, until it has been read the address's host.
    pub name: String,
    /// Whether its events are shown.
    pub shown: bool,
    /// The last calendar text read (empty before the first read).
    pub text: String,
    /// The app clock when the last read began; not saved.
    pub last: Option<f64>,
}

/// The address a pasted or opened link names: `webcal://` and `webcals://` become `https://`, `http://` and
/// `https://` stay. None for anything else, or an address with spaces.
pub fn normalize_url(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let lower = line.to_ascii_lowercase();
    let rest = if lower.starts_with("webcals://") {
        format!("https://{}", &line["webcals://".len()..])
    } else if lower.starts_with("webcal://") {
        format!("https://{}", &line["webcal://".len()..])
    } else if lower.starts_with("https://") || lower.starts_with("http://") {
        line.to_string()
    } else {
        return None;
    };
    let host_ok = rest.split_once("://").is_some_and(|(_, r)| !r.is_empty() && !r.starts_with('/'));
    (host_ok && rest.len() <= MAX_URL && !rest.chars().any(|c| c.is_whitespace() || c.is_control())).then_some(rest)
}

/// Whether a link opened by the system is a calendar subscription.
pub fn is_calendar_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("webcal://") || lower.starts_with("webcals://")
}

/// The host of an address, for a name before the calendar has told its own.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
}

fn clean_name(name: &str) -> String {
    let flat: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    flat.trim().chars().take(MAX_NAME).collect()
}

/// The subscriptions file beside the events file.
pub fn list_path(events: &Path) -> PathBuf {
    events.with_file_name("calendar-subscriptions.txt")
}

/// Where the last answer of the subscription at `url` is cached: a name made from the address.
pub fn cache_path(events: &Path, url: &str) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        hash = (hash ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    events.with_file_name(format!("calendar-subscription-{hash:016x}.ics"))
}

/// The list as text: a header, then one line per subscription: address, name, 1 or 0 for shown.
pub fn list_text(subs: &[Subscription]) -> String {
    let mut out = format!("{HEADER}\n");
    for s in subs {
        out.push_str(&format!("{}\t{}\t{}\n", s.url, clean_name(&s.name).replace('\t', " "), u8::from(s.shown)));
    }
    out
}

/// The list from its text. A line that is not an address and a name is skipped; so is anything past the limit.
pub fn parse_list(text: &str) -> Vec<Subscription> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return vec![];
    }
    lines
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let url = normalize_url(parts.next()?)?;
            let name = clean_name(parts.next()?);
            let shown = parts.next()? != "0";
            Some(Subscription { name: if name.is_empty() { host_of(&url) } else { name }, url, shown, text: String::new(), last: None })
        })
        .take(MAX_SUBSCRIPTIONS)
        .collect()
}

/// The list saved beside `events`, with each subscription's cached calendar.
pub fn load(events: &Path) -> Vec<Subscription> {
    let mut subs = std::fs::read_to_string(list_path(events)).map(|t| parse_list(&t)).unwrap_or_default();
    for s in &mut subs {
        s.text = std::fs::read(cache_path(events, &s.url)).ok().filter(|b| b.len() <= 16 * 1024 * 1024).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    }
    subs
}

impl Calendar {
    /// Writes the list of subscriptions.
    pub(super) fn save_subscriptions(&self) {
        if let Err(e) = springboard::shell::fsutil::write_atomic(&list_path(&self.path), &list_text(&self.subs)) {
            eprintln!("springboard: calendar: {e}");
        }
    }

    /// The names of the subscriptions, in the order of the calendar list.
    pub fn subscription_names(&self) -> Vec<String> {
        self.subs.iter().map(|s| s.name.clone()).collect()
    }

    /// Subscribes to the calendar at the address in `text` (a `webcal://` or `https://` link). Returns its name, which
    /// is the address's host until the calendar has been read. It is read on the next `next_fetch`.
    pub fn subscribe(&mut self, text: &str) -> Result<String, String> {
        let Some(url) = normalize_url(text) else { return Err("That is not a calendar address (webcal:// or https://)".into()) };
        if self.subs.iter().any(|s| s.url == url) {
            return Err("You are already subscribed to that calendar".into());
        }
        if self.subs.len() >= MAX_SUBSCRIPTIONS {
            return Err("There are too many subscriptions".into());
        }
        let name = host_of(&url);
        self.subs.push(Subscription { url, name: name.clone(), shown: true, text: String::new(), last: None });
        self.save_subscriptions();
        self.overlay_version += 1;
        self.sync_overlay();
        self.dirty = true;
        Ok(name)
    }

    /// Removes the subscription at `index` of the calendar list, and its cached calendar.
    pub fn unsubscribe(&mut self, index: usize) -> bool {
        if index >= self.subs.len() {
            return false;
        }
        let gone = self.subs.remove(index);
        let _ = std::fs::remove_file(cache_path(&self.path, &gone.url));
        self.save_subscriptions();
        self.overlay_version += 1;
        self.sync_overlay();
        self.dirty = true;
        true
    }

    /// The address of the next subscription that is due to be read (never read, or read an hour ago), marked as read
    /// now so it is not offered again until it is due.
    pub fn next_fetch(&mut self, t: f64) -> Option<String> {
        let s = self.subs.iter_mut().find(|s| s.last.map_or(true, |last| t - last >= EVERY_MS))?;
        s.last = Some(t);
        Some(s.url.clone())
    }

    /// When the next subscription falls due, on the app clock.
    pub fn fetch_wake(&self) -> Option<f64> {
        self.subs.iter().map(|s| s.last.map_or(0.0, |last| last + EVERY_MS)).reduce(f64::min)
    }

    /// The answer of a read of `url`: the calendar text, or why it failed. A good answer replaces the cached calendar
    /// and shows the name the calendar gives itself; a failure is shown and the cached calendar stays.
    pub fn fetch_finished(&mut self, url: &str, result: Result<String, String>) {
        let Some(i) = self.subs.iter().position(|s| s.url == url) else { return };
        let name = self.subs[i].name.clone();
        let text = result.and_then(|text| if ics::parse(&text).events.is_empty() && !text.contains("BEGIN:VCALENDAR") { Err("it is not a calendar".to_string()) } else { Ok(text) });
        match text {
            Ok(text) => {
                let called = clean_name(&ics::parse(&text).name);
                let s = &mut self.subs[i];
                if !called.is_empty() {
                    s.name = called;
                }
                s.text = text;
                if let Err(e) = springboard::shell::fsutil::write_atomic(&cache_path(&self.path, url), &self.subs[i].text) {
                    eprintln!("springboard: calendar: {e}");
                }
                self.save_subscriptions();
                self.overlay_version += 1;
                self.sync_overlay();
                self.dirty = true;
            }
            Err(e) => self.show_error(&format!("Could not update {name}: {e}")),
        }
    }
}

/// Reads subscriptions on the background queue, one at a time, and hands each answer to the calendar. A worker puts its
/// answer in the shared list and wakes the app loop through the main queue, so `tick` takes it without polling.
#[derive(Default)]
pub struct Fetcher {
    answers: Arc<Mutex<Vec<(String, Result<String, String>)>>>,
    /// The address being read.
    running: Option<String>,
}

impl Fetcher {
    /// Puts an answer in the list `poll` takes from: what a worker does when its read ends.
    pub fn post(&self, url: &str, result: Result<String, String>) {
        self.answers.lock().unwrap_or_else(|e| e.into_inner()).push((url.to_string(), result));
    }

    /// Hands the calendar the answers that have come in.
    pub fn deliver(&mut self, c: &mut Calendar) {
        let answers = std::mem::take(&mut *self.answers.lock().unwrap_or_else(|e| e.into_inner()));
        for (url, result) in answers {
            if self.running.as_deref() == Some(url.as_str()) {
                self.running = None;
            }
            c.fetch_finished(&url, result);
        }
    }

    /// Call from `tick` with the app clock: takes the answers that came in, then starts the next read that is due.
    pub fn poll(&mut self, c: &mut Calendar, t: f64) {
        self.deliver(c);
        if self.running.is_some() {
            return;
        }
        let Some(url) = c.next_fetch(t) else { return };
        let answers = Arc::clone(&self.answers);
        let target = url.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let result = read(&target);
            answers.lock().unwrap_or_else(|e| e.into_inner()).push((target, result));
            // an empty main-queue job wakes the app loop, which then runs `tick`
            DispatchQueue::main().async_(|| {});
        });
        self.running = Some(url);
    }

    /// When `tick` next has work for subscriptions: when the next one falls due. A read going on wakes the loop itself.
    pub fn wake(&self, c: &Calendar) -> Option<f64> {
        if self.running.is_some() {
            return None;
        }
        c.fetch_wake()
    }
}

/// Reads the calendar at `url`.
fn read(url: &str) -> Result<String, String> {
    let (status, body) = session_send(&URLRequest::new(url))?;
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(format!("the server answered {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webcal_links_become_https_addresses() {
        assert_eq!(normalize_url("webcal://example.is/cal.ics"), Some("https://example.is/cal.ics".into()));
        assert_eq!(normalize_url("WEBCALS://example.is/cal.ics"), Some("https://example.is/cal.ics".into()));
        assert_eq!(normalize_url("  https://example.is/a.ics\nignored line"), Some("https://example.is/a.ics".into()));
        assert_eq!(normalize_url("\n\nhttp://example.is/a.ics"), Some("http://example.is/a.ics".into()));
        for bad in ["", "   ", "example.is/a.ics", "ftp://example.is/a.ics", "webcal://", "https:///x", "https://exa mple.is/a.ics", "mailto:a@b.is"] {
            assert_eq!(normalize_url(bad), None, "{bad:?}");
        }
        assert_eq!(normalize_url(&format!("https://example.is/{}", "a".repeat(2000))), None);
    }

    #[test]
    fn calendar_links_are_recognised() {
        assert!(is_calendar_link("webcal://example.is/a.ics"));
        assert!(is_calendar_link(" WebCals://example.is/a.ics"));
        assert!(!is_calendar_link("https://example.is/a.ics"));
        assert!(!is_calendar_link("file:///tmp/a.ics"));
    }

    #[test]
    fn a_host_names_the_calendar_before_it_is_read() {
        assert_eq!(host_of("https://cal.example.is/team/a.ics?x=1"), "cal.example.is");
        assert_eq!(host_of("https://example.is"), "example.is");
        assert_eq!(host_of("example.is/x"), "example.is");
    }

    #[test]
    fn the_list_round_trips_and_ignores_damaged_lines() {
        let subs = vec![
            Subscription { url: "https://a.is/x.ics".into(), name: "Team".into(), shown: true, text: "ignored".into(), last: Some(5.0) },
            Subscription { url: "https://b.is/y.ics".into(), name: "Tab\tbed".into(), shown: false, text: String::new(), last: None },
        ];
        let text = list_text(&subs);
        let back = parse_list(&text);
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].url.as_str(), back[0].name.as_str(), back[0].shown, back[0].text.as_str(), back[0].last), ("https://a.is/x.ics", "Team", true, "", None));
        assert_eq!((back[1].name.as_str(), back[1].shown), ("Tab bed", false));
        let damaged = format!("{HEADER}\nnot a line\nftp://a.is/x\tName\t1\nhttps://c.is/z.ics\t\t1\nhttps://d.is/w.ics\tD\n");
        let got = parse_list(&damaged);
        assert_eq!(got.len(), 1, "only the line with an address, a name field and a flag");
        assert_eq!(got[0].name, "c.is", "an empty name falls back to the host");
        assert!(parse_list("wrong header\nhttps://a.is/x.ics\tA\t1\n").is_empty());
    }

    #[test]
    fn the_list_is_limited() {
        let mut text = format!("{HEADER}\n");
        for i in 0..MAX_SUBSCRIPTIONS + 5 {
            text.push_str(&format!("https://h{i}.is/c.ics\tC{i}\t1\n"));
        }
        assert_eq!(parse_list(&text).len(), MAX_SUBSCRIPTIONS);
    }

    #[test]
    fn each_address_has_its_own_cache_file_beside_the_events() {
        let events = Path::new("/data/events.tsv");
        let a = cache_path(events, "https://a.is/x.ics");
        assert_eq!(a.parent(), Some(Path::new("/data")));
        assert_ne!(a, cache_path(events, "https://b.is/x.ics"));
        assert_eq!(a, cache_path(events, "https://a.is/x.ics"));
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("calendar-subscription-"));
        assert_eq!(list_path(events), PathBuf::from("/data/calendar-subscriptions.txt"));
    }
}
