//! Sound output routing between the built-in speaker and a wired headset jack, each route with its own remembered volume.
//!
//! The audio service applies the route to the mixer. This model answers the questions an app asks about it: which
//! route is active, the volume that route plays at, what a jack change does, and when a headphone volume is capped.
//! Volumes are levels from 0 to 1. Headphones are capped at [`HEADPHONE_CAP`] until the user goes above the cap once,
//! which lifts the cap for good.

/// The loudest headphone level before the cap applies, until the user lifts it.
pub const HEADPHONE_CAP: f32 = 0.85;
/// Settings key for the speaker volume.
pub const PREF_SPEAKER: &str = "volume_speaker";
/// Settings key for the headphone volume.
pub const PREF_HEADPHONES: &str = "volume_headphones";

/// Which output the sound goes to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Route {
    #[default]
    Speaker,
    Headphones,
}

impl Route {
    /// The name a status display shows for the route.
    pub fn label(self) -> &'static str {
        match self {
            Route::Speaker => "Speaker",
            Route::Headphones => "Headphones",
        }
    }

    /// The settings key that holds this route's remembered volume.
    pub fn pref_key(self) -> &'static str {
        match self {
            Route::Speaker => PREF_SPEAKER,
            Route::Headphones => PREF_HEADPHONES,
        }
    }
}

/// A route change caused by a jack event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteChange {
    pub route: Route,
    /// True when playback should pause: sound left the headphones.
    pub pause: bool,
}

/// The active route with each route's volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Router {
    route: Route,
    speaker: f32,
    headphones: f32,
    cap_lifted: bool,
}

fn clamp_level(level: f32) -> f32 {
    if level.is_nan() {
        0.0
    } else {
        level.clamp(0.0, 1.0)
    }
}

impl Router {
    /// A router from the remembered levels, whether the cap was lifted, and whether the jack is plugged in at start.
    /// A plugged jack selects headphones at once and applies the cap to their remembered volume.
    pub fn new(speaker: f32, headphones: f32, cap_lifted: bool, plugged: bool) -> Self {
        let mut router = Router { route: Route::Speaker, speaker: clamp_level(speaker), headphones: clamp_level(headphones), cap_lifted };
        if plugged {
            router.route = Route::Headphones;
            router.headphones = router.capped(router.headphones);
        }
        router
    }

    fn capped(&self, level: f32) -> f32 {
        if self.cap_lifted {
            level
        } else {
            level.min(HEADPHONE_CAP)
        }
    }

    /// The active route.
    pub fn route(&self) -> Route {
        self.route
    }

    /// The volume of a route.
    pub fn volume_of(&self, route: Route) -> f32 {
        match route {
            Route::Speaker => self.speaker,
            Route::Headphones => self.headphones,
        }
    }

    /// The volume of the active route.
    pub fn volume(&self) -> f32 {
        self.volume_of(self.route)
    }

    /// Whether the user has lifted the headphone cap.
    pub fn cap_lifted(&self) -> bool {
        self.cap_lifted
    }

    /// The jack changed. Returns the change when the route moves, and None when it stays. A plug-in lowers a too-loud
    /// remembered headphone volume to the cap; an unplug moves sound back to the speaker and asks for a pause.
    pub fn set_jack(&mut self, plugged: bool) -> Option<RouteChange> {
        let to = if plugged { Route::Headphones } else { Route::Speaker };
        if to == self.route {
            return None;
        }
        self.route = to;
        if plugged {
            self.headphones = self.capped(self.headphones);
        }
        Some(RouteChange { route: to, pause: !plugged })
    }

    /// Sets the active route's volume, clamped to 0 to 1. Going above the cap on headphones lifts it for good.
    pub fn set_volume(&mut self, level: f32) {
        let v = clamp_level(level);
        match self.route {
            Route::Speaker => self.speaker = v,
            Route::Headphones => {
                self.headphones = v;
                if v > HEADPHONE_CAP {
                    self.cap_lifted = true;
                }
            }
        }
    }
}
