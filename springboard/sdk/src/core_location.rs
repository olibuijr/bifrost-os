//! Core Location for apps: `CLLocationManager`, `CLLocation` and `CLGeocoder`-style reverse geocoding over the shell's
//! `location` service (`springboard/src/shell/services/location.rs`).
//!
//! ```ignore
//! let mut manager = LocationManager::new();
//! manager.desired_accuracy = accuracy::HUNDRED_METERS;
//! manager.request_when_in_use_authorization(ctx);      // the privacy sheet; the answer arrives in service_reply
//! manager.start_updating_location(ctx);
//! // in App::service_reply: manager.handle_reply(&result, &mut delegate)
//! // in App::service_event: manager.handle_event(service, event, body, &mut delegate)
//! ```
//! The person decides authorization (Settings > Privacy > App Permissions > Location); an app that has not been
//! answered reads `NotDetermined`. There is no GPS hardware and no network lookup: a location is the fixed location the
//! person set, so `horizontal_accuracy` is 100 m. An app without Precise Location gets the coordinates moved to a
//! grid of about 3 km (accuracy about 3 km), and the global Location Services switch off denies every app. `altitude`
//! is always `None`. Heading, regions and beacons are not provided. Decoding is pure, so everything here is testable
//! without a shell.
pub use crate::controls::location_auth::{AuthorizationStatus, LocationError};
use crate::privacy_permissions;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "location";
/// The event that carries new locations.
pub const EVENT_UPDATE: &str = "didUpdateLocations";
/// The event that carries a failure.
pub const EVENT_FAIL: &str = "didFailWithError";

/// `desiredAccuracy` values, in metres (`kCLLocationAccuracy*`). The best levels are negative, as on the reference.
pub mod accuracy {
    pub const BEST_FOR_NAVIGATION: f64 = -2.0;
    pub const BEST: f64 = -1.0;
    pub const NEAREST_TEN_METERS: f64 = 10.0;
    pub const HUNDRED_METERS: f64 = 100.0;
    pub const KILOMETER: f64 = 1000.0;
    pub const THREE_KILOMETERS: f64 = 3000.0;
}

/// `distanceFilter` that reports every update (`kCLDistanceFilterNone`).
pub const DISTANCE_FILTER_NONE: f64 = -1.0;

/// A latitude and longitude in degrees (`CLLocationCoordinate2D`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}

impl Coordinate {
    pub fn new(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate { latitude, longitude }
    }

    /// Whether both parts are in range (`CLLocationCoordinate2DIsValid`).
    pub fn is_valid(&self) -> bool {
        (-90.0..=90.0).contains(&self.latitude) && (-180.0..=180.0).contains(&self.longitude)
    }

    /// Great-circle distance to another coordinate, in metres.
    pub fn distance_to(&self, other: &Coordinate) -> f64 {
        let (p1, p2) = (self.latitude.to_radians(), other.latitude.to_radians());
        let dp = (other.latitude - self.latitude).to_radians();
        let dl = (other.longitude - self.longitude).to_radians();
        let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        6_371_000.0 * 2.0 * h.sqrt().asin()
    }
}

/// One location (`CLLocation`).
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub coordinate: Coordinate,
    /// Metres above sea level; `None` because there is no altitude source.
    pub altitude: Option<f64>,
    /// Radius of uncertainty in metres; negative would mean invalid.
    pub horizontal_accuracy: f64,
    /// Unix seconds.
    pub timestamp: f64,
    /// Where the fix came from: `manual` or `ip`.
    pub source: String,
    /// The city and country the source named, empty when it named none.
    pub city: String,
    pub country: String,
}

impl Location {
    /// Distance to another location in metres (`distanceFromLocation:`).
    pub fn distance_from(&self, other: &Location) -> f64 {
        self.coordinate.distance_to(&other.coordinate)
    }
}

