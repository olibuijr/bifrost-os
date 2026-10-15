//! Nearby browsing for apps: whether one service type is being browsed, and what happens to that browse when the app
//! leaves the foreground.
//!
//! [`NearbyBrowser::start`] and [`NearbyBrowser::stop`] record what the app wants. The app reports foreground changes
//! with [`NearbyBrowser::did_enter_background`] and [`NearbyBrowser::will_enter_foreground`]. While the app is in the
//! background, a wanted browse is [`Phase::Suspended`]: it does not run, and it resumes when the app comes back to the
//! foreground. A browse that the app stopped never resumes by itself.
//!
//! This is the pure model: no radio, no beacon and no timer. The shell's nearby beacon does not yet take an app's
//! service type or its foreground state, so nothing is browsed on the air because of this model yet.

use crate::nearby_service::ServiceType;

/// What the browser is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The app does not want to browse, or stopped browsing.
    Stopped,
    /// The app wants to browse and is in the foreground.
    Browsing,
    /// The app wants to browse, but it is in the background and waits for the foreground.
    Suspended,
}

/// The browse of one service type, with the app's wish and its foreground state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NearbyBrowser {
    service: ServiceType,
    wanted: bool,
    in_background: bool,
}

impl NearbyBrowser {
    /// A browser for `service` that is stopped and in the foreground.
    pub fn new(service: ServiceType) -> Self {
        NearbyBrowser { service, wanted: false, in_background: false }
    }

    pub fn service(&self) -> &ServiceType {
        &self.service
    }

    pub fn phase(&self) -> Phase {
        match (self.wanted, self.in_background) {
            (false, _) => Phase::Stopped,
            (true, false) => Phase::Browsing,
            (true, true) => Phase::Suspended,
        }
    }

    pub fn is_browsing(&self) -> bool {
        self.phase() == Phase::Browsing
    }

    /// Asks to browse. Starting while the app is in the background waits for the foreground.
    pub fn start(&mut self) {
        self.wanted = true;
    }

    /// Stops browsing for good: a later foreground change does not start it again.
    pub fn stop(&mut self) {
        self.wanted = false;
    }

    /// The app left the foreground: a wanted browse is suspended.
    pub fn did_enter_background(&mut self) {
        self.in_background = true;
    }

    /// The app is back in the foreground: a wanted browse resumes.
    pub fn will_enter_foreground(&mut self) {
        self.in_background = false;
    }
}
