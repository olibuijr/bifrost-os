//! Local authentication model (task theme-02): which biometric method the device has, the policies an app may ask for,
//! the requirements that gate a stored secret, and the device's domain state that a stored secret is bound to. Pure
//! model: no sensor, no prompt and no secure storage. The shell feeds it the device facts it knows (the login and
//! passcode store are the existing sources), and an app asks `resolve` which method would succeed.

use crate::sha256;

/// The biometric method the device supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiometryType {
    None,
    Touch,
    Face,
    Optic,
}

/// The facts about the device that an authentication decision depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub biometry: BiometryType,
    /// At least one biometric identity is enrolled. Identities are named by `enrolled_sets`.
    pub enrolled_sets: Vec<String>,
    pub passcode_set: bool,
    /// Failed biometric attempts since the last success.
    pub failures: u32,
    /// Failures after which biometry is locked and only the fallback can succeed.
    pub max_failures: u32,
}

/// Which method satisfied a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Biometry,
    Passcode,
}

/// Why a request cannot be satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The device has no biometric sensor.
    BiometryUnavailable,
    /// The sensor exists but no identity is enrolled.
    BiometryNotEnrolled,
    /// Too many failed attempts; biometry is locked.
    BiometryLockout,
    /// No passcode is set, so the passcode fallback cannot run.
    PasscodeNotSet,
    /// The enrolled identities changed since the secret was stored.
    StateChanged,
}

/// The app-facing policy, mirroring the two device-owner policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Biometry only; no passcode fallback.
    BiometricsOnly,
    /// Biometry, with the device passcode as the fallback.
    DeviceOwner,
}

/// What the fallback does when biometry is not usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// The system default, which is the device passcode.
    Default,
    DevicePasscode,
}

/// The requirement that protects a stored secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// User authentication: biometry if usable, otherwise the passcode.
    Default,
    /// Biometry only, optionally with a fallback. `current_set` binds the secret to the enrolled identities.
    Biometry { current_set: bool, fallback: Option<Fallback> },
}

/// A digest of the device's biometric state. It changes when the sensor type or the enrolled identities change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainState {
    pub hash: String,
}

impl DomainState {
    /// Digest of the sensor type and the sorted enrolled identity names.
    pub fn of(device: &Device) -> DomainState {
        let mut names = device.enrolled_sets.clone();
        names.sort();
        let kind = match device.biometry {
            BiometryType::None => "none",
            BiometryType::Touch => "touch",
            BiometryType::Face => "face",
            BiometryType::Optic => "optic",
        };
        let text = format!("{kind}\n{}", names.join("\n"));
        DomainState { hash: sha256::hex(&sha256::digest(text.as_bytes())) }
    }
}

