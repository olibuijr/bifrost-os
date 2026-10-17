//! The `location` service: where the device is, for the apps the person let use location. The grant is the one in
//! the `privacy` service (Settings > Privacy > App Permissions > Location); the error model is
//! `bifrost_ui::location_auth`. The SDK client is `bifrost_app::core_location`.
//!
//! There is no GPS hardware and no network lookup. The only source is a fixed location the person sets in Settings
//! (`location.manual.enabled`, `.latitude`, `.longitude`, `.city`, `.country` in the shell prefs), accurate to 100 m.
//! No third-party service is called, and the IP-address estimate is gone. A GeoClue2 client is not built: the repo has
//! no D-Bus client for it in use yet.
//!
//! Privacy rules (owner decision, follows the reference platform):
//! - `location.services.enabled` (default `1`) is the global Location Services switch. `0` denies every app.
//! - Each app is approximate by default. `location.precise.<app id>` set to `1` turns on its Precise Location. An
//!   approximate app gets its coordinates on a grid of about 3 km (`bifrost_ui::location_auth::approximate`).
//! - Nothing is stored beyond the last fix in memory. The only per-app record is when each app last received a fix
//!   (`Location::last_use`), kept in memory and never written to disk.
//!
//! Methods (params are a JSON object):
//! - `status {}` returns `{status, authorized}` (names as in the `privacy` service).
//! - `current {}` returns one location, or an error when the app is not authorized or no source has a fix.
//! - `start {}` / `stop {}` begin and end live updates for the app. While started, a changed fix is pushed as the
//!   `didUpdateLocations` event (`{locations: [location]}`) and a failure as `didFailWithError` (`{code, domain,
//!   message}`), the second only once per run of the same failure.
//! - `reverse {latitude, longitude}` returns the `{city, country, latitude, longitude}` of the known place within
//!   50 km. The known place is the fixed location.
//!
//! A location is `{latitude, longitude, altitude: null, horizontal_accuracy, timestamp, source, city, country}`.
use super::{Event, Shell, SystemService};
use crate::shell::prefs::Prefs;
use crate::shell::services::privacy_permissions::{location_name, location_status};
use bifrost_core::json::Json;
use bifrost_ui::location_auth::{accuracy_for, approximate, AccuracyAuthorization, AuthorizationStatus, LocationError, APPROXIMATE_ACCURACY};
use std::collections::BTreeMap;

/// The service name.
pub const SERVICE: &str = "location";
/// The event for new locations.
pub const EVENT_UPDATE: &str = "didUpdateLocations";
/// The event for a failure.
pub const EVENT_FAIL: &str = "didFailWithError";
/// The prefs key of the global Location Services switch (`1` on, the default).
pub const PREFS_SERVICES: &str = "location.services.enabled";
/// Prefix of the per-app Precise Location switch: `location.precise.<app id>` (`1` on, the default is off).
pub const PREFS_PRECISE_PREFIX: &str = "location.precise.";

/// Accuracy of a manual location, in metres.
pub const MANUAL_ACCURACY: f64 = 100.0;
/// Farthest a reverse lookup reaches, in metres.
const REVERSE_RADIUS: f64 = 50_000.0;
/// How long after a fix an app still counts as using location for the menu bar indicator, in seconds.
pub const ACTIVE_WINDOW: f64 = 5.0;

thread_local! {
    /// The apps using location as of the last service call or tick. The menu bar reads it through `in_use_now`.
    static IN_USE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// When each app last got a fix this run (Unix seconds). Kept in memory only: nothing is written to disk.
    static LAST_USE: std::cell::RefCell<BTreeMap<String, f64>> = const { std::cell::RefCell::new(BTreeMap::new()) };
}

/// Records that `app_id` got a fix at `t` (Unix seconds). Memory only, see the module note.
pub fn record_use(app_id: &str, t: f64) {
    LAST_USE.with(|u| {
        u.borrow_mut().insert(app_id.to_string(), t);
    });
}

/// When `app_id` last got a fix this run, in Unix seconds. `None` when it never has.
pub fn last_use_of(app_id: &str) -> Option<f64> {
    LAST_USE.with(|u| u.borrow().get(app_id).copied())
}

