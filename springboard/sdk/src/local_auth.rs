//! Local Authentication: `LAContext`, `LAPolicy`, `LAError` and `LABiometryType`.
//!
//! An app builds a [`LAContext`], asks [`LAContext::can_evaluate_policy`] whether a policy could succeed on this
//! device, and calls [`LAContext::evaluate_policy`] with a reason the person reads in the prompt. The prompt itself is
//! owned by the shell; this type models the context (fallback and cancel titles, reuse window, invalidation) and
//! decodes the shell's reply. The `local_auth` shell service verifies the account password; there is no biometry,
//! so `deviceOwnerAuthenticationWithBiometrics` always fails with `biometryNotAvailable`.
//!
//! ```ignore
//! let mut ctx = LAContext::new(Device { password_set: true });
//! ctx.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication)?;
//! ctx.evaluate_policy(LAPolicy::DeviceOwnerAuthentication, "Reveal password", now, |reason| shell_ask(reason))?;
//! ```

use bifrost_core::json::Json;

/// `LAPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LAPolicy {
    DeviceOwnerAuthenticationWithBiometrics,
    DeviceOwnerAuthentication,
}

impl LAPolicy {
    /// The wire name the shell service uses.
    pub fn wire(self) -> &'static str {
        match self {
            LAPolicy::DeviceOwnerAuthenticationWithBiometrics => "device_owner_authentication_with_biometrics",
            LAPolicy::DeviceOwnerAuthentication => "device_owner_authentication",
        }
    }
}

/// `LABiometryType`. BifrOSt has no sensor, so a context always reports `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiometryType {
    None,
    TouchID,
    FaceID,
    OpticID,
}

/// The error domain name of [`LAError`] values on the wire and in logs.
pub const ERROR_DOMAIN: &str = "bifrost.local-authentication";

/// Every [`LAError`], in code order.
const ALL_ERRORS: [LAError; 11] = [
    LAError::AuthenticationFailed,
    LAError::UserCancel,
    LAError::UserFallback,
    LAError::SystemCancel,
    LAError::PasscodeNotSet,
    LAError::BiometryNotAvailable,
    LAError::BiometryNotEnrolled,
    LAError::BiometryLockout,
    LAError::AppCancel,
    LAError::InvalidContext,
    LAError::NotInteractive,
];

/// `LAError.Code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LAError {
    AuthenticationFailed,
    UserCancel,
    UserFallback,
    SystemCancel,
    PasscodeNotSet,
    BiometryNotAvailable,
    BiometryNotEnrolled,
    BiometryLockout,
    AppCancel,
    InvalidContext,
    NotInteractive,
}

impl LAError {
    /// The wire name the shell service uses.
    pub fn wire(self) -> &'static str {
        match self {
            LAError::AuthenticationFailed => "authentication_failed",
            LAError::UserCancel => "user_cancel",
            LAError::UserFallback => "user_fallback",
            LAError::SystemCancel => "system_cancel",
            LAError::PasscodeNotSet => "passcode_not_set",
            LAError::BiometryNotAvailable => "biometry_not_available",
            LAError::BiometryNotEnrolled => "biometry_not_enrolled",
            LAError::BiometryLockout => "biometry_lockout",
            LAError::AppCancel => "app_cancel",
            LAError::InvalidContext => "invalid_context",
            LAError::NotInteractive => "not_interactive",
        }
    }

    /// The error for a wire name; `None` for an unknown name.
    pub fn from_wire(name: &str) -> Option<LAError> {
        ALL_ERRORS.into_iter().find(|e| e.wire() == name)
    }

    /// The numeric code of the error in [`ERROR_DOMAIN`], as the reference platform numbers `LAError.Code`.
    pub fn code(self) -> i64 {
        match self {
            LAError::AuthenticationFailed => -1,
            LAError::UserCancel => -2,
            LAError::UserFallback => -3,
            LAError::SystemCancel => -4,
            LAError::PasscodeNotSet => -5,
            LAError::BiometryNotAvailable => -6,
            LAError::BiometryNotEnrolled => -7,
            LAError::BiometryLockout => -8,
            LAError::AppCancel => -9,
            LAError::InvalidContext => -10,
            LAError::NotInteractive => -1004,
        }
    }

    /// The error for a numeric code; `None` for a code this domain does not define.
    pub fn from_code(code: i64) -> Option<LAError> {
        ALL_ERRORS.into_iter().find(|e| e.code() == code)
    }
}

/// Facts about the device an evaluation depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    pub password_set: bool,
}

/// Decodes the shell's reply to an evaluate call: `{ok: true}` or `{ok: false, error}`.
pub fn decode_reply(reply: &Json) -> Result<(), LAError> {
    if matches!(reply.get("ok"), Some(Json::Bool(true))) {
        return Ok(());
    }
    Err(reply.get("error").and_then(Json::as_str).and_then(LAError::from_wire).unwrap_or(LAError::AuthenticationFailed))
}

/// `LAContext`. Times are seconds since any fixed epoch the caller uses.
#[derive(Clone, Debug, PartialEq)]
pub struct LAContext {
    device: Device,
    pub localized_fallback_title: Option<String>,
    pub localized_cancel_title: Option<String>,
    /// `touchIDAuthenticationAllowableReuseDuration`: seconds a success stays valid. 0 always prompts.
    pub touch_id_authentication_allowable_reuse_duration: u64,
    last_success: Option<u64>,
    invalidated: bool,
}

impl LAContext {
    pub fn new(device: Device) -> LAContext {
        LAContext {
            device,
            localized_fallback_title: None,
            localized_cancel_title: None,
            touch_id_authentication_allowable_reuse_duration: 0,
            last_success: None,
            invalidated: false,
        }
    }

