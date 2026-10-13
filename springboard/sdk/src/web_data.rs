//! Cookies and website data kept in the profile directory (`HTTPCookieStore` and `WebsiteDataStore`).
//!
//! [`CookieStore`] holds cookies in memory and saves the persistent ones (those with an expiry) to
//! `cookies.tsv` in the profile directory. [`WebsiteDataStore`] adds per-site storage folders next to it, so an app
//! can list what a site keeps (`fetchDataRecords`) and remove it (`removeData`). Times are seconds since the
//! Unix epoch, passed in by the caller.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `HTTPCookie`. `expires` is `None` for a session cookie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Host the cookie belongs to; a leading dot is ignored and the cookie also matches subdomains.
    pub domain: String,
    pub path: String,
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
}

/// True when `host` is `base` or a subdomain of it. Both are lowercase.
fn host_within(host: &str, base: &str) -> bool {
    host == base || host.strip_suffix(base).is_some_and(|rest| rest.ends_with('.'))
}

impl Cookie {
    pub fn new(name: &str, value: &str, domain: &str) -> Self {
        Self { name: name.into(), value: value.into(), domain: domain.into(), path: "/".into(), expires: None, secure: false, http_only: false }
    }

    fn host(&self) -> String {
        self.domain.trim_start_matches('.').to_ascii_lowercase()
    }

    fn expired(&self, now: u64) -> bool {
        self.expires.is_some_and(|e| e <= now)
    }

    fn same_key(&self, other: &Cookie) -> bool {
        self.name == other.name && self.host() == other.host() && self.path == other.path
    }

    fn matches(&self, scheme: &str, host: &str, path: &str) -> bool {
        let host_ok = host_within(&host.to_ascii_lowercase(), &self.host());
        host_ok && path.starts_with(self.path.as_str()) && (!self.secure || scheme == "https")
    }

    fn storable(&self) -> bool {
        let clean = |s: &str| !s.contains(['\t', '\n', '\r']);
        !self.name.is_empty()
            && !self.domain.is_empty()
            && self.path.starts_with('/')
            && clean(&self.name)
            && clean(&self.value)
            && clean(&self.domain)
            && clean(&self.path)
    }
}

/// `(scheme, host, path)` of an absolute URL, or `None` without a scheme and host.
fn split_url(url: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    if host.is_empty() {
        return None;
    }
    let tail = rest[end..].split(['?', '#']).next().unwrap_or("");
    let path = if tail.starts_with('/') { tail } else { "/" };
    Some((scheme.to_ascii_lowercase(), host.to_string(), path.to_string()))
}

/// `HTTPCookieStore`.
#[derive(Clone, Debug, Default)]
pub struct CookieStore {
    cookies: Vec<Cookie>,
}

impl CookieStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// `getAllCookies`: every cookie that has not expired at `now`.
    pub fn get_all_cookies(&self, now: u64) -> Vec<Cookie> {
        self.cookies.iter().filter(|c| !c.expired(now)).cloned().collect()
    }

    /// The cookies a request to `url` carries at `now`, longest path first.
    pub fn cookies_for(&self, url: &str, now: u64) -> Vec<Cookie> {
        let Some((scheme, host, path)) = split_url(url) else { return Vec::new() };
        let mut found: Vec<Cookie> = self.cookies.iter().filter(|c| !c.expired(now) && c.matches(&scheme, &host, &path)).cloned().collect();
        found.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        found
    }

    /// `setCookie`: adds the cookie or replaces the one with the same name, domain and path. A cookie that is
    /// already expired at `now` removes its match instead. Returns false for a malformed cookie.
    pub fn set_cookie(&mut self, cookie: Cookie, now: u64) -> bool {
        if !cookie.storable() {
            return false;
        }
        self.cookies.retain(|c| !c.same_key(&cookie));
        if !cookie.expired(now) {
            self.cookies.push(cookie);
        }
        true
    }

    /// `delete`: removes the cookie with the same name, domain and path. Returns whether one was removed.
    pub fn delete(&mut self, cookie: &Cookie) -> bool {
        let before = self.cookies.len();
        self.cookies.retain(|c| !c.same_key(cookie));
        self.cookies.len() != before
    }

    /// Removes every cookie of `host` and its subdomains; returns how many.
    pub fn delete_host(&mut self, host: &str) -> usize {
        let host = host.trim_start_matches('.').to_ascii_lowercase();
        let before = self.cookies.len();
        self.cookies.retain(|c| !host_within(&c.host(), &host));
        before - self.cookies.len()
    }

    /// Saves the persistent, unexpired cookies to `file`.
    pub fn save(&self, file: &Path, now: u64) -> io::Result<()> {
        let mut out = String::new();
        for c in self.cookies.iter().filter(|c| !c.expired(now)) {
            let Some(expires) = c.expires else { continue };
            out.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\n", c.domain, c.path, c.name, c.value, expires, c.secure as u8, c.http_only as u8));
        }
        fs::write(file, out)
    }

    /// Loads a file written by `save`, dropping expired and malformed lines. A missing file gives an empty store.
    pub fn load(file: &Path, now: u64) -> io::Result<Self> {
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(e),
        };
        let mut store = Self::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 7 {
                continue;
            }
            let Ok(expires) = f[4].parse::<u64>() else { continue };
            let cookie = Cookie {
                domain: f[0].into(),
                path: f[1].into(),
                name: f[2].into(),
                value: f[3].into(),
                expires: Some(expires),
                secure: f[5] == "1",
                http_only: f[6] == "1",
            };
            store.set_cookie(cookie, now);
        }
        Ok(store)
    }
}

