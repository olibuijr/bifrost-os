//! The `location` service: where the device is, for the apps the person let use location. The grant is the one in
//! the `privacy` service (Settings > Privacy > App Permissions > Location); the error model is
//! `bifrost_ui::location_auth`. The SDK client is `bifrost_app::core_location`.
//!
//! There is no GPS hardware, so a fix comes from one of two sources, tried in this order:
//! 1. a manual location the person set in Settings (`location.manual.enabled`, `.latitude`, `.longitude`, `.city`,
//!    `.country` in the shell prefs), accurate to 100 m;
//! 2. an IP geolocation lookup, **off by default** for privacy: it runs only when `location.ip.enabled` is `1` and
//!    `location.ip.endpoint` names an HTTPS endpoint answering JSON with `latitude`/`longitude` (or `lat`/`lon`) and
//!    optional `city` and `country`. It is fetched on a worker thread, at most every 10 minutes, and is accurate to
//!    about 25 km (city level).
//!
//! Methods (params are a JSON object):
//! - `status {}` returns `{status, authorized}` (names as in the `privacy` service).
//! - `current {}` returns one location, or an error when the app is not authorized or no source has a fix.
//! - `start {}` / `stop {}` begin and end live updates for the app. While started, a changed fix is pushed as the
//!   `didUpdateLocations` event (`{locations: [location]}`) and a failure as `didFailWithError` (`{code, domain,
//!   message}`), the second only once per run of the same failure.
//! - `reverse {latitude, longitude}` returns the `{city, country, latitude, longitude}` of the known place within
//!   50 km. Places are the manual and IP places; there is no worldwide gazetteer yet.
//!
//! A location is `{latitude, longitude, altitude: null, horizontal_accuracy, timestamp, source, city, country}`.
use super::{Event, Shell, SystemService};
use crate::shell::music::radio::{Fetch, Http};
use crate::shell::prefs::Prefs;
use crate::shell::services::privacy_permissions::{location_name, location_status};
use bifrost_core::json::Json;
use bifrost_ui::location_auth::{AuthorizationStatus, LocationError};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The service name.
pub const SERVICE: &str = "location";
/// The event for new locations.
pub const EVENT_UPDATE: &str = "didUpdateLocations";
/// The event for a failure.
pub const EVENT_FAIL: &str = "didFailWithError";

/// Accuracy of a manual location, in metres.
pub const MANUAL_ACCURACY: f64 = 100.0;
/// Accuracy of an IP lookup, in metres (city level).
pub const IP_ACCURACY: f64 = 25_000.0;
/// Seconds between IP lookups.
const IP_REFRESH: f64 = 600.0;
/// Farthest a reverse lookup reaches, in metres.
const REVERSE_RADIUS: f64 = 50_000.0;

/// One position fix.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    /// Radius of uncertainty in metres.
    pub horizontal_accuracy: f64,
    /// Unix seconds.
    pub timestamp: f64,
    /// `manual` or `ip`.
    pub source: &'static str,
    pub city: String,
    pub country: String,
}

impl Fix {
    fn same_place(&self, other: &Fix) -> bool {
        self.source == other.source && self.latitude == other.latitude && self.longitude == other.longitude
    }
}

/// The JSON of a fix.
pub fn fix_json(f: &Fix) -> Json {
    Json::Obj(vec![
        ("latitude".into(), Json::Num(f.latitude)),
        ("longitude".into(), Json::Num(f.longitude)),
        ("altitude".into(), Json::Null),
        ("horizontal_accuracy".into(), Json::Num(f.horizontal_accuracy)),
        ("timestamp".into(), Json::Num(f.timestamp)),
        ("source".into(), Json::Str(f.source.into())),
        ("city".into(), Json::Str(f.city.clone())),
        ("country".into(), Json::Str(f.country.clone())),
    ])
}

/// The manual location in the prefs, stamped `now`. `None` when it is off or incomplete or out of range.
pub fn manual_fix(prefs: &Prefs, now: f64) -> Option<Fix> {
    if !prefs.get_bool("location.manual.enabled", false) {
        return None;
    }
    let latitude: f64 = prefs.get_parsed("location.manual.latitude")?;
    let longitude: f64 = prefs.get_parsed("location.manual.longitude")?;
    valid(latitude, longitude)?;
    Some(Fix {
        latitude,
        longitude,
        horizontal_accuracy: MANUAL_ACCURACY,
        timestamp: now,
        source: "manual",
        city: prefs.get("location.manual.city").unwrap_or("").to_string(),
        country: prefs.get("location.manual.country").unwrap_or("").to_string(),
    })
}

