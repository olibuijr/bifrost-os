//! `URLCache` with a disk store, `CachedURLResponse` and `URLRequest.CachePolicy`.
//!
//! Entries live as one file each under a directory (the app container's cache folder). A file holds the URL, the
//! status, the time it was stored (Unix seconds, passed in so freshness is testable), the headers and the body.
//! `Cache-Control: no-store` is never stored; `no-cache` is stored but always revalidated; `max-age` sets freshness;
//! an `ETag` lets a stale entry be revalidated with `If-None-Match` and renewed by a 304. When the bodies on disk
//! exceed `disk_capacity`, the oldest entries go first. `memory_capacity` bounds a read-through copy kept in memory.
//! The in-memory `url::URLCache` used by `URLSession::shared()` is separate and unchanged.

use crate::url::HTTPURLResponse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `URLRequest.CachePolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePolicy {
    UseProtocolCachePolicy,
    ReloadIgnoringLocalCacheData,
    ReturnCacheDataElseLoad,
    ReturnCacheDataDontLoad,
    ReloadRevalidatingCacheData,
}

/// What a session does for a request, given the policy and the stored entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheDecision {
    /// Go to the network, no conditional header.
    Load,
    /// Answer from the cache without the network.
    UseCache,
    /// Go to the network with `If-None-Match` set to this validator; a 304 renews the entry.
    Revalidate(String),
    /// `ReturnCacheDataDontLoad` with nothing stored: the request fails.
    Fail,
}

/// `CachedURLResponse`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedURLResponse {
    pub response: HTTPURLResponse,
    pub data: Vec<u8>,
    /// Unix seconds when stored or last renewed.
    pub stored: u64,
}

impl CachedURLResponse {
    fn max_age(&self) -> Option<u64> {
        let cc = self.response.value("Cache-Control")?.to_ascii_lowercase();
        if cc.contains("no-cache") {
            return None;
        }
        cc.split(',').find_map(|d| d.trim().strip_prefix("max-age=").and_then(|v| v.trim().parse::<u64>().ok()))
    }
    /// True while `max-age` has not run out at `now`.
    pub fn is_fresh(&self, now: u64) -> bool {
        self.max_age().map_or(false, |m| now.saturating_sub(self.stored) < m)
    }
    pub fn validator(&self) -> Option<String> {
        self.response.value("ETag").map(str::to_string)
    }
}

/// `URLCache`: memory and disk capacity, a directory, and the stored responses.
pub struct URLCache {
    pub memory_capacity: usize,
    pub disk_capacity: usize,
    dir: PathBuf,
    memory: Mutex<HashMap<String, CachedURLResponse>>,
}

fn key_of(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn encode(url: &str, c: &CachedURLResponse) -> Vec<u8> {
    let mut s = format!("{url}\n{}\n{}\n", c.response.status_code, c.stored);
    for (k, v) in &c.response.headers {
        s.push_str(&format!("{}: {}\n", k.replace('\n', " "), v.replace('\n', " ")));
    }
    s.push('\n');
    let mut out = s.into_bytes();
    out.extend_from_slice(&c.data);
    out
}

fn decode(bytes: &[u8]) -> Option<(String, CachedURLResponse)> {
    let split = bytes.windows(2).position(|w| w == b"\n\n")? + 2;
    let head = std::str::from_utf8(&bytes[..split - 2]).ok()?;
    let mut lines = head.split('\n');
    let url = lines.next()?.to_string();
    let status_code = lines.next()?.parse().ok()?;
    let stored = lines.next()?.parse().ok()?;
    let headers = lines.filter_map(|l| l.split_once(": ").map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    let response = HTTPURLResponse { url: url.clone(), status_code, headers };
    Some((url, CachedURLResponse { response, data: bytes[split..].to_vec(), stored }))
}

impl URLCache {
    /// `init(memoryCapacity:diskCapacity:directory:)`. The directory is created on the first store.
    pub fn new(memory_capacity: usize, disk_capacity: usize, dir: &Path) -> Self {
        URLCache { memory_capacity, disk_capacity, dir: dir.to_path_buf(), memory: Mutex::new(HashMap::new()) }
    }

    fn path(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.entry", key_of(url)))
    }

    fn disk_entries(&self) -> Vec<(PathBuf, String, CachedURLResponse)> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        rd.filter_map(|e| {
            let p = e.ok()?.path();
            if p.extension()? != "entry" {
                return None;
            }
            let (url, c) = decode(&std::fs::read(&p).ok()?)?;
            Some((p, url, c))
        })
        .collect()
    }

    /// `currentDiskUsage`: bytes of stored bodies.
    pub fn current_disk_usage(&self) -> usize {
        self.disk_entries().iter().map(|(_, _, c)| c.data.len()).sum()
    }

    /// `currentMemoryUsage`.
    pub fn current_memory_usage(&self) -> usize {
        self.memory.lock().unwrap().values().map(|c| c.data.len()).sum()
    }

    /// `storeCachedResponse(_:for:)` at time `now`. Returns false when the response is not cacheable: not a 200, marked
    /// `no-store`, bigger than the disk capacity, or none of `max-age`, `ETag` and `no-cache` is given.
    pub fn store_cached_response(&self, response: &HTTPURLResponse, data: &[u8], now: u64) -> bool {
        let cc = response.value("Cache-Control").unwrap_or("").to_ascii_lowercase();
        let usable = cc.contains("max-age=") || cc.contains("no-cache") || response.value("ETag").is_some();
        if response.status_code != 200 || cc.contains("no-store") || data.len() > self.disk_capacity || !usable {
            return false;
        }
        self.write(&CachedURLResponse { response: response.clone(), data: data.to_vec(), stored: now })
    }

    fn write(&self, cached: &CachedURLResponse) -> bool {
        let url = &cached.response.url;
        if std::fs::create_dir_all(&self.dir).is_err() || std::fs::write(self.path(url), encode(url, cached)).is_err() {
            return false;
        }
        self.remember(cached);
        self.evict();
        true
    }

    fn remember(&self, cached: &CachedURLResponse) {
        let mut mem = self.memory.lock().unwrap();
        mem.remove(&cached.response.url);
        if cached.data.len() <= self.memory_capacity {
            mem.insert(cached.response.url.clone(), cached.clone());
            while mem.values().map(|c| c.data.len()).sum::<usize>() > self.memory_capacity {
                let Some(oldest) = mem.iter().min_by_key(|(_, c)| c.stored).map(|(k, _)| k.clone()) else { break };
                mem.remove(&oldest);
            }
        }
    }

    /// Drops the oldest entries until the bodies fit `disk_capacity`.
    fn evict(&self) {
        let mut entries = self.disk_entries();
        let mut total: usize = entries.iter().map(|(_, _, c)| c.data.len()).sum();
        entries.sort_by_key(|(_, url, c)| (c.stored, url.clone()));
        for (path, url, c) in entries {
            if total <= self.disk_capacity {
                break;
            }
            total -= c.data.len();
            let _ = std::fs::remove_file(path);
            self.memory.lock().unwrap().remove(&url);
        }
    }

    /// `cachedResponse(for:)`: memory first, then disk.
    pub fn cached_response(&self, url: &str) -> Option<CachedURLResponse> {
        if let Some(c) = self.memory.lock().unwrap().get(url) {
            return Some(c.clone());
        }
        let (stored_url, c) = decode(&std::fs::read(self.path(url)).ok()?)?;
        if stored_url != url {
            return None;
        }
        self.remember(&c);
        Some(c)
    }

    /// Renews the entry after a 304 at `now`; a `Cache-Control` in the 304 replaces the old one.
    pub fn renew(&self, url: &str, not_modified: &HTTPURLResponse, now: u64) -> Option<CachedURLResponse> {
        let mut c = self.cached_response(url)?;
        c.stored = now;
        if let Some(cc) = not_modified.value("Cache-Control") {
            c.response.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Cache-Control"));
            c.response.headers.push(("Cache-Control".to_string(), cc.to_string()));
        }
        self.write(&c).then_some(c)
    }

    /// `removeCachedResponse(for:)`.
    pub fn remove_cached_response(&self, url: &str) {
        self.memory.lock().unwrap().remove(url);
        let _ = std::fs::remove_file(self.path(url));
    }

    /// `removeAllCachedResponses()`.
    pub fn remove_all_cached_responses(&self) {
        self.memory.lock().unwrap().clear();
        for (p, _, _) in self.disk_entries() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// What to do for a request with `policy` at `now`.
    pub fn decide(&self, url: &str, policy: CachePolicy, now: u64) -> CacheDecision {
        let cached = self.cached_response(url);
        match (policy, cached) {
            (CachePolicy::ReloadIgnoringLocalCacheData, _) => CacheDecision::Load,
            (CachePolicy::ReturnCacheDataDontLoad, None) => CacheDecision::Fail,
            (CachePolicy::ReturnCacheDataDontLoad, Some(_)) | (CachePolicy::ReturnCacheDataElseLoad, Some(_)) => CacheDecision::UseCache,
            (_, None) => CacheDecision::Load,
            (CachePolicy::ReloadRevalidatingCacheData, Some(c)) => c.validator().map_or(CacheDecision::Load, CacheDecision::Revalidate),
            (CachePolicy::UseProtocolCachePolicy, Some(c)) if c.is_fresh(now) => CacheDecision::UseCache,
            (CachePolicy::UseProtocolCachePolicy, Some(c)) => c.validator().map_or(CacheDecision::Load, CacheDecision::Revalidate),
            (CachePolicy::ReturnCacheDataElseLoad, None) => CacheDecision::Load,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-urlcache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }
    fn resp(url: &str, cc: Option<&str>, etag: Option<&str>) -> HTTPURLResponse {
        let mut headers = Vec::new();
        if let Some(c) = cc {
            headers.push(("Cache-Control".to_string(), c.to_string()));
        }
        if let Some(e) = etag {
            headers.push(("ETag".to_string(), e.to_string()));
        }
        HTTPURLResponse { url: url.to_string(), status_code: 200, headers }
    }

    #[test]
    fn a_stored_response_survives_a_new_cache_over_the_same_directory() {
        let d = dir("persist");
        let c = URLCache::new(1 << 10, 1 << 20, &d);
        assert!(c.store_cached_response(&resp("http://a/x", Some("max-age=60"), Some("\"v1\"")), b"body", 100));
        let again = URLCache::new(0, 1 << 20, &d);
        let got = again.cached_response("http://a/x").unwrap();
        assert_eq!(got.data, b"body");
        assert_eq!(got.stored, 100);
        assert_eq!(got.validator().as_deref(), Some("\"v1\""));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn no_store_and_unvalidatable_responses_are_not_kept() {
        let c = URLCache::new(0, 1 << 20, &dir("nostore"));
        assert!(!c.store_cached_response(&resp("http://a/1", Some("no-store"), Some("\"v\"")), b"x", 0));
        assert!(!c.store_cached_response(&resp("http://a/2", None, None), b"x", 0));
        assert!(c.cached_response("http://a/1").is_none());
    }

    #[test]
    fn freshness_follows_max_age_and_no_cache_is_always_stale() {
        let c = URLCache::new(0, 1 << 20, &dir("fresh"));
        c.store_cached_response(&resp("http://a/m", Some("max-age=10"), None), b"m", 100);
        c.store_cached_response(&resp("http://a/n", Some("no-cache"), Some("\"n\"")), b"n", 100);
        assert_eq!(c.decide("http://a/m", CachePolicy::UseProtocolCachePolicy, 105), CacheDecision::UseCache);
        assert_eq!(c.decide("http://a/m", CachePolicy::UseProtocolCachePolicy, 110), CacheDecision::Load);
        assert_eq!(c.decide("http://a/n", CachePolicy::UseProtocolCachePolicy, 100), CacheDecision::Revalidate("\"n\"".into()));
    }

    #[test]
    fn policies_choose_between_cache_and_network() {
        let c = URLCache::new(0, 1 << 20, &dir("policy"));
        let u = "http://a/p";
        assert_eq!(c.decide(u, CachePolicy::ReturnCacheDataDontLoad, 0), CacheDecision::Fail);
        assert_eq!(c.decide(u, CachePolicy::ReturnCacheDataElseLoad, 0), CacheDecision::Load);
        c.store_cached_response(&resp(u, Some("max-age=1"), Some("\"e\"")), b"p", 0);
        assert_eq!(c.decide(u, CachePolicy::ReturnCacheDataDontLoad, 999), CacheDecision::UseCache);
        assert_eq!(c.decide(u, CachePolicy::ReturnCacheDataElseLoad, 999), CacheDecision::UseCache);
        assert_eq!(c.decide(u, CachePolicy::ReloadIgnoringLocalCacheData, 0), CacheDecision::Load);
        assert_eq!(c.decide(u, CachePolicy::ReloadRevalidatingCacheData, 0), CacheDecision::Revalidate("\"e\"".into()));
    }

    #[test]
    fn a_304_renews_the_entry() {
        let c = URLCache::new(0, 1 << 20, &dir("renew"));
        let u = "http://a/r";
        c.store_cached_response(&resp(u, Some("max-age=5"), Some("\"e\"")), b"r", 0);
        let nm = HTTPURLResponse { url: u.into(), status_code: 304, headers: vec![("Cache-Control".into(), "max-age=100".into())] };
        let r = c.renew(u, &nm, 50).unwrap();
        assert_eq!(r.data, b"r");
        assert!(c.cached_response(u).unwrap().is_fresh(120));
    }

    #[test]
    fn the_oldest_entries_go_when_the_disk_capacity_is_exceeded() {
        let c = URLCache::new(0, 10, &dir("evict"));
        c.store_cached_response(&resp("http://a/1", Some("max-age=9"), None), b"aaaa", 1);
        c.store_cached_response(&resp("http://a/2", Some("max-age=9"), None), b"bbbb", 2);
        c.store_cached_response(&resp("http://a/3", Some("max-age=9"), None), b"cccc", 3);
        assert!(c.current_disk_usage() <= 10);
        assert!(c.cached_response("http://a/1").is_none());
        assert!(c.cached_response("http://a/3").is_some());
        assert!(!c.store_cached_response(&resp("http://a/big", Some("max-age=9"), None), &[0u8; 11], 4));
    }

    #[test]
    fn remove_clears_one_or_all() {
        let c = URLCache::new(100, 1 << 20, &dir("remove"));
        c.store_cached_response(&resp("http://a/1", Some("max-age=9"), None), b"1", 1);
        c.store_cached_response(&resp("http://a/2", Some("max-age=9"), None), b"2", 1);
        c.remove_cached_response("http://a/1");
        assert!(c.cached_response("http://a/1").is_none());
        c.remove_all_cached_responses();
        assert_eq!(c.current_disk_usage(), 0);
        assert_eq!(c.current_memory_usage(), 0);
    }
}
