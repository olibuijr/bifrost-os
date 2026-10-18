//! Location authorization and location errors: the status an app holds for location services, the accuracy the user
//! allowed, what a request does in each status, the usage text a prompt needs, and the error cases of the service.
//!
//! The names follow the official location framework: its authorization status (`notDetermined`, `restricted`,
//! `denied`, `authorizedAlways`, `authorizedWhenInUse`), its accuracy authorization (`fullAccuracy`,
//! `reducedAccuracy`) and the cases of its error type (for example `locationUnknown`, `denied`, `network`,
//! `geocodeCanceled`). The types hold and classify values only. No prompt is shown, no service is started and no error
//! is raised here; a shell that owns the permission feeds in the status it has.
//!
//! Numeric error codes are this module's own, numbered in declaration order from zero. The rules in
//! [`LocationError::needs_user_action`], [`LocationError::is_transient`] and [`UsageDescriptions::for_level`] are this
//! module's own rules and are not part of the official framework.

/// What an app may do with location services, as the user answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationStatus {
    /// The user has not answered yet. A request shows the prompt.
    NotDetermined,
    /// The app is not authorized to use location services.
    Restricted,
    /// The user refused, or location services are disabled for the whole device.
    Denied,
    /// The app may start location services at any time.
    AuthorizedAlways,
    /// The app may start location services while it is in use.
    AuthorizedWhenInUse,
}

impl AuthorizationStatus {
    /// Whether either authorized status holds.
    pub fn is_authorized(self) -> bool {
        matches!(self, Self::AuthorizedAlways | Self::AuthorizedWhenInUse)
    }

    /// Whether location services may start now. `in_foreground` says whether the app is in use, which only the
    /// when-in-use status depends on.
    pub fn may_start(self, in_foreground: bool) -> bool {
        match self {
            Self::AuthorizedAlways => true,
            Self::AuthorizedWhenInUse => in_foreground,
            Self::NotDetermined | Self::Restricted | Self::Denied => false,
        }
    }

    /// What asking for access does in this status.
    pub fn request_outcome(self) -> RequestOutcome {
        match self {
            Self::NotDetermined => RequestOutcome::Prompt,
            Self::AuthorizedAlways | Self::AuthorizedWhenInUse => RequestOutcome::AlreadyGranted,
            Self::Restricted | Self::Denied => RequestOutcome::Refused,
        }
    }
}

/// The result of asking for location access in a given status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestOutcome {
    /// The user has not answered, so the prompt is shown.
    Prompt,
    /// Access is already allowed; no prompt.
    AlreadyGranted,
    /// Access is refused; no prompt, and only the user in Settings can change it.
    Refused,
}

/// The accuracy the user allowed for an authorized app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccuracyAuthorization {
    FullAccuracy,
    ReducedAccuracy,
}

impl AccuracyAuthorization {
    /// Whether the app may read precise positions.
    pub fn is_full(self) -> bool {
        self == Self::FullAccuracy
    }
}

/// The two levels of authorization a usage text can explain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    WhenInUse,
    Always,
}

/// The message an app shows the user to explain why it asks for location, one for each level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageDescriptions {
    when_in_use: String,
    always: String,
}

/// A usage text that is needed but missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageError {
    /// The text for this level is empty or only spaces.
    Missing(Level),
}

impl UsageDescriptions {
    /// Stores both texts. Surrounding spaces are removed when they are read.
    pub fn new(when_in_use: &str, always: &str) -> UsageDescriptions {
        UsageDescriptions { when_in_use: when_in_use.to_string(), always: always.to_string() }
    }

    /// The text for `level`, trimmed. `Missing` when it is empty or only spaces: a request at that level must not
    /// show a prompt with no reason.
    pub fn for_level(&self, level: Level) -> Result<&str, UsageError> {
        let text = match level {
            Level::WhenInUse => self.when_in_use.trim(),
            Level::Always => self.always.trim(),
        };
        if text.is_empty() {
            Err(UsageError::Missing(level))
        } else {
            Ok(text)
        }
    }
}

/// Grid of the approximate location, in degrees of latitude (0.027 is about 3 km). An app without the Precise
/// Location switch gets its coordinates moved to the centre of this grid cell.
pub const APPROXIMATE_GRID_DEG: f64 = 0.027;
/// The accuracy an approximate fix reports, in metres: about half a grid cell, never finer than the grid.
pub const APPROXIMATE_ACCURACY: f64 = 3_000.0;

/// Rounds a coordinate pair to the centre of its approximate grid cell. Latitude stays within -90...90 and longitude
/// within -180...180, so a point near the edge keeps a valid position.
pub fn approximate(latitude: f64, longitude: f64) -> (f64, f64) {
    let snap = |v: f64| (v / APPROXIMATE_GRID_DEG).round() * APPROXIMATE_GRID_DEG;
    (snap(latitude).clamp(-90.0, 90.0), snap(longitude).clamp(-180.0, 180.0))
}