fn valid(latitude: f64, longitude: f64) -> Option<()> {
    ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)).then_some(())
}

fn number(v: &Json) -> Option<f64> {
    match v {
        Json::Num(n) => Some(*n),
        Json::Str(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Reads an IP geolocation answer. `None` when it has no usable coordinates.
pub fn parse_ip_response(body: &[u8], now: f64) -> Option<Fix> {
    let json = bifrost_core::json::parse(std::str::from_utf8(body).ok()?).ok()?;
    let pick = |names: &[&str]| names.iter().find_map(|n| json.get(n).and_then(number));
    let text = |names: &[&str]| names.iter().find_map(|n| json.get(n).and_then(Json::as_str)).unwrap_or("").to_string();
    let latitude = pick(&["latitude", "lat"])?;
    let longitude = pick(&["longitude", "lon", "lng"])?;
    valid(latitude, longitude)?;
    Some(Fix {
        latitude,
        longitude,
        horizontal_accuracy: IP_ACCURACY,
        timestamp: now,
        source: "ip",
        city: text(&["city"]),
        country: text(&["country", "country_name"]),
    })
}

/// Great-circle distance in metres.
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let (dp, dl) = ((b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    6_371_000.0 * 2.0 * h.sqrt().asin()
}

/// The nearest place with a city within [`REVERSE_RADIUS`] of the point.
pub fn reverse(places: &[Fix], latitude: f64, longitude: f64) -> Option<&Fix> {
    places
        .iter()
        .filter(|p| !p.city.is_empty())
        .map(|p| (distance((p.latitude, p.longitude), (latitude, longitude)), p))
        .filter(|(d, _)| *d <= REVERSE_RADIUS)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
}

fn error_json(e: LocationError) -> Json {
    Json::Obj(vec![
        ("code".into(), Json::Num(e.code() as f64)),
        ("domain".into(), Json::Str(LocationError::DOMAIN.into())),
        ("message".into(), Json::Str(format!("{e:?}"))),
    ])
}

/// The error an app in this status gets instead of a location, if any.
fn refusal(status: AuthorizationStatus) -> Option<LocationError> {
    if status.is_authorized() {
        None
    } else {
        Some(LocationError::Denied)
    }
}

#[derive(Default)]
struct Watch {
    last: Option<Fix>,
    failed: Option<LocationError>,
}

/// The service state: who watches, and the IP lookup.
#[derive(Default)]
pub struct Location {
    watching: BTreeMap<String, Watch>,
    ip: Arc<Mutex<Option<Fix>>>,
    in_flight: Arc<AtomicBool>,
    last_try: Option<f64>,
}

impl Location {
    /// Answers one call. `fix` is the best fix now, `places` the known places for reverse lookups.
    pub fn answer(
        &mut self,
        status: AuthorizationStatus,
        app_id: &str,
        method: &str,
        params: &Json,
        fix: Option<&Fix>,
        places: &[Fix],
    ) -> Result<Json, String> {
        match method {
            "status" => Ok(Json::Obj(vec![
                ("status".into(), Json::Str(location_name(status).into())),
                ("authorized".into(), Json::Bool(status.is_authorized())),
            ])),
            "current" => {
                if let Some(e) = refusal(status) {
                    return Err(format!("{e:?}"));
                }
                fix.map(fix_json).ok_or_else(|| format!("{:?}", LocationError::LocationUnknown))
            }
            "start" => {
                self.watching.insert(app_id.to_string(), Watch::default());
                Ok(Json::Obj(vec![("started".into(), Json::Bool(true))]))
            }
            "stop" => Ok(Json::Obj(vec![("stopped".into(), Json::Bool(self.watching.remove(app_id).is_some()))])),
            "reverse" => {
                let lat = params.get("latitude").and_then(number).ok_or("latitude is required")?;
                let lon = params.get("longitude").and_then(number).ok_or("longitude is required")?;
                valid(lat, lon).ok_or("latitude or longitude is out of range")?;
                let p = reverse(places, lat, lon).ok_or_else(|| format!("{:?}", LocationError::GeocodeFoundNoResult))?;
                Ok(Json::Obj(vec![
                    ("city".into(), Json::Str(p.city.clone())),
                    ("country".into(), Json::Str(p.country.clone())),
                    ("latitude".into(), Json::Num(p.latitude)),
                    ("longitude".into(), Json::Num(p.longitude)),
                ]))
            }
            _ => Err(format!("location has no method {method}")),
        }
    }

    /// The events due now for the watching apps. `status_of` is each app's grant.
    pub fn updates(&mut self, status_of: impl Fn(&str) -> AuthorizationStatus, fix: Option<&Fix>) -> Vec<Event> {
        let mut out = Vec::new();
        for (app, w) in self.watching.iter_mut() {
            let problem = refusal(status_of(app)).or(if fix.is_none() { Some(LocationError::LocationUnknown) } else { None });
            let (Some(f), None) = (fix, problem) else {
                let e = problem.unwrap_or(LocationError::LocationUnknown);
                if w.failed != Some(e) {
                    w.failed = Some(e);
                    out.push(Event { app: Some(app.clone()), event: EVENT_FAIL.into(), body: error_json(e) });
                }
                continue;
            };
            w.failed = None;
            if !w.last.as_ref().is_some_and(|l| l.same_place(f)) {
                w.last = Some(f.clone());
                let body = Json::Obj(vec![("locations".into(), Json::Arr(vec![fix_json(f)]))]);
                out.push(Event { app: Some(app.clone()), event: EVENT_UPDATE.into(), body });
            }
        }
        out
    }

    /// Starts an IP lookup on a worker when it is on, due, and none runs.
    fn refresh_ip(&mut self, prefs: &Prefs, t: f64) {
        if !prefs.get_bool("location.ip.enabled", false) {
            return;
        }
        let Some(endpoint) = prefs.get("location.ip.endpoint").filter(|e| e.starts_with("https://")).map(str::to_string) else {
            return;
        };
        if self.last_try.is_some_and(|l| t - l < IP_REFRESH) || self.in_flight.swap(true, Ordering::SeqCst) {
            return;
        }
        self.last_try = Some(t);
        let (slot, flag) = (self.ip.clone(), self.in_flight.clone());
        std::thread::spawn(move || {
            if let Ok(body) = Http::default().get(&endpoint, 64 * 1024) {
                if let Some(f) = parse_ip_response(&body, now_unix()) {
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(f);
                    }
                }
            }
            flag.store(false, Ordering::SeqCst);
        });
    }

    /// The places known now: the manual one, then the IP one.
    fn places(&self, prefs: &Prefs) -> Vec<Fix> {
        let ip = self.ip.lock().ok().and_then(|g| g.clone());
        manual_fix(prefs, now_unix()).into_iter().chain(ip).collect()
    }
}

fn now_unix() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl SystemService for Location {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        self.refresh_ip(&shell.prefs, t);
        let places = self.places(&shell.prefs);
        let status = location_status(app_id);
        self.answer(status, app_id, method, params, places.first(), &places)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        if self.watching.is_empty() {
            return Vec::new();
        }
        self.refresh_ip(&shell.prefs, t);
        let places = self.places(&shell.prefs);
        self.updates(location_status, places.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHEN: AuthorizationStatus = AuthorizationStatus::AuthorizedWhenInUse;

    fn manual_prefs() -> Prefs {
        let mut p = Prefs::memory();
        p.set_many(&[
            ("location.manual.enabled", "1"),
            ("location.manual.latitude", "65.68"),
            ("location.manual.longitude", "-18.10"),
            ("location.manual.city", "Akureyri"),
            ("location.manual.country", "Iceland"),
        ])
        .unwrap();
        p
    }

    fn here() -> Fix {
        manual_fix(&manual_prefs(), 1000.0).unwrap()
    }

    fn call(s: &mut Location, st: AuthorizationStatus, m: &str, p: &Json, fix: Option<&Fix>) -> Result<Json, String> {
        s.answer(st, "a", m, p, fix, fix.map(std::slice::from_ref).unwrap_or(&[]))
    }

    #[test]
    fn a_manual_location_comes_from_the_prefs_with_its_accuracy_and_time() {
        let f = here();
        assert_eq!((f.latitude, f.longitude, f.city.as_str()), (65.68, -18.10, "Akureyri"));
        assert_eq!((f.horizontal_accuracy, f.timestamp, f.source), (MANUAL_ACCURACY, 1000.0, "manual"));
    }

    #[test]
    fn a_manual_location_that_is_off_or_out_of_range_is_no_fix() {
        assert!(manual_fix(&Prefs::memory(), 0.0).is_none());
        let mut p = manual_prefs();
        p.set("location.manual.latitude", "95").unwrap();
        assert!(manual_fix(&p, 0.0).is_none());
    }

    #[test]
    fn current_needs_a_grant_and_a_fix() {
        let mut s = Location::default();
        let f = here();
        let denied = call(&mut s, AuthorizationStatus::NotDetermined, "current", &Json::Null, Some(&f));
        assert_eq!(denied, Err("Denied".to_string()));
        assert_eq!(call(&mut s, WHEN, "current", &Json::Null, None), Err("LocationUnknown".to_string()));
        let ok = call(&mut s, WHEN, "current", &Json::Null, Some(&f)).unwrap();
        assert_eq!(ok.get("latitude"), Some(&Json::Num(65.68)));
        assert_eq!(ok.get("horizontal_accuracy"), Some(&Json::Num(MANUAL_ACCURACY)));
        assert_eq!(ok.get("source").and_then(Json::as_str), Some("manual"));
    }

    #[test]
    fn status_reports_the_privacy_grant() {
        let mut s = Location::default();
        let r = call(&mut s, AuthorizationStatus::Denied, "status", &Json::Null, None).unwrap();
        assert_eq!(r.get("status").and_then(Json::as_str), Some("denied"));
        assert_eq!(r.get("authorized"), Some(&Json::Bool(false)));
    }

    #[test]
    fn a_watching_app_gets_a_changed_fix_once() {
        let mut s = Location::default();
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        let f = here();
        let first = s.updates(|_| WHEN, Some(&f));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event, EVENT_UPDATE);
        assert_eq!(first[0].app.as_deref(), Some("a"));
        assert!(s.updates(|_| WHEN, Some(&f)).is_empty());
        let mut moved = f.clone();
        moved.latitude = 64.0;
        assert_eq!(s.updates(|_| WHEN, Some(&moved)).len(), 1);
    }

    #[test]
    fn a_failure_is_delivered_once_and_a_denied_grant_wins() {
        let mut s = Location::default();
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        let e = s.updates(|_| WHEN, None);
        assert_eq!(e[0].event, EVENT_FAIL);
        assert_eq!(e[0].body.get("code"), Some(&Json::Num(LocationError::LocationUnknown.code() as f64)));
        assert!(s.updates(|_| WHEN, None).is_empty());
        let d = s.updates(|_| AuthorizationStatus::Denied, Some(&here()));
        assert_eq!(d[0].body.get("code"), Some(&Json::Num(LocationError::Denied.code() as f64)));
    }

    #[test]
    fn stop_ends_updates() {
        let mut s = Location::default();
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        let r = call(&mut s, WHEN, "stop", &Json::Null, None).unwrap();
        assert_eq!(r.get("stopped"), Some(&Json::Bool(true)));
        assert!(s.updates(|_| WHEN, Some(&here())).is_empty());
    }

    #[test]
    fn an_ip_answer_is_read_at_city_accuracy() {
        let body = br#"{"lat": 64.14, "lon": "-21.94", "city": "Reykjavik", "country_name": "Iceland"}"#;
        let f = parse_ip_response(body, 5.0).unwrap();
        assert_eq!((f.latitude, f.longitude, f.source), (64.14, -21.94, "ip"));
        assert_eq!((f.city.as_str(), f.country.as_str(), f.horizontal_accuracy), ("Reykjavik", "Iceland", IP_ACCURACY));
        assert!(parse_ip_response(b"{}", 0.0).is_none());
        assert!(parse_ip_response(br#"{"latitude": 200, "longitude": 0}"#, 0.0).is_none());
        assert!(parse_ip_response(b"nope", 0.0).is_none());
    }

    #[test]
    fn reverse_geocoding_finds_the_known_city_near_the_point() {
        let mut s = Location::default();
        let f = here();
        let near = Json::Obj(vec![("latitude".into(), Json::Num(65.70)), ("longitude".into(), Json::Num(-18.15))]);
        let r = call(&mut s, WHEN, "reverse", &near, Some(&f)).unwrap();
        assert_eq!(r.get("city").and_then(Json::as_str), Some("Akureyri"));
        let far = Json::Obj(vec![("latitude".into(), Json::Num(64.14)), ("longitude".into(), Json::Num(-21.94))]);
        assert_eq!(call(&mut s, WHEN, "reverse", &far, Some(&f)), Err("GeocodeFoundNoResult".to_string()));
        assert!(call(&mut s, WHEN, "reverse", &Json::Null, Some(&f)).is_err());
    }

    #[test]
    fn distance_is_about_right() {
        let d = distance((64.14, -21.94), (65.68, -18.10));
        assert!((d - 250_000.0).abs() < 15_000.0, "{d}");
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(call(&mut Location::default(), WHEN, "nope", &Json::Null, None).is_err());
    }
}