/// `WebsiteDataTypeCookies`, `...LocalStorage`, `...DiskCache`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WebsiteDataType {
    Cookies,
    LocalStorage,
    DiskCache,
}

impl WebsiteDataType {
    fn folder(self) -> Option<&'static str> {
        match self {
            WebsiteDataType::Cookies => None,
            WebsiteDataType::LocalStorage => Some("local-storage"),
            WebsiteDataType::DiskCache => Some("disk-cache"),
        }
    }
}

/// `WebsiteDataRecord`: what one site stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebsiteDataRecord {
    /// `displayName`: the site's host.
    pub display_name: String,
    pub data_types: BTreeSet<WebsiteDataType>,
}

/// `WebsiteDataStore` over a profile directory: `cookies.tsv`, and one folder per site and data type under
/// `local-storage/<host>` and `disk-cache/<host>`.
#[derive(Debug)]
pub struct WebsiteDataStore {
    dir: PathBuf,
    pub cookies: CookieStore,
}

impl WebsiteDataStore {
    /// `default()`: opens the store in `dir`, creating it, and loads the saved cookies.
    pub fn open(dir: &Path, now: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let cookies = CookieStore::load(&dir.join("cookies.tsv"), now)?;
        Ok(Self { dir: dir.to_path_buf(), cookies })
    }

    /// Writes the cookies to the profile directory.
    pub fn flush(&self, now: u64) -> io::Result<()> {
        self.cookies.save(&self.dir.join("cookies.tsv"), now)
    }

    /// `fetchDataRecords(ofTypes:)`: one record per site that stores any of `types`, sorted by host.
    pub fn fetch_data_records(&self, types: &[WebsiteDataType], now: u64) -> Vec<WebsiteDataRecord> {
        let mut sites: BTreeMap<String, BTreeSet<WebsiteDataType>> = BTreeMap::new();
        for &t in types {
            match t.folder() {
                None => {
                    for c in self.cookies.get_all_cookies(now) {
                        sites.entry(c.host()).or_default().insert(t);
                    }
                }
                Some(folder) => {
                    let Ok(entries) = fs::read_dir(self.dir.join(folder)) else { continue };
                    for e in entries.flatten() {
                        if e.path().is_dir() {
                            sites.entry(e.file_name().to_string_lossy().to_ascii_lowercase()).or_default().insert(t);
                        }
                    }
                }
            }
        }
        sites.into_iter().map(|(display_name, data_types)| WebsiteDataRecord { display_name, data_types }).collect()
    }

    /// `removeData(ofTypes:for:)`: removes `types` of the sites in `records`.
    pub fn remove_data(&mut self, types: &[WebsiteDataType], records: &[WebsiteDataRecord]) -> io::Result<()> {
        for r in records {
            self.remove_site(types, &r.display_name)?;
        }
        Ok(())
    }

    /// Removes `types` of every site.
    pub fn remove_all(&mut self, types: &[WebsiteDataType], now: u64) -> io::Result<()> {
        let records = self.fetch_data_records(types, now);
        self.remove_data(types, &records)
    }

    fn remove_site(&mut self, types: &[WebsiteDataType], host: &str) -> io::Result<()> {
        // a host with a separator would reach outside the site folders
        if host.is_empty() || host.contains(['/', '\\']) || host == ".." || host == "." {
            return Ok(());
        }
        for t in types {
            match t.folder() {
                None => {
                    self.cookies.delete_host(host);
                }
                Some(folder) => match fs::remove_dir_all(self.dir.join(folder).join(host)) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                },
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-web-data-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn persistent(name: &str, domain: &str) -> Cookie {
        Cookie { expires: Some(1_000), ..Cookie::new(name, "v", domain) }
    }

    #[test]
    fn set_replaces_by_name_domain_and_path() {
        let mut s = CookieStore::new();
        assert!(s.set_cookie(Cookie::new("sid", "1", "a.test"), 0));
        assert!(s.set_cookie(Cookie::new("sid", "2", ".A.test"), 0));
        assert!(s.set_cookie(Cookie { path: "/x".into(), ..Cookie::new("sid", "3", "a.test") }, 0));
        let all = s.get_all_cookies(0);
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|c| c.value == "2"));
    }