/// The accuracy an app gets for a fix: Precise Location keeps the fix; approximate uses the grid.
pub fn accuracy_for(precise: bool) -> AccuracyAuthorization {
    if precise {
        AccuracyAuthorization::FullAccuracy
    } else {
        AccuracyAuthorization::ReducedAccuracy
    }
}

/// The purpose text a request must show: the app's own text for the level, or `Missing`. A request with no purpose
/// string is refused, so the person is never asked without a reason.
pub fn request_purpose(texts: &UsageDescriptions, level: Level) -> Result<&str, UsageError> {
    texts.for_level(level)
}

#[cfg(test)]
mod privacy_rule_tests {
    use super::*;

    #[test]
    fn approximate_moves_a_point_at_most_half_a_cell_and_about_3_km() {
        let (lat, lon) = (65.68, -18.10);
        let (a, b) = approximate(lat, lon);
        assert!((a - lat).abs() <= APPROXIMATE_GRID_DEG / 2.0 + 1e-9);
        assert!((b - lon).abs() <= APPROXIMATE_GRID_DEG / 2.0 + 1e-9);
        let km = ((a - lat).powi(2) + (b - lon).powi(2)).sqrt() * 111.0;
        assert!(km <= 3.0, "{km}");
    }

    #[test]
    fn approximate_is_idempotent_and_stays_in_range() {
        let once = approximate(64.14, -21.94);
        assert_eq!(approximate(once.0, once.1), once);
        assert!(approximate(90.0, 180.0).0 <= 90.0);
        let (lat, lon) = approximate(89.999, -179.999);
        assert!((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon));
    }

    #[test]
    fn approximate_access_is_the_default_and_precise_is_the_switch() {
        assert_eq!(accuracy_for(false), AccuracyAuthorization::ReducedAccuracy);
        assert_eq!(accuracy_for(true), AccuracyAuthorization::FullAccuracy);
        assert!(APPROXIMATE_ACCURACY >= APPROXIMATE_GRID_DEG * 111_000.0 / 2.0);
    }

    #[test]
    fn a_request_without_a_purpose_string_is_refused() {
        let blank = UsageDescriptions::new("   ", "");
        assert_eq!(request_purpose(&blank, Level::WhenInUse), Err(UsageError::Missing(Level::WhenInUse)));
        let text = UsageDescriptions::new(" Finna næstu stöð ", "");
        assert_eq!(request_purpose(&text, Level::WhenInUse), Ok("Finna næstu stöð"));
    }
}

/// The error cases of the location service, in the order the official reference lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocationError {
    /// The location manager could not obtain a location right now.
    LocationUnknown,
    /// The user denied access to the location service.
    Denied,
    /// The user did not grant the requested temporary authorization.
    PromptDeclined,
    /// The network was unavailable or a network error occurred.
    Network,
    /// The heading cannot be determined.
    HeadingFailure,
    /// Ranging is disabled.
    RangingUnavailable,
    /// A general ranging error occurred.
    RangingFailure,
    /// The user denied access to region monitoring.
    RegionMonitoringDenied,
    /// A registered region could not be monitored.
    RegionMonitoringFailure,
    /// Region monitoring could not start immediately.
    RegionMonitoringSetupDelayed,
    /// Region events will arrive, but they may be late.
    RegionMonitoringResponseDelayed,
    /// The geocode request was canceled.
    GeocodeCanceled,
    /// The geocode request gave no result.
    GeocodeFoundNoResult,
    /// The geocode request gave a partial result.
    GeocodeFoundPartialResult,
    /// Deferred updates did not start, for an unknown reason.
    DeferredFailed,
    /// Deferred updates were canceled by the app or the location manager.
    DeferredCanceled,
    /// Deferred mode does not support the requested accuracy.
    DeferredAccuracyTooLow,
    /// Deferred mode does not support distance filters.
    DeferredDistanceFiltered,
    /// Deferred mode did not start because location updates were already off or paused.
    DeferredNotUpdatingLocation,
}

/// Every error case, in declaration order. The index of a case is its [`LocationError::code`].
pub const ERRORS: [LocationError; 19] = [
    LocationError::LocationUnknown,
    LocationError::Denied,
    LocationError::PromptDeclined,
    LocationError::Network,
    LocationError::HeadingFailure,
    LocationError::RangingUnavailable,
    LocationError::RangingFailure,
    LocationError::RegionMonitoringDenied,
    LocationError::RegionMonitoringFailure,
    LocationError::RegionMonitoringSetupDelayed,
    LocationError::RegionMonitoringResponseDelayed,
    LocationError::GeocodeCanceled,
    LocationError::GeocodeFoundNoResult,
    LocationError::GeocodeFoundPartialResult,
    LocationError::DeferredFailed,
    LocationError::DeferredCanceled,
    LocationError::DeferredAccuracyTooLow,
    LocationError::DeferredDistanceFiltered,
    LocationError::DeferredNotUpdatingLocation,
];