/// Whether `last` (a fix time) is within `window` seconds before `now`. A fix in the future does not count.
pub fn used_within(last: Option<f64>, now: f64, window: f64) -> bool {
    last.is_some_and(|u| now >= u && now - u <= window)
}

/// Records the apps using location now, for the menu bar.
pub fn publish_in_use(apps: Vec<String>) {
    IN_USE.with(|s| *s.borrow_mut() = apps);
}

/// The apps using location as the service last published them (sorted by id). Empty means the indicator is hidden.
pub fn in_use_now() -> Vec<String> {
    IN_USE.with(|s| s.borrow().clone())
}

/// One position fix.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    /// Radius of uncertainty in metres.
    pub horizontal_accuracy: f64,
    /// Unix seconds.
    pub timestamp: f64,
    /// Always `manual` for now.
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

/// Whether the global Location Services switch is on.
pub fn services_on(prefs: &Prefs) -> bool {
    prefs.get_bool(PREFS_SERVICES, true)
}

/// Whether the app has Precise Location on (default off).
pub fn precise_for(prefs: &Prefs, app_id: &str) -> bool {
    prefs.get_bool(&format!("{PREFS_PRECISE_PREFIX}{app_id}"), false)
}

/// The status an app is held to: the global switch off denies every app, whatever its grant.
pub fn effective_status(status: AuthorizationStatus, services_on: bool) -> AuthorizationStatus {
    if services_on {
        status
    } else {
        AuthorizationStatus::Denied
    }
}