    #[test]
    fn malformed_cookies_are_refused() {
        let mut s = CookieStore::new();
        assert!(!s.set_cookie(Cookie::new("", "v", "a.test"), 0));
        assert!(!s.set_cookie(Cookie::new("n", "a\tb", "a.test"), 0));
        assert!(!s.set_cookie(Cookie { path: "x".into(), ..Cookie::new("n", "v", "a.test") }, 0));
        assert!(s.get_all_cookies(0).is_empty());
    }

    #[test]
    fn expired_cookies_are_hidden_and_an_expired_set_removes() {
        let mut s = CookieStore::new();
        s.set_cookie(persistent("a", "a.test"), 0);
        assert_eq!(s.get_all_cookies(999).len(), 1);
        assert!(s.get_all_cookies(1_000).is_empty());
        s.set_cookie(Cookie { expires: Some(5), ..Cookie::new("a", "v", "a.test") }, 10);
        assert!(s.get_all_cookies(0).is_empty());
    }

    #[test]
    fn delete_removes_only_the_matching_cookie() {
        let mut s = CookieStore::new();
        s.set_cookie(Cookie::new("a", "1", "a.test"), 0);
        s.set_cookie(Cookie::new("b", "1", "a.test"), 0);
        assert!(s.delete(&Cookie::new("a", "ignored", "a.test")));
        assert!(!s.delete(&Cookie::new("a", "1", "a.test")));
        assert_eq!(s.get_all_cookies(0).len(), 1);
    }

    #[test]
    fn a_request_carries_matching_domain_path_and_scheme() {
        let mut s = CookieStore::new();
        s.set_cookie(Cookie::new("all", "1", "a.test"), 0);
        s.set_cookie(Cookie { secure: true, ..Cookie::new("sec", "1", "a.test") }, 0);
        s.set_cookie(Cookie { path: "/app".into(), ..Cookie::new("deep", "1", "a.test") }, 0);
        s.set_cookie(Cookie::new("other", "1", "b.test"), 0);
        s.set_cookie(Cookie::new("lookalike", "1", "xa.test"), 0);
        let names = |url: &str| s.cookies_for(url, 0).into_iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(names("http://a.test/"), vec!["all"]);
        assert_eq!(names("https://www.a.test/app/x?q=1"), vec!["deep", "all", "sec"]);
        assert!(names("not a url").is_empty());
    }

    #[test]
    fn only_persistent_cookies_survive_a_save_and_load() {
        let dir = scratch("cookies");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cookies.tsv");
        let mut s = CookieStore::new();
        s.set_cookie(persistent("keep", "a.test"), 0);
        s.set_cookie(Cookie::new("session", "v", "a.test"), 0);
        s.save(&file, 0).unwrap();
        let loaded = CookieStore::load(&file, 0).unwrap();
        assert_eq!(loaded.get_all_cookies(0), vec![persistent("keep", "a.test")]);
        assert!(CookieStore::load(&file, 2_000).unwrap().get_all_cookies(2_000).is_empty());
        assert!(CookieStore::load(&dir.join("missing"), 0).unwrap().get_all_cookies(0).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_list_sites_by_type_and_removal_is_scoped() {
        let dir = scratch("store");
        let mut store = WebsiteDataStore::open(&dir, 0).unwrap();
        store.cookies.set_cookie(Cookie::new("a", "1", "one.test"), 0);
        fs::create_dir_all(dir.join("local-storage/one.test")).unwrap();
        fs::create_dir_all(dir.join("local-storage/two.test")).unwrap();
        let all = [WebsiteDataType::Cookies, WebsiteDataType::LocalStorage, WebsiteDataType::DiskCache];
        let records = store.fetch_data_records(&all, 0);
        assert_eq!(records.iter().map(|r| r.display_name.as_str()).collect::<Vec<_>>(), vec!["one.test", "two.test"]);
        assert_eq!(records[0].data_types.len(), 2);
        store.remove_data(&[WebsiteDataType::LocalStorage], &records[..1]).unwrap();
        assert!(!dir.join("local-storage/one.test").exists() && dir.join("local-storage/two.test").exists());
        assert_eq!(store.cookies.get_all_cookies(0).len(), 1);
        store.remove_all(&all, 0).unwrap();
        assert!(store.fetch_data_records(&all, 0).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removal_ignores_hosts_that_would_leave_the_store() {
        let dir = scratch("escape");
        let mut store = WebsiteDataStore::open(&dir, 0).unwrap();
        fs::create_dir_all(dir.join("local-storage")).unwrap();
        let bad = WebsiteDataRecord { display_name: "../cookies".into(), data_types: BTreeSet::new() };
        store.remove_data(&[WebsiteDataType::LocalStorage], &[bad]).unwrap();
        assert!(dir.join("local-storage").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_open_restores_cookies() {
        let dir = scratch("flush");
        let mut store = WebsiteDataStore::open(&dir, 0).unwrap();
        store.cookies.set_cookie(persistent("kept", "a.test"), 0);
        store.flush(0).unwrap();
        let again = WebsiteDataStore::open(&dir, 0).unwrap();
        assert_eq!(again.cookies.get_all_cookies(0).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
