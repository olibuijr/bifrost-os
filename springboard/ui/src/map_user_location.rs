//! User location on a map: joins the location authorization and the latest position fix to the map camera's tracking.
//!
//! The model reads no sensor and starts no service. A caller reports the authorization status the user answered and
//! the fixes its position source delivers, then asks the model to move a [`MapCamera`]. Tracking only has an effect
//! while the app may use location services, and it follows a fix only while that fix is valid, recent and accurate
//! enough. The age and accuracy limits are this module's own rules.

use crate::location::LocationFix;
use crate::location_auth::AuthorizationStatus;
use crate::mapview::{MapCamera, UserTracking};

/// The oldest fix, in seconds, that tracking still follows.
pub const MAX_FIX_AGE_S: f64 = 120.0;

/// The least accurate horizontal fix, in metres, that tracking still follows.
pub const MAX_FIX_ACCURACY_M: f64 = 1000.0;

/// The user's location as a map sees it: access, the last fix, and the tracking state the map button sets.
#[derive(Clone, Debug, PartialEq)]
pub struct UserLocation {
    authorization: AuthorizationStatus,
    fix: Option<LocationFix>,
    tracking: UserTracking,
}

impl UserLocation {
    /// A map with no fix and tracking off.
    pub fn new(authorization: AuthorizationStatus) -> Self {
        UserLocation { authorization, fix: None, tracking: UserTracking::Off }
    }

    pub fn authorization(&self) -> AuthorizationStatus {
        self.authorization
    }

    /// Records the status the user answered. Any status that does not authorize location services drops the last fix
    /// and turns tracking off.
    pub fn set_authorization(&mut self, authorization: AuthorizationStatus) {
        self.authorization = authorization;
        if !authorization.is_authorized() {
            self.fix = None;
            self.tracking = UserTracking::Off;
        }
    }

    /// Keeps `fix` as the latest position. Returns false, and keeps the previous fix, when the app may not use location
    /// services or the fix has no valid horizontal accuracy or timestamp.
    pub fn record_fix(&mut self, fix: LocationFix) -> bool {
        if !self.authorization.is_authorized() || !fix.is_valid() {
            return false;
        }
        self.fix = Some(fix);
        true
    }

    /// The latest recorded fix, whatever its age.
    pub fn fix(&self) -> Option<&LocationFix> {
        self.fix.as_ref()
    }

    /// The tracking state the map shows. Without access it is always off.
    pub fn tracking(&self) -> UserTracking {
        if self.authorization.is_authorized() {
            self.tracking
        } else {
            UserTracking::Off
        }
    }

    /// Presses the tracking button: off, follow, follow with heading, then off again. Without access nothing changes
    /// and the state stays off. Returns the state now shown.
    pub fn press_tracking(&mut self) -> UserTracking {
        if self.authorization.is_authorized() {
            self.tracking = self.tracking.next();
        }
        self.tracking()
    }

    /// The fix tracking would follow at `now`: the latest one when it is zero to [`MAX_FIX_AGE_S`] seconds old and its
    /// horizontal accuracy is at most [`MAX_FIX_ACCURACY_M`] metres.
    pub fn followable_fix(&self, now: f64) -> Option<&LocationFix> {
        self.fix
            .as_ref()
            .filter(|fix| (0.0..=MAX_FIX_AGE_S).contains(&fix.age_at(now)) && fix.horizontal_accuracy_m <= MAX_FIX_ACCURACY_M)
    }

    /// Moves `camera` to the user for the current tracking state, turning it to `heading_deg` in the heading state when a
    /// heading is known. Returns true when the camera moved. Tracking off, or no followable fix, leaves the camera alone.
    pub fn follow(&self, camera: &mut MapCamera, now: f64, heading_deg: Option<f64>) -> bool {
        let mode = self.tracking();
        if !mode.centres_on_user() {
            return false;
        }
        let Some(fix) = self.followable_fix(now) else {
            return false;
        };
        camera.follow(fix.coordinate, heading_deg, mode);
        true
    }
}
