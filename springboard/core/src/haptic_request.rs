//! App-facing haptic requests: the policy that turns an app's haptic call into a pulse on a force-feedback device.
//!
//! The SDK's parameterless haptic call asks for a [`Pattern::Generic`] pulse. Before it reaches the device, a request
//! passes two checks: an app may play at most one pulse per [`MIN_INTERVAL_MS`], so a loop that calls the haptic call
//! every frame cannot buzz a wheel continuously, and the device must have a free effect slot. The module keeps the
//! last accepted time for each app and drives nothing itself; the caller passes in the device and the time.

use crate::force_effect::{pulse_effect, Device, Error, Pattern};
use std::collections::HashMap;

/// The shortest gap between two pulses from one app, in milliseconds.
pub const MIN_INTERVAL_MS: u64 = 30;

/// What became of one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// the pulse was played; the value is its effect id on the device
    Played(u16),
    /// the app asked again too soon after its last pulse, so nothing was played
    Throttled,
    /// the device refused the pulse
    Refused(Error),
}

/// The per-app request state: when each app last had a pulse accepted.
#[derive(Debug, Default)]
pub struct Requests {
    last_played_ms: HashMap<String, u64>,
}

impl Requests {
    pub fn new() -> Self {
        Requests::default()
    }

    /// Asks for one pulse for `app` at `now_ms`. A request inside [`MIN_INTERVAL_MS`] of the app's last accepted pulse
    /// is throttled. A time that goes backwards counts as no time passed, so it is also throttled.
    pub fn request(&mut self, device: &mut Device, app: &str, pattern: Pattern, now_ms: u64) -> Outcome {
        if let Some(&last) = self.last_played_ms.get(app) {
            if now_ms.saturating_sub(last) < MIN_INTERVAL_MS {
                return Outcome::Throttled;
            }
        }
        match device.pulse(pattern, now_ms) {
            Ok(id) => {
                self.last_played_ms.insert(app.to_owned(), now_ms);
                Outcome::Played(id)
            }
            Err(error) => Outcome::Refused(error),
        }
    }

    /// Forgets an app's history, for when it quits.
    pub fn forget(&mut self, app: &str) {
        self.last_played_ms.remove(app);
    }
}

/// The effect a request would play, for callers that show or log the pattern before playing it.
pub fn effect_for(pattern: Pattern) -> crate::force_effect::Effect {
    pulse_effect(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::force_effect::MAX_EFFECTS;

    #[test]
    fn the_first_request_from_an_app_plays() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        assert_eq!(requests.request(&mut device, "a", Pattern::Generic, 0), Outcome::Played(1));
    }

    #[test]
    fn a_second_request_inside_the_interval_is_throttled_and_plays_nothing() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        requests.request(&mut device, "a", Pattern::Generic, 100);
        let before = device.len();
        let outcome = requests.request(&mut device, "a", Pattern::Alignment, 100 + MIN_INTERVAL_MS - 1);
        assert_eq!(outcome, Outcome::Throttled);
        assert_eq!(device.len(), before);
    }

    #[test]
    fn a_request_at_exactly_the_interval_plays_again() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        requests.request(&mut device, "a", Pattern::Generic, 0);
        assert!(matches!(
            requests.request(&mut device, "a", Pattern::Generic, MIN_INTERVAL_MS),
            Outcome::Played(_)
        ));
    }

    #[test]
    fn apps_are_throttled_separately() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        requests.request(&mut device, "a", Pattern::Generic, 0);
        assert!(matches!(requests.request(&mut device, "b", Pattern::Generic, 1), Outcome::Played(_)));
    }

    #[test]
    fn a_time_that_goes_backwards_is_throttled() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        requests.request(&mut device, "a", Pattern::Generic, 500);
        assert_eq!(requests.request(&mut device, "a", Pattern::Generic, 10), Outcome::Throttled);
    }

    #[test]
    fn a_full_device_refuses_the_pulse_and_the_app_is_not_marked() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        // long-lived effects are not freed by reaping, so they fill every slot
        for _ in 0..MAX_EFFECTS {
            device.create(pulse_effect(Pattern::Generic)).unwrap();
        }
        let outcome = requests.request(&mut device, "a", Pattern::Generic, 0);
        assert_eq!(outcome, Outcome::Refused(Error::Full));
        // a refused pulse does not start the throttle, so the app is not held back afterwards
        assert!(!requests.last_played_ms.contains_key("a"));
    }

    #[test]
    fn forgetting_an_app_lets_it_play_again_at_once() {
        let mut device = Device::new();
        let mut requests = Requests::new();
        requests.request(&mut device, "a", Pattern::Generic, 0);
        requests.forget("a");
        assert!(matches!(requests.request(&mut device, "a", Pattern::Generic, 1), Outcome::Played(_)));
    }

    #[test]
    fn effect_for_matches_the_pulse_the_device_plays() {
        assert_eq!(effect_for(Pattern::LevelChange), pulse_effect(Pattern::LevelChange));
    }
}
