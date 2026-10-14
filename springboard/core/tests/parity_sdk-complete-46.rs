//! Parity sdk-complete-46: headless tests of the view state an authentication context presents. No prompt is shown and
//! no sensor is read.

use bifrost_core::auth_context::{AuthContext, Presentation};
use bifrost_core::biometry::{BiometryType, Device, Failure, Method, Policy};

fn phone() -> Device {
    Device {
        biometry: BiometryType::Face,
        enrolled_sets: vec!["owner".into()],
        passcode_set: true,
        failures: 0,
        max_failures: 3,
    }
}

#[test]
fn headless_a_ready_context_with_biometry_presents_the_biometric_prompt() {
    let ctx = AuthContext::new(Policy::DeviceOwner, phone());
    assert_eq!(ctx.presentation(), Presentation::Prompt(Method::Biometry));
}

#[test]
fn headless_a_biometric_only_policy_on_a_device_without_a_sensor_explains_why() {
    let mut d = phone();
    d.biometry = BiometryType::None;
    let ctx = AuthContext::new(Policy::BiometricsOnly, d);
    assert_eq!(ctx.presentation(), Presentation::Unavailable(Failure::BiometryUnavailable));
}

#[test]
fn headless_a_running_evaluation_keeps_the_prompt_for_its_method() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.begin().unwrap();
    assert_eq!(ctx.presentation(), Presentation::Prompt(Method::Biometry));
}

#[test]
fn headless_the_outcome_of_an_evaluation_is_presented_until_the_next_begin() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.begin().unwrap();
    ctx.finish(Err(Failure::BiometryLockout)).unwrap();
    assert_eq!(ctx.presentation(), Presentation::Failed(Failure::BiometryLockout));
}

#[test]
fn headless_a_successful_passcode_evaluation_is_presented_as_succeeded() {
    let mut d = phone();
    d.failures = 3;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    ctx.begin().unwrap();
    ctx.finish(Ok(())).unwrap();
    assert_eq!(ctx.presentation(), Presentation::Succeeded(Method::Passcode));
}

#[test]
fn headless_an_invalidated_context_presents_a_closed_view() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.invalidate();
    assert_eq!(ctx.presentation(), Presentation::Closed);
}