/// A place a coordinate resolves to (`CLPlacemark`, city level).
#[derive(Clone, Debug, PartialEq)]
pub struct Placemark {
    pub locality: String,
    pub country: String,
    pub coordinate: Coordinate,
}

/// The callbacks of `CLLocationManagerDelegate`. Every method has an empty default.
pub trait Delegate {
    /// `locationManager:didUpdateLocations:`, oldest first.
    fn did_update_locations(&mut self, _locations: &[Location]) {}
    /// `locationManager:didFailWithError:`.
    fn did_fail_with_error(&mut self, _error: LocationError) {}
    /// `locationManagerDidChangeAuthorization:`.
    fn did_change_authorization(&mut self, _status: AuthorizationStatus) {}
}

/// The state of one app's location manager.
#[derive(Clone, Debug, PartialEq)]
pub struct LocationManager {
    /// The accuracy the app wants (`desiredAccuracy`). A source cannot be better than it is, so this only records intent.
    pub desired_accuracy: f64,
    /// The least distance in metres between two delivered updates (`distanceFilter`).
    pub distance_filter: f64,
    status: AuthorizationStatus,
    updating: bool,
    last: Option<Location>,
}

impl Default for LocationManager {
    fn default() -> Self {
        LocationManager::new()
    }
}

impl LocationManager {
    pub fn new() -> LocationManager {
        LocationManager {
            desired_accuracy: accuracy::BEST,
            distance_filter: DISTANCE_FILTER_NONE,
            status: AuthorizationStatus::NotDetermined,
            updating: false,
            last: None,
        }
    }

    /// `authorizationStatus`, as of the last reply.
    pub fn authorization_status(&self) -> AuthorizationStatus {
        self.status
    }

    /// The last delivered location (`location`).
    pub fn location(&self) -> Option<&Location> {
        self.last.as_ref()
    }

    /// Whether live updates are on.
    pub fn is_updating(&self) -> bool {
        self.updating
    }

    /// Asks for the status. The reply goes to [`handle_reply`](Self::handle_reply).
    pub fn refresh_authorization(&self, ctx: &Ctx) -> u32 {
        privacy_permissions::location_status(ctx)
    }

    /// `requestWhenInUseAuthorization`: shows the Allow sheet when the person has not answered yet.
    pub fn request_when_in_use_authorization(&self, ctx: &Ctx) -> u32 {
        privacy_permissions::location_request(ctx)
    }

    /// `startUpdatingLocation`. Does nothing if the app is not authorized; the delegate then gets the failure when the
    /// first event arrives, as the shell reports it.
    pub fn start_updating_location(&mut self, ctx: &Ctx) -> u32 {
        self.updating = true;
        service::call(ctx, SERVICE, "start", &Json::Null)
    }

    /// `stopUpdatingLocation`.
    pub fn stop_updating_location(&mut self, ctx: &Ctx) -> u32 {
        self.updating = false;
        service::call(ctx, SERVICE, "stop", &Json::Null)
    }

    /// `requestLocation`: one location. The reply goes to [`handle_reply`](Self::handle_reply).
    pub fn request_location(&self, ctx: &Ctx) -> u32 {
        service::call(ctx, SERVICE, "current", &Json::Null)
    }