    /// `biometryType`.
    pub fn biometry_type(&self) -> BiometryType {
        BiometryType::None
    }

    /// `canEvaluatePolicy(_:error:)`.
    pub fn can_evaluate_policy(&self, policy: LAPolicy) -> Result<(), LAError> {
        if self.invalidated {
            return Err(LAError::InvalidContext);
        }
        match policy {
            LAPolicy::DeviceOwnerAuthenticationWithBiometrics => Err(LAError::BiometryNotAvailable),
            LAPolicy::DeviceOwnerAuthentication if self.device.password_set => Ok(()),
            LAPolicy::DeviceOwnerAuthentication => Err(LAError::PasscodeNotSet),
        }
    }

    /// `evaluatePolicy(_:localizedReason:reply:)`. `ask` shows the prompt with the reason and returns the shell's
    /// reply; it is not called when a success inside the reuse window is still valid.
    pub fn evaluate_policy(
        &mut self,
        policy: LAPolicy,
        localized_reason: &str,
        now: u64,
        ask: impl FnOnce(&str) -> Json,
    ) -> Result<(), LAError> {
        self.can_evaluate_policy(policy)?;
        if let Some(t) = self.last_success {
            if now.saturating_sub(t) < self.touch_id_authentication_allowable_reuse_duration {
                return Ok(());
            }
        }
        decode_reply(&ask(localized_reason)).map(|()| self.last_success = Some(now))
    }

    /// `invalidate()`: every later call fails with `invalidContext` and the reuse window is dropped.
    pub fn invalidate(&mut self) {
        self.invalidated = true;
        self.last_success = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LAContext {
        LAContext::new(Device { password_set: true })
    }
    fn ok() -> Json {
        Json::Obj(vec![("ok".into(), Json::Bool(true))])
    }
    fn err(name: &str) -> Json {
        Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str(name.into()))])
    }

    #[test]
    fn biometrics_policy_is_not_available() {
        assert_eq!(ctx().can_evaluate_policy(LAPolicy::DeviceOwnerAuthenticationWithBiometrics), Err(LAError::BiometryNotAvailable));
        assert_eq!(ctx().biometry_type(), BiometryType::None);
    }

    #[test]
    fn device_owner_needs_a_password() {
        assert_eq!(ctx().can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Ok(()));
        let c = LAContext::new(Device { password_set: false });
        assert_eq!(c.can_evaluate_policy(LAPolicy::DeviceOwnerAuthentication), Err(LAError::PasscodeNotSet));
    }

    #[test]
    fn reply_decodes_success_cancel_and_failure() {
        assert_eq!(decode_reply(&ok()), Ok(()));
        assert_eq!(decode_reply(&err("user_cancel")), Err(LAError::UserCancel));
        assert_eq!(decode_reply(&err("authentication_failed")), Err(LAError::AuthenticationFailed));
        assert_eq!(decode_reply(&err("nonsense")), Err(LAError::AuthenticationFailed));
        assert_eq!(decode_reply(&Json::Null), Err(LAError::AuthenticationFailed));
    }

    #[test]
    fn reuse_window_skips_second_prompt() {
        let mut c = ctx();
        c.touch_id_authentication_allowable_reuse_duration = 10;
        let p = LAPolicy::DeviceOwnerAuthentication;
        assert_eq!(c.evaluate_policy(p, "x", 100, |_| ok()), Ok(()));
        assert_eq!(c.evaluate_policy(p, "x", 105, |_| panic!("prompted again")), Ok(()));
        assert_eq!(c.evaluate_policy(p, "x", 110, |_| err("user_cancel")), Err(LAError::UserCancel));
    }

    #[test]
    fn no_reuse_by_default_and_failure_is_not_remembered() {
        let mut c = ctx();
        let p = LAPolicy::DeviceOwnerAuthentication;
        assert_eq!(c.evaluate_policy(p, "x", 1, |_| err("authentication_failed")), Err(LAError::AuthenticationFailed));
        assert_eq!(c.evaluate_policy(p, "x", 1, |_| ok()), Ok(()));
        assert_eq!(c.evaluate_policy(p, "x", 1, |_| err("user_cancel")), Err(LAError::UserCancel));
    }

    #[test]
    fn reason_reaches_the_prompt() {
        let mut c = ctx();
        let mut seen = String::new();
        let _ = c.evaluate_policy(LAPolicy::DeviceOwnerAuthentication, "Reveal password", 0, |r| {
            seen = r.to_string();
            ok()
        });
        assert_eq!(seen, "Reveal password");
    }

    #[test]
    fn invalidate_fails_everything_and_drops_reuse() {
        let mut c = ctx();
        c.touch_id_authentication_allowable_reuse_duration = 100;
        let p = LAPolicy::DeviceOwnerAuthentication;
        c.evaluate_policy(p, "x", 0, |_| ok()).unwrap();
        c.invalidate();
        assert_eq!(c.can_evaluate_policy(p), Err(LAError::InvalidContext));
        assert_eq!(c.evaluate_policy(p, "x", 1, |_| ok()), Err(LAError::InvalidContext));
    }

    #[test]
    fn error_wire_names_round_trip() {
        for e in [LAError::UserCancel, LAError::AppCancel, LAError::InvalidContext, LAError::PasscodeNotSet, LAError::BiometryNotAvailable] {
            assert_eq!(LAError::from_wire(e.wire()), Some(e));
        }
        assert_eq!(LAError::from_wire("x"), None);
    }
}
