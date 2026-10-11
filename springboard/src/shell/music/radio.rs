//! The Radio section's data: the station list from spilarinn.is, its disk
//! cache, the logos and the favourites.
//!
//! The site's backend answers `GET /api/stations/` with a JSON object whose
//! `objects` are the stations (name, frequency, stream URL, logos and, where
//! the broadcaster reports it, the song on air). Everything fetched is kept
//! under the Music data directory, so the list and the logos still show
//! with no network; fetching happens on worker threads (see `Music`).

use crate::deildu::art;
use crate::deildu::http::{Client, Tls, Transport};
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

pub const API: &str = "https://bakendi.spilarinn.is/api/stations/";
const SITE: &str = "https://bakendi.spilarinn.is";
/// A cached list younger than this is not fetched again.
pub const FRESH: Duration = Duration::from_secs(60);
const LIST_LIMIT: usize = 2 * 1024 * 1024;
const LOGO_LIMIT: usize = 2 * 1024 * 1024;
const LIST_FILE: &str = "stations.json";
const FAVOURITES_FILE: &str = "favourites.txt";
/// Logo variants in order of preference (the row is 48 pt, the sheet larger).
const LOGO_TYPES: [&str; 3] = ["ios2x", "ios3x", "ios1x"];

/// One live station.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub id: u64,
    pub name: String,
    /// FM frequency in MHz, when it has one
    pub frequency: Option<f64>,
    pub stream_url: String,
    /// path of the logo on the site
    pub logo: Option<String>,
    /// "Artist - Song" as the API reports it
    pub now: Option<String>,
}

impl Station {
    /// "FM 98.9", or "" without a frequency.
    pub fn frequency_text(&self) -> String {
        self.frequency.map_or(String::new(), |f| format!("FM {f:.1}"))
    }
}

/// The stations in an API response; `temp*` placeholders and entries
/// without a stream are left out. None when the text is not the API's.
pub fn parse_stations(text: &str) -> Option<Vec<Station>> {
    let root = json::parse(text)?;
    let objects = root.get("objects")?.array()?;
    let stations = objects
        .iter()
        .filter_map(|o| {
            let name = o.get("name")?.string()?.trim().to_string();
            let stream_url = o.get("stream_url")?.string()?.trim().to_string();
            if name.is_empty() || stream_url.is_empty() || name.to_ascii_lowercase().starts_with("temp") {
                return None;
            }
            let logos = o.get("logos").and_then(json::Json::array).unwrap_or(&[]);
            let logo_url = |kind: &str| logos.iter().find(|l| l.get("type").and_then(json::Json::string) == Some(kind)).and_then(|l| l.get("url")?.string()).map(str::to_string);
            let playing = o.get("playing");
            let text_of = |v: Option<&json::Json>| v.and_then(json::Json::string).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
            let artist = text_of(playing.and_then(|p| p.get("current_artist")).and_then(|a| a.get("name")));
            let song = text_of(playing.and_then(|p| p.get("current_song")).and_then(|s| s.get("name")));
            Some(Station {
                id: o.get("id")?.number()? as u64,
                name,
                frequency: o.get("frequency").and_then(json::Json::number),
                stream_url,
                logo: LOGO_TYPES.iter().find_map(|k| logo_url(k)).or_else(|| logos.first().and_then(|l| l.get("url")?.string()).map(str::to_string)),
                now: match (artist, song) {
                    (Some(a), Some(s)) => Some(format!("{a} - {s}")),
                    (None, Some(s)) => Some(s),
                    (Some(a), None) => Some(a),
                    (None, None) => None,
                },
            })
        })
        .collect();
    Some(stations)
}

/// Downloads bytes; the network seam (tests and previews swap it).
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str, limit: usize) -> io::Result<Vec<u8>>;

    /// `len` bytes from `start`, for downloads too big to hold in memory; callers fall
    /// back to `get` when the fetcher cannot do ranges.
    fn get_range(&self, _url: &str, _start: u64, _len: u64) -> io::Result<Chunk> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "ranged downloads are not supported"))
    }
}

/// One piece of a ranged download.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub bytes: Vec<u8>,
    /// the whole file's size, when the server said
    pub total: Option<u64>,
}

/// The file size in a `Content-Range: bytes 0-99/1234` header.
pub fn content_range_total(header: &str) -> Option<u64> {
    header.rsplit_once('/')?.1.trim().parse().ok()
}