impl Device {
    /// Records a failed biometric attempt.
    pub fn record_failure(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    /// Records a successful biometric attempt, clearing the failure count.
    pub fn record_success(&mut self) {
        self.failures = 0;
    }

    fn locked_out(&self) -> bool {
        self.failures >= self.max_failures
    }

    /// True when biometry can answer right now: a sensor exists, an identity is enrolled and it is not locked.
    fn biometry_usable(&self) -> bool {
        self.biometry != BiometryType::None && !self.enrolled_sets.is_empty() && !self.locked_out()
    }

    /// The failure to report when biometry is not usable and the fallback does not apply.
    fn biometry_failure(&self) -> Failure {
        if self.biometry == BiometryType::None {
            Failure::BiometryUnavailable
        } else if self.enrolled_sets.is_empty() {
            Failure::BiometryNotEnrolled
        } else {
            Failure::BiometryLockout
        }
    }
}

impl Policy {
    /// The requirement that a policy evaluates to.
    pub fn requirement(self) -> Requirement {
        match self {
            Policy::BiometricsOnly => Requirement::Biometry { current_set: false, fallback: None },
            Policy::DeviceOwner => Requirement::Default,
        }
    }
}

/// Decides which method satisfies `req` on `device`. `recorded` is the state digest saved with the secret; a
/// `current_set` requirement needs it to match the device's current digest.
pub fn resolve(req: Requirement, device: &Device, recorded: Option<&DomainState>) -> Result<Method, Failure> {
    match req {
        Requirement::Default => {
            if device.biometry_usable() {
                Ok(Method::Biometry)
            } else if device.passcode_set {
                Ok(Method::Passcode)
            } else {
                Err(Failure::PasscodeNotSet)
            }
        }
        Requirement::Biometry { current_set, fallback } => {
            if current_set {
                let current = DomainState::of(device);
                if recorded != Some(&current) {
                    return Err(Failure::StateChanged);
                }
            }
            if device.biometry_usable() {
                return Ok(Method::Biometry);
            }
            match fallback {
                Some(_) if device.passcode_set => Ok(Method::Passcode),
                Some(_) => Err(Failure::PasscodeNotSet),
                None => Err(device.biometry_failure()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(biometry: BiometryType) -> Device {
        Device {
            biometry,
            enrolled_sets: vec!["owner".into()],
            passcode_set: true,
            failures: 0,
            max_failures: 3,
        }
    }

    #[test]
    fn default_policy_prefers_biometry_then_passcode() {
        let d = phone(BiometryType::Face);
        assert_eq!(resolve(Policy::DeviceOwner.requirement(), &d, None), Ok(Method::Biometry));
        let mut locked = d.clone();
        locked.failures = 3;
        assert_eq!(resolve(Policy::DeviceOwner.requirement(), &locked, None), Ok(Method::Passcode));
    }

    #[test]
    fn biometrics_only_policy_fails_when_biometry_is_locked_out() {
        let mut d = phone(BiometryType::Touch);
        d.failures = 3;
        assert_eq!(resolve(Policy::BiometricsOnly.requirement(), &d, None), Err(Failure::BiometryLockout));
        d.record_success();
        assert_eq!(resolve(Policy::BiometricsOnly.requirement(), &d, None), Ok(Method::Biometry));
    }

    #[test]
    fn missing_sensor_and_missing_enrolment_are_reported_separately() {
        let none = phone(BiometryType::None);
        assert_eq!(resolve(Policy::BiometricsOnly.requirement(), &none, None), Err(Failure::BiometryUnavailable));
        let mut bare = phone(BiometryType::Optic);
        bare.enrolled_sets.clear();
        assert_eq!(resolve(Policy::BiometricsOnly.requirement(), &bare, None), Err(Failure::BiometryNotEnrolled));
        assert_eq!(resolve(Policy::DeviceOwner.requirement(), &bare, None), Ok(Method::Passcode));
    }

    #[test]
    fn fallback_to_passcode_needs_a_passcode() {
        let mut d = phone(BiometryType::Face);
        d.failures = 3;
        let req = Requirement::Biometry { current_set: false, fallback: Some(Fallback::DevicePasscode) };
        assert_eq!(resolve(req, &d, None), Ok(Method::Passcode));
        d.passcode_set = false;
        assert_eq!(resolve(req, &d, None), Err(Failure::PasscodeNotSet));
    }

    #[test]
    fn current_set_requirement_fails_after_enrolment_changes() {
        let mut d = phone(BiometryType::Face);
        let stored = DomainState::of(&d);
        let req = Requirement::Biometry { current_set: true, fallback: None };
        assert_eq!(resolve(req, &d, Some(&stored)), Ok(Method::Biometry));
        d.enrolled_sets.push("guest".into());
        assert_eq!(resolve(req, &d, Some(&stored)), Err(Failure::StateChanged));
        assert_eq!(resolve(req, &d, None), Err(Failure::StateChanged));
    }

    #[test]
    fn domain_state_ignores_enrolment_order_but_tracks_sensor_type() {
        let mut a = phone(BiometryType::Face);
        a.enrolled_sets = vec!["b".into(), "a".into()];
        let mut b = a.clone();
        b.enrolled_sets = vec!["a".into(), "b".into()];
        assert_eq!(DomainState::of(&a), DomainState::of(&b));
        let other = phone(BiometryType::Touch);
        assert_ne!(DomainState::of(&a), DomainState::of(&other));
    }

    #[test]
    fn failure_counter_saturates_and_success_clears_it() {
        let mut d = phone(BiometryType::Touch);
        for _ in 0..5 {
            d.record_failure();
        }
        assert!(d.locked_out());
        d.record_success();
        assert_eq!(d.failures, 0);
        assert!(!d.locked_out());
    }
}