    /// Takes a reply to any call above. A `status` reply updates the authorization; a location reply is delivered like
    /// an update; an error message that names a [`LocationError`] reaches the delegate. Returns whether it was one of
    /// ours.
    pub fn handle_reply(&mut self, result: &Result<Json, String>, delegate: &mut dyn Delegate) -> bool {
        match result {
            Err(message) => match error_named(message) {
                Some(e) => {
                    delegate.did_fail_with_error(e);
                    true
                }
                None => false,
            },
            Ok(body) => {
                if let Some(status) = privacy_permissions::decode_location(body) {
                    self.set_status(status, delegate);
                    true
                } else if let Some(location) = decode_location(body) {
                    self.deliver(vec![location], delegate);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Takes a service event. Returns whether it was a location event.
    pub fn handle_event(&mut self, service: &str, event: &str, body: &Json, delegate: &mut dyn Delegate) -> bool {
        if service != SERVICE {
            return false;
        }
        match event {
            EVENT_UPDATE if self.updating => {
                self.deliver(decode_locations(body), delegate);
                true
            }
            EVENT_UPDATE => true,
            EVENT_FAIL => {
                if self.updating {
                    if let Some(e) = decode_error(body) {
                        delegate.did_fail_with_error(e);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn set_status(&mut self, status: AuthorizationStatus, delegate: &mut dyn Delegate) {
        if self.status != status {
            self.status = status;
            delegate.did_change_authorization(status);
        }
    }

    /// Delivers the locations that moved at least `distance_filter` from the last delivered one.
    fn deliver(&mut self, locations: Vec<Location>, delegate: &mut dyn Delegate) {
        let mut out = Vec::new();
        for l in locations {
            let moved = match &self.last {
                Some(last) if self.distance_filter >= 0.0 => last.distance_from(&l) >= self.distance_filter,
                _ => true,
            };
            if moved {
                self.last = Some(l.clone());
                out.push(l);
            }
        }
        if !out.is_empty() {
            delegate.did_update_locations(&out);
        }
    }
}

/// Asks the shell which place a coordinate is in (`reverseGeocodeLocation:`, city level). The reply decodes with
/// [`decode_placemark`]; an error reply names [`LocationError::GeocodeFoundNoResult`] when no known place is near.
pub fn reverse_geocode(ctx: &Ctx, coordinate: Coordinate) -> u32 {
    let params = Json::Obj(vec![
        ("latitude".into(), Json::Num(coordinate.latitude)),
        ("longitude".into(), Json::Num(coordinate.longitude)),
    ]);
    service::call(ctx, SERVICE, "reverse", &params)
}

/// Asks the shell which known places an address names (`geocodeAddressString:`). The reply decodes with
/// [`decode_placemarks`]; an error reply names [`LocationError::GeocodeFoundNoResult`] when nothing matches.
pub fn geocode_address(ctx: &Ctx, address: &str) -> u32 {
    let params = Json::Obj(vec![("address".into(), Json::Str(address.into()))]);
    service::call(ctx, SERVICE, "geocode", &params)
}

/// The location in a `current` reply or an update, or `None` for another shape or an invalid coordinate.
pub fn decode_location(body: &Json) -> Option<Location> {
    let num = |k: &str| match body.get(k)? {
        Json::Num(n) => Some(*n),
        _ => None,
    };
    let coordinate = Coordinate::new(num("latitude")?, num("longitude")?);
    if !coordinate.is_valid() {
        return None;
    }
    let text = |k: &str| body.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    Some(Location {
        coordinate,
        altitude: num("altitude"),
        horizontal_accuracy: num("horizontal_accuracy")?,
        timestamp: num("timestamp")?,
        source: text("source"),
        city: text("city"),
        country: text("country"),
    })
}

/// The locations of a `didUpdateLocations` body; entries of another shape are skipped.
pub fn decode_locations(body: &Json) -> Vec<Location> {
    body.get("locations").and_then(Json::as_array).map(|a| a.iter().filter_map(decode_location).collect()).unwrap_or_default()
}

/// The placemark in a `reverse` reply, or `None` for another shape.
pub fn decode_placemark(body: &Json) -> Option<Placemark> {
    let n = |k: &str| match body.get(k)? {
        Json::Num(v) => Some(*v),
        _ => None,
    };
    Some(Placemark {
        locality: body.get("city")?.as_str()?.to_string(),
        country: body.get("country").and_then(Json::as_str).unwrap_or("").to_string(),
        coordinate: Coordinate::new(n("latitude")?, n("longitude")?),
    })
}

/// The placemarks in a `geocode` reply; entries of another shape are skipped.
pub fn decode_placemarks(body: &Json) -> Vec<Placemark> {
    body.get("places").and_then(Json::as_array).map(|a| a.iter().filter_map(decode_placemark).collect()).unwrap_or_default()
}

/// The error in a `didFailWithError` body, or `None` for another shape or an unknown code.
pub fn decode_error(body: &Json) -> Option<LocationError> {
    match body.get("code")? {
        Json::Num(c) if *c >= 0.0 => LocationError::from_code(*c as u32),
        _ => None,
    }
}

/// The location error an error reply's message names (the case name, for example `Denied`).
pub fn error_named(message: &str) -> Option<LocationError> {
    crate::controls::location_auth::ERRORS.iter().copied().find(|e| format!("{e:?}") == message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Spy {
        updates: Vec<Vec<Location>>,
        errors: Vec<LocationError>,
        statuses: Vec<AuthorizationStatus>,
    }

    impl Delegate for Spy {
        fn did_update_locations(&mut self, l: &[Location]) {
            self.updates.push(l.to_vec());
        }
        fn did_fail_with_error(&mut self, e: LocationError) {
            self.errors.push(e);
        }
        fn did_change_authorization(&mut self, s: AuthorizationStatus) {
            self.statuses.push(s);
        }
    }

    fn loc_json(lat: f64, lon: f64) -> Json {
        Json::Obj(vec![
            ("latitude".into(), Json::Num(lat)),
            ("longitude".into(), Json::Num(lon)),
            ("altitude".into(), Json::Null),
            ("horizontal_accuracy".into(), Json::Num(100.0)),
            ("timestamp".into(), Json::Num(1000.0)),
            ("source".into(), Json::Str("manual".into())),
            ("city".into(), Json::Str("Akureyri".into())),
            ("country".into(), Json::Str("Iceland".into())),
        ])
    }

    fn update(lat: f64, lon: f64) -> Json {
        Json::Obj(vec![("locations".into(), Json::Arr(vec![loc_json(lat, lon)]))])
    }

    fn started() -> LocationManager {
        let mut m = LocationManager::new();
        m.updating = true;
        m
    }

    #[test]
    fn a_new_manager_is_not_determined_and_wants_the_best_accuracy() {
        let m = LocationManager::new();
        assert_eq!(m.authorization_status(), AuthorizationStatus::NotDetermined);
        assert_eq!((m.desired_accuracy, m.distance_filter), (accuracy::BEST, DISTANCE_FILTER_NONE));
        assert!(m.location().is_none() && !m.is_updating());
    }

    #[test]
    fn a_status_reply_moves_the_state_and_tells_the_delegate_once() {
        let (mut m, mut d) = (LocationManager::new(), Spy::default());
        let reply = Ok(Json::Obj(vec![("status".into(), Json::Str("authorized_when_in_use".into()))]));
        assert!(m.handle_reply(&reply, &mut d));
        assert!(m.handle_reply(&reply, &mut d));
        assert_eq!(m.authorization_status(), AuthorizationStatus::AuthorizedWhenInUse);
        assert_eq!(d.statuses, vec![AuthorizationStatus::AuthorizedWhenInUse]);
        let denied = Ok(Json::Obj(vec![("status".into(), Json::Str("denied".into()))]));
        m.handle_reply(&denied, &mut d);
        assert_eq!(d.statuses.len(), 2);
    }

    #[test]
    fn an_update_event_reaches_the_delegate_with_all_fields() {
        let (mut m, mut d) = (started(), Spy::default());
        assert!(m.handle_event(SERVICE, EVENT_UPDATE, &update(65.68, -18.10), &mut d));
        let l = &d.updates[0][0];
        assert_eq!(l.coordinate, Coordinate::new(65.68, -18.10));
        assert_eq!((l.horizontal_accuracy, l.timestamp, l.altitude), (100.0, 1000.0, None));
        assert_eq!((l.source.as_str(), l.city.as_str()), ("manual", "Akureyri"));
        assert_eq!(m.location(), Some(l));
    }

    #[test]
    fn updates_are_ignored_when_not_updating_and_other_services_are_not_ours() {
        let (mut m, mut d) = (LocationManager::new(), Spy::default());
        assert!(m.handle_event(SERVICE, EVENT_UPDATE, &update(65.0, -18.0), &mut d));
        assert!(d.updates.is_empty());
        assert!(!m.handle_event("network", EVENT_UPDATE, &update(65.0, -18.0), &mut d));
        assert!(!m.handle_event(SERVICE, "other", &Json::Null, &mut d));
    }

    #[test]
    fn the_distance_filter_drops_small_moves() {
        let (mut m, mut d) = (started(), Spy::default());
        m.distance_filter = 5_000.0;
        m.handle_event(SERVICE, EVENT_UPDATE, &update(65.68, -18.10), &mut d);
        m.handle_event(SERVICE, EVENT_UPDATE, &update(65.69, -18.10), &mut d);
        assert_eq!(d.updates.len(), 1);
        m.handle_event(SERVICE, EVENT_UPDATE, &update(64.14, -21.94), &mut d);
        assert_eq!(d.updates.len(), 2);
    }

    #[test]
    fn a_failure_event_maps_the_code_to_the_error() {
        let (mut m, mut d) = (started(), Spy::default());
        let body = Json::Obj(vec![("code".into(), Json::Num(LocationError::Denied.code() as f64))]);
        m.handle_event(SERVICE, EVENT_FAIL, &body, &mut d);
        assert_eq!(d.errors, vec![LocationError::Denied]);
    }

    #[test]
    fn an_error_reply_that_names_a_case_reaches_the_delegate() {
        let (mut m, mut d) = (LocationManager::new(), Spy::default());
        assert!(m.handle_reply(&Err("LocationUnknown".into()), &mut d));
        assert!(!m.handle_reply(&Err("something else".into()), &mut d));
        assert_eq!(d.errors, vec![LocationError::LocationUnknown]);
    }

    #[test]
    fn a_current_reply_is_delivered_like_an_update() {
        let (mut m, mut d) = (LocationManager::new(), Spy::default());
        assert!(m.handle_reply(&Ok(loc_json(64.14, -21.94)), &mut d));
        assert_eq!(d.updates.len(), 1);
    }

    #[test]
    fn decoding_rejects_an_invalid_coordinate_and_reads_a_placemark() {
        assert!(decode_location(&loc_json(95.0, 0.0)).is_none());
        assert!(decode_location(&Json::Null).is_none());
        let body = Json::Obj(vec![
            ("city".into(), Json::Str("Akureyri".into())),
            ("country".into(), Json::Str("Iceland".into())),
            ("latitude".into(), Json::Num(65.68)),
            ("longitude".into(), Json::Num(-18.10)),
        ]);
        let p = decode_placemark(&body).unwrap();
        assert_eq!((p.locality.as_str(), p.country.as_str()), ("Akureyri", "Iceland"));
    }

    #[test]
    fn a_geocode_reply_decodes_to_every_placemark_and_skips_other_shapes() {
        let place = |city: &str| {
            Json::Obj(vec![
                ("city".into(), Json::Str(city.into())),
                ("country".into(), Json::Str("Iceland".into())),
                ("latitude".into(), Json::Num(65.68)),
                ("longitude".into(), Json::Num(-18.10)),
            ])
        };
        let body = Json::Obj(vec![("places".into(), Json::Arr(vec![place("Akureyri"), Json::Null, place("Husavik")]))]);
        let found = decode_placemarks(&body);
        assert_eq!(found.iter().map(|p| p.locality.as_str()).collect::<Vec<_>>(), ["Akureyri", "Husavik"]);
        assert!(decode_placemarks(&Json::Null).is_empty());
    }

    #[test]
    fn distance_between_two_cities_is_about_right() {
        let d = Coordinate::new(64.14, -21.94).distance_to(&Coordinate::new(65.68, -18.10));
        assert!((d - 250_000.0).abs() < 15_000.0, "{d}");
    }
}