/// The real thing: the Deildu app's HTTPS client, built on first use.
#[derive(Default)]
pub struct Http {
    client: OnceLock<Client>,
}

impl Fetch for Http {
    fn get(&self, url: &str, limit: usize) -> io::Result<Vec<u8>> {
        let client = self.client.get_or_init(|| Client::new(Arc::new(Tls::new()) as Arc<dyn Transport>, Default::default()));
        let r = client.get(url, None, limit)?;
        if r.status != 200 {
            return Err(io::Error::other(format!("{url}: status {}", r.status)));
        }
        Ok(r.body)
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> io::Result<Chunk> {
        let client = self.client.get_or_init(|| Client::new(Arc::new(Tls::new()) as Arc<dyn Transport>, Default::default()));
        let r = client.get(url, Some((start, start + len.max(1) - 1)), len as usize + 4096)?;
        match r.status {
            206 => Ok(Chunk { total: r.header("content-range").and_then(content_range_total), bytes: r.body }),
            // a server that ignores ranges sent everything: only fine when that is what was asked for
            200 if start == 0 => Ok(Chunk { total: Some(r.body.len() as u64), bytes: r.body }),
            s => Err(io::Error::other(format!("{url}: status {s}"))),
        }
    }
}

/// The directory the section keeps its files in.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Store { dir: dir.into() }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }

    /// The saved list and how old it is.
    pub fn load_list(&self) -> Option<(Vec<Station>, Duration)> {
        let path = self.dir.join(LIST_FILE);
        let list = parse_stations(&std::fs::read_to_string(&path).ok()?)?;
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).unwrap_or(Duration::MAX);
        Some((list, age))
    }

    /// Fetches the list; a good answer replaces the saved one.
    pub fn refresh(&self, net: &dyn Fetch) -> io::Result<Vec<Station>> {
        let body = net.get(API, LIST_LIMIT)?;
        let text = String::from_utf8_lossy(&body);
        let list = parse_stations(&text).ok_or_else(|| io::Error::other("unexpected answer from the station list"))?;
        self.write(LIST_FILE, text.as_bytes());
        Ok(list)
    }

    /// The raw logo of a station, from disk or else downloaded and saved.
    /// The file name carries the logo's path, so a changed logo is fetched
    /// again.
    pub fn logo(&self, station: &Station, net: &dyn Fetch) -> Option<Vec<u8>> {
        let path = station.logo.as_deref()?;
        let name = format!("logos/{}-{}.img", station.id, path.chars().filter(char::is_ascii_digit).collect::<String>());
        if let Some(bytes) = std::fs::read(self.dir.join(&name)).ok().filter(|b| art::decode(b).is_some()) {
            return Some(bytes);
        }
        let bytes = net.get(&format!("{SITE}{path}"), LOGO_LIMIT).ok().filter(|b| art::decode(b).is_some())?;
        self.write(&name, &bytes);
        Some(bytes)
    }

    pub fn load_favourites(&self) -> BTreeSet<u64> {
        std::fs::read_to_string(self.dir.join(FAVOURITES_FILE)).map(|t| t.lines().filter_map(|l| l.trim().parse().ok()).collect()).unwrap_or_default()
    }

    pub fn save_favourites(&self, ids: &BTreeSet<u64>) {
        self.write(FAVOURITES_FILE, ids.iter().map(|i| format!("{i}\n")).collect::<String>().as_bytes());
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// What the Radio tab shows and remembers.
#[derive(Default)]
pub struct Radio {
    store: Option<Store>,
    stations: Vec<Station>,
    favourites: BTreeSet<u64>,
    /// a fetch is in flight
    pub fetching: bool,
    /// when the list was last fetched or loaded, as an age at that moment
    fetched: Option<std::time::Instant>,
    /// why the last fetch failed
    pub error: Option<String>,
}

impl Radio {
    /// Loads whatever the store holds (age and all).
    pub fn open(store: Store) -> Self {
        let mut r = Radio { favourites: store.load_favourites(), ..Radio::default() };
        if let Some((list, age)) = store.load_list() {
            r.stations = list;
            r.fetched = std::time::Instant::now().checked_sub(age);
        }
        r.store = Some(store);
        r
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    pub fn stations(&self) -> &[Station] {
        &self.stations
    }

    pub fn station(&self, id: u64) -> Option<&Station> {
        self.stations.iter().find(|s| s.id == id)
    }

    /// Whether the list should be fetched now.
    pub fn stale(&self) -> bool {
        self.store.is_some() && !self.fetching && self.fetched.is_none_or(|t| t.elapsed() >= FRESH)
    }

    /// A fetch finished.
    pub fn fetched(&mut self, result: Result<Vec<Station>, String>) {
        self.fetching = false;
        match result {
            Ok(list) => {
                self.stations = list;
                self.fetched = Some(std::time::Instant::now());
                self.error = None;
            }
            Err(e) => {
                // retry after the usual interval, not at once
                self.fetched = Some(std::time::Instant::now());
                self.error = Some(e);
            }
        }
    }

    /// Indices into `stations()` in display order: favourites first.
    pub fn order(&self) -> Vec<usize> {
        let (fav, rest): (Vec<usize>, Vec<usize>) = (0..self.stations.len()).partition(|&i| self.favourites.contains(&self.stations[i].id));
        fav.into_iter().chain(rest).collect()
    }

    pub fn is_favourite(&self, id: u64) -> bool {
        self.favourites.contains(&id)
    }

    /// Flips a favourite and saves them.
    pub fn toggle_favourite(&mut self, id: u64) {
        if !self.favourites.remove(&id) {
            self.favourites.insert(id);
        }
        if let Some(store) = &self.store {
            store.save_favourites(&self.favourites);
        }
    }
}

/// Compatibility adapter; the parser is shared with the store and updates.
pub mod json {
    pub use bifrost_core::json::Json;
    pub fn parse(text: &str) -> Option<Json> { bifrost_core::json::parse(text).ok() }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn content_range_gives_the_total_size() {
        assert_eq!(content_range_total("bytes 0-1023/52428800"), Some(52_428_800));
        assert_eq!(content_range_total("bytes 0-1023/*"), None);
        assert_eq!(content_range_total("nonsense"), None);
    }

    /// Four entries as the API sends them: Rás 2 (no song), Bylgjan (artist
    /// and song, an escaped name elsewhere), a placeholder and a station
    /// without a frequency or logos.
    pub const API_JSON: &str = r#"{"objects": [
        {"art": [], "frequency": 90.1, "id": 4, "name": "Rás 2", "radio": true, "stream_url": "http://ice-11.spilarinn.is/truv2",
         "logos": [{"type": "ios1x", "url": "/api/files/1"}, {"type": "ios2x", "url": "/api/files/2"}, {"type": "atv", "url": "/api/files/9"}],
         "playing": {"current_artist": {"files": [], "name": null}, "current_song": {"name": null}}},
        {"frequency": 98.9, "id": 12, "name": "Bylgjan", "stream_url": "https://ice-11.spilarinn.is/zbylgjan",
         "logos": [{"type": "atv", "url": "/api/files/20"}],
         "playing": {"current_artist": {"name": "Björk"}, "current_song": {"name": "Hunter 🎵"}}},
        {"frequency": 102.5, "id": 81, "name": "temp1", "stream_url": "http://ice-11.spilarinn.is/leiga2", "logos": []},
        {"id": 7, "name": "Netradio", "stream_url": "http://example.test/live", "frequency": null}
    ]}"#;

    #[test]
    fn parses_the_api_skipping_placeholders() {
        let list = parse_stations(API_JSON).unwrap();
        assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Rás 2", "Bylgjan", "Netradio"]);
        let ras = &list[0];
        assert_eq!((ras.id, ras.frequency, ras.logo.as_deref(), ras.now.as_deref()), (4, Some(90.1), Some("/api/files/2"), None), "prefers the ios2x logo");
        assert_eq!(ras.frequency_text(), "FM 90.1");
        assert_eq!(list[1].now.as_deref(), Some("Björk - Hunter \u{1f3b5}"));
        assert_eq!(list[1].logo.as_deref(), Some("/api/files/20"), "falls back to any logo");
        assert_eq!((list[2].frequency_text(), list[2].logo.clone()), (String::new(), None));
        assert!(parse_stations("{\"objects\": 3}").is_none() && parse_stations("not json").is_none() && parse_stations("{}").is_none());
    }

    #[test]
    fn json_reader_handles_the_basics_and_rejects_junk() {
        use json::{parse, Json};
        assert_eq!(parse(" [1, -2.5e1, true, null, \"a\\n\\\"b\"] "), Some(Json::Arr(vec![Json::Num(1.0), Json::Num(-25.0), Json::Bool(true), Json::Null, Json::Str("a\n\"b".into())])));
        assert_eq!(parse("{\"a\": {\"b\": []}}").unwrap().get("a").unwrap().get("b"), Some(&Json::Arr(vec![])));
        for bad in ["", "[1,", "{\"a\" 1}", "[1] x", "\"open", "{1: 2}", &"[".repeat(100)] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }

    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-radio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Answers from a table and counts the calls.
    pub struct Canned {
        pub pages: Vec<(String, Vec<u8>)>,
        pub calls: Mutex<Vec<String>>,
    }

    impl Fetch for Canned {
        fn get(&self, url: &str, _limit: usize) -> io::Result<Vec<u8>> {
            self.calls.lock().unwrap().push(url.to_string());
            self.pages.iter().find(|(u, _)| u == url).map(|(_, b)| b.clone()).ok_or_else(|| io::Error::other("offline"))
        }
    }

    pub fn png() -> Vec<u8> {
        crate::pngenc::encode_rgb(2, 2, &[0xff0000ff; 4])
    }

    #[test]
    fn the_list_is_fetched_cached_and_readable_offline() {
        let store = Store::new(temp_dir("list"));
        assert!(store.load_list().is_none());
        let online = Canned { pages: vec![(API.into(), API_JSON.as_bytes().to_vec())], calls: Default::default() };
        assert_eq!(store.refresh(&online).unwrap().len(), 3);
        let (list, age) = store.load_list().unwrap();
        assert_eq!(list.len(), 3);
        assert!(age < Duration::from_secs(5));
        let offline = Canned { pages: vec![], calls: Default::default() };
        assert!(store.refresh(&offline).is_err());
        assert_eq!(store.load_list().unwrap().0.len(), 3, "a failed refresh keeps the saved list");
        let junk = Canned { pages: vec![(API.into(), b"<html>".to_vec())], calls: Default::default() };
        assert!(store.refresh(&junk).is_err());
        assert_eq!(store.load_list().unwrap().0.len(), 3, "so does a bad answer");
    }

    #[test]
    fn logos_are_downloaded_once_and_kept_per_logo_path() {
        let store = Store::new(temp_dir("logo"));
        let list = parse_stations(API_JSON).unwrap();
        let net = Canned { pages: vec![("https://bakendi.spilarinn.is/api/files/2".into(), png())], calls: Default::default() };
        assert!(store.logo(&list[0], &net).is_some());
        assert!(store.logo(&list[0], &net).is_some());
        assert_eq!(net.calls.lock().unwrap().len(), 1, "second look is from disk");
        assert!(store.logo(&list[0], &Canned { pages: vec![], calls: Default::default() }).is_some(), "and works offline");
        let bad = Canned { pages: vec![("https://bakendi.spilarinn.is/api/files/20".into(), b"nope".to_vec())], calls: Default::default() };
        assert!(store.logo(&list[1], &bad).is_none(), "a non-image is not kept");
        assert!(store.logo(&list[2], &net).is_none(), "no logo, no request");
        let moved = Station { logo: Some("/api/files/3".into()), ..list[0].clone() };
        assert!(store.logo(&moved, &net).is_none(), "a new logo path is a new download");
    }

    #[test]
    fn favourites_persist_and_lead_the_order() {
        let dir = temp_dir("fav");
        let mut r = Radio::open(Store::new(&dir));
        r.fetched(Ok(parse_stations(API_JSON).unwrap()));
        assert_eq!(r.order(), [0, 1, 2]);
        r.toggle_favourite(7);
        r.toggle_favourite(12);
        assert_eq!(r.order(), [1, 2, 0], "favourites first, each group in list order");
        r.toggle_favourite(12);
        assert!(!r.is_favourite(12) && r.is_favourite(7));
        let again = Radio::open(Store::new(&dir));
        assert!(again.is_favourite(7) && !again.is_favourite(12), "saved");
    }

    #[test]
    fn staleness_follows_fetches_and_failures_back_off() {
        let dir = temp_dir("stale");
        let mut r = Radio::open(Store::new(&dir));
        assert!(r.stale(), "nothing yet");
        r.fetching = true;
        assert!(!r.stale(), "one fetch at a time");
        r.fetched(Err("offline".into()));
        assert!(!r.stale() && r.error.as_deref() == Some("offline"), "a failure waits out the interval too");
        r.fetched(Ok(parse_stations(API_JSON).unwrap()));
        assert!(!r.stale() && r.error.is_none());
        assert!(!Radio::default().stale(), "without a store there is nothing to fetch to");
    }
}