/// The fix an app receives: unchanged when it has Precise Location, otherwise moved to the approximate grid and
/// reported with the grid's accuracy (never finer than the grid).
pub fn fix_for(fix: &Fix, accuracy: AccuracyAuthorization) -> Fix {
    if accuracy.is_full() {
        return fix.clone();
    }
    let (latitude, longitude) = approximate(fix.latitude, fix.longitude);
    Fix { latitude, longitude, horizontal_accuracy: fix.horizontal_accuracy.max(APPROXIMATE_ACCURACY), ..fix.clone() }
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

/// The service state: who watches, and when each app last got a fix. Kept in memory only.
#[derive(Default)]
pub struct Location {
    watching: BTreeMap<String, Watch>,
}

impl Location {
    /// When `app_id` last received a fix this run, in Unix seconds. `None` when it never has.
    pub fn last_use(&self, app_id: &str) -> Option<f64> {
        last_use_of(app_id)
    }

    /// The apps using location now, sorted by id: each one with a live watch, or with a fix within `window` seconds of
    /// `t`. Drives the menu bar location indicator, which shows while this list is not empty. Nothing is recorded here.
    pub fn in_use(&self, t: f64, window: f64) -> Vec<String> {
        let recent: Vec<String> = LAST_USE.with(|u| u.borrow().iter().filter(|&(_, &u)| t >= u && t - u <= window).map(|(a, _)| a.clone()).collect());
        let mut apps: Vec<String> = self.watching.keys().cloned().chain(recent).collect();
        apps.sort();
        apps.dedup();
        apps
    }

    /// Answers one call. `fix` is the best fix now, already shaped for this app; `places` the known places for
    /// reverse lookups; `t` the time in Unix seconds, recorded as the app's last use when it gets a fix.
    pub fn answer(
        &mut self,
        status: AuthorizationStatus,
        app_id: &str,
        method: &str,
        params: &Json,
        fix: Option<&Fix>,
        places: &[Fix],
        t: f64,
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
                let f = fix.ok_or_else(|| format!("{:?}", LocationError::LocationUnknown))?;
                record_use(app_id, t);
                Ok(fix_json(f))
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

    /// The events due now for the watching apps. `status_of` is each app's effective grant, `accuracy_of` its
    /// Precise Location setting, and `fix` the best fix (shaped per app here).
    pub fn updates(
        &mut self,
        status_of: impl Fn(&str) -> AuthorizationStatus,
        accuracy_of: impl Fn(&str) -> AccuracyAuthorization,
        fix: Option<&Fix>,
        t: f64,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        let mut used = Vec::new();
        for (app, w) in self.watching.iter_mut() {
            let problem = refusal(status_of(app)).or(if fix.is_none() { Some(LocationError::LocationUnknown) } else { None });
            let (Some(raw), None) = (fix, problem) else {
                let e = problem.unwrap_or(LocationError::LocationUnknown);
                if w.failed != Some(e) {
                    w.failed = Some(e);
                    out.push(Event { app: Some(app.clone()), event: EVENT_FAIL.into(), body: error_json(e) });
                }
                continue;
            };
            w.failed = None;
            let f = fix_for(raw, accuracy_of(app));
            used.push(app.clone());
            if !w.last.as_ref().is_some_and(|l| l.same_place(&f)) {
                let body = Json::Obj(vec![("locations".into(), Json::Arr(vec![fix_json(&f)]))]);
                w.last = Some(f);
                out.push(Event { app: Some(app.clone()), event: EVENT_UPDATE.into(), body });
            }
        }
        for app in used {
            record_use(&app, t);
        }
        out
    }

    /// The fixed location, the only source: the places known now.
    fn places(&self, prefs: &Prefs) -> Vec<Fix> {
        manual_fix(prefs, now_unix()).into_iter().collect()
    }
}

/// The current time in Unix seconds (0 when the clock is before 1970).
pub fn now_unix() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl SystemService for Location {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        let places = self.places(&shell.prefs);
        let status = effective_status(location_status(app_id), services_on(&shell.prefs));
        let accuracy = accuracy_for(precise_for(&shell.prefs, app_id));
        let shaped = places.first().map(|f| fix_for(f, accuracy));
        let reply = self.answer(status, app_id, method, params, shaped.as_ref(), &places, t);
        publish_in_use(self.in_use(t, ACTIVE_WINDOW));
        reply
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        publish_in_use(self.in_use(t, ACTIVE_WINDOW));
        if self.watching.is_empty() {
            return Vec::new();
        }
        let places = self.places(&shell.prefs);
        let on = services_on(&shell.prefs);
        let prefs = &shell.prefs;
        self.updates(
            |app| effective_status(location_status(app), on),
            |app| accuracy_for(precise_for(prefs, app)),
            places.first(),
            t,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHEN: AuthorizationStatus = AuthorizationStatus::AuthorizedWhenInUse;
    const FULL: AccuracyAuthorization = AccuracyAuthorization::FullAccuracy;
    const REDUCED: AccuracyAuthorization = AccuracyAuthorization::ReducedAccuracy;

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

    /// Calls `answer` for app "a" at time 0 with the fix shaped for the given accuracy.
    fn call(s: &mut Location, st: AuthorizationStatus, m: &str, p: &Json, fix: Option<&Fix>) -> Result<Json, String> {
        let shaped = fix.map(|f| fix_for(f, FULL));
        s.answer(st, "a", m, p, shaped.as_ref(), fix.map(std::slice::from_ref).unwrap_or(&[]), 0.0)
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
    fn the_global_switch_off_denies_an_authorized_app() {
        assert_eq!(effective_status(WHEN, false), AuthorizationStatus::Denied);
        assert_eq!(effective_status(WHEN, true), WHEN);
        let mut s = Location::default();
        let f = here();
        let r = s.answer(effective_status(WHEN, false), "a", "current", &Json::Null, Some(&f), &[], 0.0);
        assert_eq!(r, Err("Denied".to_string()));
        assert!(s.last_use("a").is_none());
    }

    #[test]
    fn approximate_is_the_default_and_precise_is_read_per_app() {
        let mut p = manual_prefs();
        assert!(!precise_for(&p, "a"));
        p.set("location.precise.a", "1").unwrap();
        assert!(precise_for(&p, "a"));
        assert!(!precise_for(&p, "b"));
    }

    #[test]
    fn an_approximate_app_gets_the_grid_and_a_wider_accuracy() {
        let f = here();
        let rough = fix_for(&f, REDUCED);
        assert_ne!((rough.latitude, rough.longitude), (f.latitude, f.longitude));
        assert_eq!(rough.horizontal_accuracy, APPROXIMATE_ACCURACY);
        assert!(distance((f.latitude, f.longitude), (rough.latitude, rough.longitude)) <= 2_500.0);
        assert_eq!(fix_for(&f, FULL), f);
        assert_eq!(rough.city, "Akureyri");
    }

    #[test]
    fn a_watching_app_gets_a_changed_fix_once() {
        let mut s = Location::default();
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        let f = here();
        let first = s.updates(|_| WHEN, |_| FULL, Some(&f), 1.0);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event, EVENT_UPDATE);
        assert_eq!(first[0].app.as_deref(), Some("a"));
        assert!(s.updates(|_| WHEN, |_| FULL, Some(&f), 2.0).is_empty());
        let mut moved = f.clone();
        moved.latitude = 64.0;
        assert_eq!(s.updates(|_| WHEN, |_| FULL, Some(&moved), 3.0).len(), 1);
    }

    #[test]
    fn a_failure_is_delivered_once_and_a_denied_grant_wins() {
        let mut s = Location::default();
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        let e = s.updates(|_| WHEN, |_| FULL, None, 0.0);
        assert_eq!(e[0].event, EVENT_FAIL);
        assert_eq!(e[0].body.get("code"), Some(&Json::Num(LocationError::LocationUnknown.code() as f64)));
        assert!(s.updates(|_| WHEN, |_| FULL, None, 0.0).is_empty());
        let d = s.updates(|_| AuthorizationStatus::Denied, |_| FULL, Some(&here()), 0.0);
        assert_eq!(d[0].body.get("code"), Some(&Json::Num(LocationError::Denied.code() as f64)));
    }

    #[test]
    fn stop_ends_updates() {
        let mut s = Location::default();
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        let r = call(&mut s, WHEN, "stop", &Json::Null, None).unwrap();
        assert_eq!(r.get("stopped"), Some(&Json::Bool(true)));
        assert!(s.updates(|_| WHEN, |_| FULL, Some(&here()), 0.0).is_empty());
    }

    #[test]
    fn the_last_use_is_kept_in_memory_for_an_app_that_got_a_fix() {
        let mut s = Location::default();
        let f = here();
        assert!(s.last_use("a").is_none());
        s.answer(WHEN, "a", "current", &Json::Null, Some(&f), &[], 42.0).unwrap();
        assert_eq!(s.last_use("a"), Some(42.0));
        assert!(s.last_use("b").is_none());
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        s.updates(|_| WHEN, |_| FULL, Some(&f), 50.0);
        assert_eq!(s.last_use("a"), Some(50.0));
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
    fn the_indicator_lists_watching_apps_and_recent_fixes_only() {
        let mut s = Location::default();
        let f = here();
        assert!(s.in_use(10.0, ACTIVE_WINDOW).is_empty(), "no app, no indicator");
        s.answer(WHEN, "b", "current", &Json::Null, Some(&f), &[], 8.0).unwrap();
        assert_eq!(s.in_use(10.0, ACTIVE_WINDOW), ["b"], "a fix 2 s ago is in use");
        assert!(s.in_use(20.0, ACTIVE_WINDOW).is_empty(), "an old fix is not in use");
        call(&mut s, WHEN, "start", &Json::Null, None).unwrap();
        assert_eq!(s.in_use(20.0, ACTIVE_WINDOW), ["a"], "a live watch is in use, sorted");
        s.answer(WHEN, "a", "stop", &Json::Null, None, &[], 20.0).unwrap();
        assert!(s.in_use(20.0, ACTIVE_WINDOW).is_empty(), "stop ends the indicator");
        assert!(s.in_use(5.0, ACTIVE_WINDOW).is_empty(), "a fix in the future is not counted");
    }

    #[test]
    fn the_published_in_use_list_is_what_the_menu_bar_reads() {
        publish_in_use(vec!["a".into()]);
        assert_eq!(in_use_now(), ["a"]);
        publish_in_use(Vec::new());
        assert!(in_use_now().is_empty(), "an empty list hides the indicator");
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(call(&mut Location::default(), WHEN, "nope", &Json::Null, None).is_err());
    }
}
