//! HTTP caching for feeds and pictures. A stored copy is used while its `max-age` lasts; an expired copy is
//! revalidated with `If-None-Match` and `If-Modified-Since`, and a 304 answer reuses the stored body. These are the
//! rules of the URL cache (`URLCache`: no-store, no-cache, max-age, ETag). The pure rules come first and are tested;
//! `Conditional` runs them over `URLSession` for the feed and picture loads.

use bifrost_app::url::{URLRequest, URLSession};
use springboard::shell::music::radio::Fetch;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Bodies larger than this are fetched every time and never kept.
pub const STORE_MAX: usize = 2_000_000;
/// How many URLs keep a stored copy. When the store is full it is emptied, and the next loads fetch again.
pub const STORE_ENTRIES: usize = 64;

/// The validators a response carries (empty values are dropped).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// A kept response body and what is needed to reuse or revalidate it.
#[derive(Clone, Debug)]
pub struct Stored {
    pub body: Vec<u8>,
    pub validators: Validators,
    pub max_age: Duration,
    pub stored: Instant,
}

/// What a load does with its stored copy.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// the copy is still fresh: use it without a request
    Fresh(Vec<u8>),
    /// the copy is stale: send these conditional headers
    Revalidate(Vec<(&'static str, String)>),
    /// no usable copy: a plain request
    Fetch,
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

fn non_empty(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The ETag and Last-Modified values of a response.
pub fn validators_of(headers: &[(String, String)]) -> Validators {
    Validators { etag: non_empty(header(headers, "ETag")), last_modified: non_empty(header(headers, "Last-Modified")) }
}

/// The freshness lifetime from a Cache-Control value. None when the response must not be stored (`no-store`).
/// `no-cache`, or no `max-age`, gives zero: the copy is kept only to revalidate.
pub fn max_age_of(cache_control: Option<&str>) -> Option<Duration> {
    let cc = cache_control.unwrap_or("").to_ascii_lowercase();
    let directives: Vec<&str> = cc.split(',').map(str::trim).collect();
    if directives.contains(&"no-store") {
        return None;
    }
    if directives.contains(&"no-cache") {
        return Some(Duration::ZERO);
    }
    let secs = directives.iter().find_map(|d| d.strip_prefix("max-age=")?.trim().parse::<u64>().ok()).unwrap_or(0);
    Some(Duration::from_secs(secs))
}

/// The copy to keep for a response, or None when it must not be kept: not a 200, too big, `no-store`, or with
/// neither a lifetime nor a validator to check later.
pub fn stored_from(status: u16, headers: &[(String, String)], body: &[u8], now: Instant) -> Option<Stored> {
    if status != 200 || body.len() > STORE_MAX {
        return None;
    }
    let max_age = max_age_of(header(headers, "Cache-Control"))?;
    let validators = validators_of(headers);
    if max_age.is_zero() && validators == Validators::default() {
        return None;
    }
    Some(Stored { body: body.to_vec(), validators, max_age, stored: now })
}

/// The headers that revalidate a stored copy.
pub fn conditional_headers(v: &Validators) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(etag) = &v.etag {
        out.push(("If-None-Match", etag.clone()));
    }
    if let Some(date) = &v.last_modified {
        out.push(("If-Modified-Since", date.clone()));
    }
    out
}

/// What a load does, given its stored copy and the time now. A copy is fresh while its age is below `max_age`.
pub fn plan(stored: Option<&Stored>, now: Instant) -> Plan {
    match stored {
        None => Plan::Fetch,
        Some(s) if now.saturating_duration_since(s.stored) < s.max_age => Plan::Fresh(s.body.clone()),
        Some(s) => {
            let headers = conditional_headers(&s.validators);
            if headers.is_empty() {
                Plan::Fetch
            } else {
                Plan::Revalidate(headers)
            }
        }
    }
}

/// A `Fetch` that keeps stored copies of responses and revalidates them (the HTTP caching above).
#[derive(Default)]
pub struct Conditional {
    store: Mutex<HashMap<String, Stored>>,
}

impl Conditional {
    fn keep(&self, url: &str, copy: Stored) {
        let mut map = self.store.lock().unwrap();
        if map.len() >= STORE_ENTRIES && !map.contains_key(url) {
            map.clear();
        }
        map.insert(url.to_string(), copy);
    }

    fn forget(&self, url: &str) {
        self.store.lock().unwrap().remove(url);
    }
}

impl Fetch for Conditional {
    fn get(&self, url: &str, limit: usize) -> io::Result<Vec<u8>> {
        let now = Instant::now();
        let stored = self.store.lock().unwrap().get(url).cloned();
        let mut req = URLRequest::new(url);
        match plan(stored.as_ref(), now) {
            Plan::Fresh(body) => return Ok(body),
            Plan::Revalidate(headers) => {
                for (name, value) in headers {
                    req.set_header(name, &value);
                }
            }
            Plan::Fetch => {}
        }
        let cancel = AtomicBool::new(false);
        let mut body = Vec::new();
        let mut too_big = false;
        let resp = URLSession::shared()
            .bytes(&req, &cancel, &mut |_| {}, &mut |chunk| {
                body.extend_from_slice(chunk);
                too_big = body.len() > limit;
                !too_big
            })
            .map_err(|e| io::Error::other(format!("{url}: {e}")))?;
        if too_big {
            return Err(io::Error::other(format!("{url}: larger than {limit} bytes")));
        }
        if resp.status_code == 304 {
            let Some(mut copy) = stored else {
                return Err(io::Error::other(format!("{url}: not modified, but no copy is stored")));
            };
            // the 304 may carry a new lifetime; without one the old lifetime stands
            if let Some(cc) = header(&resp.headers, "Cache-Control") {
                copy.max_age = max_age_of(Some(cc)).unwrap_or(copy.max_age);
            }
            copy.stored = now;
            let out = copy.body.clone();
            self.keep(url, copy);
            return Ok(out);
        }
        if resp.status_code != 200 {
            return Err(io::Error::other(format!("{url}: status {}", resp.status_code)));
        }
        match stored_from(resp.status_code, &resp.headers, &body, now) {
            Some(copy) => self.keep(url, copy),
            None => self.forget(url),
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn max_age_is_read_from_cache_control() {
        assert_eq!(max_age_of(Some("public, max-age=60")), Some(Duration::from_secs(60)));
        assert_eq!(max_age_of(Some("Max-Age=5")), Some(Duration::from_secs(5)), "directives are case-insensitive");
        assert_eq!(max_age_of(Some("no-cache")), Some(Duration::ZERO));
        assert_eq!(max_age_of(Some("no-store, max-age=60")), None, "no-store wins");
        assert_eq!(max_age_of(Some("max-age=abc")), Some(Duration::ZERO), "a bad number is no lifetime");
        assert_eq!(max_age_of(None), Some(Duration::ZERO));
    }

    #[test]
    fn validators_are_taken_case_insensitively_and_blank_values_are_dropped() {
        let h = headers(&[("etag", "\"v1\""), ("LAST-MODIFIED", "Mon, 05 Oct 2026 10:00:00 GMT")]);
        let v = validators_of(&h);
        assert_eq!(v.etag.as_deref(), Some("\"v1\""));
        assert_eq!(v.last_modified.as_deref(), Some("Mon, 05 Oct 2026 10:00:00 GMT"));
        assert_eq!(validators_of(&headers(&[("ETag", "  ")])), Validators::default());
    }

    #[test]
    fn a_response_is_kept_only_when_it_can_be_reused_or_checked() {
        let now = Instant::now();
        let fresh = headers(&[("Cache-Control", "max-age=300")]);
        let kept = stored_from(200, &fresh, b"feed", now).unwrap();
        assert_eq!(kept.body, b"feed");
        assert_eq!(kept.max_age, Duration::from_secs(300));

        let etag_only = headers(&[("ETag", "\"abc\"")]);
        assert!(stored_from(200, &etag_only, b"x", now).is_some(), "an ETag alone can be revalidated");
        assert!(stored_from(200, &headers(&[]), b"x", now).is_none(), "no lifetime and no validator: not kept");
        assert!(stored_from(200, &headers(&[("Cache-Control", "no-store"), ("ETag", "\"a\"")]), b"x", now).is_none());
        assert!(stored_from(404, &etag_only, b"x", now).is_none(), "only a 200 is kept");
        assert!(stored_from(200, &etag_only, &vec![0u8; STORE_MAX + 1], now).is_none(), "too big to keep");
    }

    #[test]
    fn a_fresh_copy_is_used_until_its_max_age_has_passed() {
        let t0 = Instant::now();
        let copy = Stored { body: b"old".to_vec(), validators: Validators { etag: Some("\"v1\"".into()), last_modified: None }, max_age: Duration::from_secs(60), stored: t0 };
        assert_eq!(plan(Some(&copy), t0 + Duration::from_secs(30)), Plan::Fresh(b"old".to_vec()));
        // the age must be strictly below max-age: at 60 s the copy is stale
        assert_eq!(plan(Some(&copy), t0 + Duration::from_secs(60)), Plan::Revalidate(vec![("If-None-Match", "\"v1\"".to_string())]));
    }

    #[test]
    fn a_stale_copy_without_validators_is_fetched_again() {
        let t0 = Instant::now();
        let copy = Stored { body: vec![1], validators: Validators::default(), max_age: Duration::from_secs(10), stored: t0 };
        assert_eq!(plan(Some(&copy), t0 + Duration::from_secs(10)), Plan::Fetch);
        assert_eq!(plan(None, t0), Plan::Fetch);
    }

    #[test]
    fn a_stale_copy_revalidates_with_both_validators() {
        let t0 = Instant::now();
        let v = Validators { etag: Some("\"e\"".into()), last_modified: Some("Mon, 05 Oct 2026 10:00:00 GMT".into()) };
        let headers = conditional_headers(&v);
        assert_eq!(headers, vec![("If-None-Match", "\"e\"".to_string()), ("If-Modified-Since", "Mon, 05 Oct 2026 10:00:00 GMT".to_string())]);
        let copy = Stored { body: vec![], validators: v, max_age: Duration::ZERO, stored: t0 };
        assert_eq!(plan(Some(&copy), t0), Plan::Revalidate(headers));
    }
}