impl LocationError {
    /// The error domain BifrOSt reports these errors under.
    pub const DOMAIN: &'static str = "bifrost.location";

    /// The numeric code of this case: its position in [`ERRORS`].
    pub fn code(self) -> u32 {
        self as u32
    }

    /// The case with this code, or `None` when the code is out of range.
    pub fn from_code(code: u32) -> Option<LocationError> {
        ERRORS.get(code as usize).copied()
    }

    /// Whether the user must change something first: answer the prompt, or change the setting in Settings.
    pub fn needs_user_action(self) -> bool {
        matches!(self, LocationError::Denied | LocationError::PromptDeclined | LocationError::RegionMonitoringDenied)
    }

    /// Whether the same request may succeed later with no change, because the cause is a passing one.
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            LocationError::LocationUnknown
                | LocationError::Network
                | LocationError::RegionMonitoringSetupDelayed
                | LocationError::RegionMonitoringResponseDelayed
                | LocationError::DeferredFailed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_authorized_statuses_are_authorized() {
        assert!(AuthorizationStatus::AuthorizedAlways.is_authorized());
        assert!(AuthorizationStatus::AuthorizedWhenInUse.is_authorized());
        assert!(!AuthorizationStatus::NotDetermined.is_authorized());
        assert!(!AuthorizationStatus::Restricted.is_authorized());
        assert!(!AuthorizationStatus::Denied.is_authorized());
    }

    #[test]
    fn when_in_use_starts_only_in_the_foreground_and_always_starts_anywhere() {
        assert!(AuthorizationStatus::AuthorizedAlways.may_start(false));
        assert!(AuthorizationStatus::AuthorizedWhenInUse.may_start(true));
        assert!(!AuthorizationStatus::AuthorizedWhenInUse.may_start(false));
        assert!(!AuthorizationStatus::Denied.may_start(true));
        assert!(!AuthorizationStatus::NotDetermined.may_start(true));
    }

    #[test]
    fn a_request_prompts_only_when_the_user_has_not_answered() {
        assert_eq!(AuthorizationStatus::NotDetermined.request_outcome(), RequestOutcome::Prompt);
        assert_eq!(AuthorizationStatus::AuthorizedWhenInUse.request_outcome(), RequestOutcome::AlreadyGranted);
        assert_eq!(AuthorizationStatus::Restricted.request_outcome(), RequestOutcome::Refused);
        assert_eq!(AuthorizationStatus::Denied.request_outcome(), RequestOutcome::Refused);
    }

    #[test]
    fn only_full_accuracy_is_precise() {
        assert!(AccuracyAuthorization::FullAccuracy.is_full());
        assert!(!AccuracyAuthorization::ReducedAccuracy.is_full());
    }

    #[test]
    fn a_usage_text_is_trimmed_and_a_blank_one_is_missing() {
        let texts = UsageDescriptions::new("  Finna næstu stöð  ", "   ");
        assert_eq!(texts.for_level(Level::WhenInUse), Ok("Finna næstu stöð"));
        assert_eq!(texts.for_level(Level::Always), Err(UsageError::Missing(Level::Always)));
        assert_eq!(UsageDescriptions::default().for_level(Level::WhenInUse), Err(UsageError::Missing(Level::WhenInUse)));
    }

    #[test]
    fn error_codes_are_the_declaration_order_and_round_trip() {
        assert_eq!(ERRORS.len(), 19);
        assert_eq!(LocationError::LocationUnknown.code(), 0);
        assert_eq!(LocationError::DeferredNotUpdatingLocation.code(), 18);
        for (i, error) in ERRORS.iter().enumerate() {
            assert_eq!(error.code(), i as u32);
            assert_eq!(LocationError::from_code(i as u32), Some(*error));
        }
        assert_eq!(LocationError::from_code(19), None);
        assert_eq!(LocationError::DOMAIN, "bifrost.location");
    }

    #[test]
    fn denied_needs_the_user_and_a_network_error_retries() {
        assert!(LocationError::Denied.needs_user_action());
        assert!(LocationError::PromptDeclined.needs_user_action());
        assert!(!LocationError::Denied.is_transient());
        assert!(LocationError::Network.is_transient());
        assert!(!LocationError::Network.needs_user_action());
        // a canceled geocode is neither: the app chose to cancel
        assert!(!LocationError::GeocodeCanceled.needs_user_action());
        assert!(!LocationError::GeocodeCanceled.is_transient());
    }
}
