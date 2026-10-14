//! Parity tests (sdk-tests-08) for the authentication context: one evaluation at a time, the biometric failure count that
//! leads to lockout, passcode results that leave that count alone, and the refusals. Pure host tests with concrete values:
//! no sensor, no prompt, no secure storage, no clock.

use bifrost_core::auth_context::{AuthContext, ContextError, Phase};
use bifrost_core::biometry::{BiometryType, Device, Failure, Method, Policy};

fn phone() -> Device {
    Device {
        biometry: BiometryType::Face,
        enrolled_sets: vec!["owner".to_string()],
        passcode_set: true,
        failures: 0,
        max_failures: 3,
    }
}

#[test]
fn context_begin_runs_one_evaluation_at_a_time() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    assert_eq!(ctx.begin(), Ok(Method::Biometry));
    assert_eq!(ctx.phase(), Phase::Evaluating(Method::Biometry));
    assert_eq!(ctx.begin(), Err(ContextError::Busy));
}

#[test]
fn context_biometric_success_clears_the_failure_count() {
    let mut d = phone();
    d.failures = 2;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    ctx.begin().unwrap();
    assert_eq!(ctx.finish(Ok(())), Ok(Phase::Succeeded(Method::Biometry)));
    assert_eq!(ctx.device().failures, 0);
}

#[test]
fn context_biometric_failure_adds_one_to_the_count() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.begin().unwrap();
    assert_eq!(
        ctx.finish(Err(Failure::BiometryUnavailable)),
        Ok(Phase::Failed(Failure::BiometryUnavailable))
    );
    assert_eq!(ctx.device().failures, 1);
}

#[test]
fn context_three_biometric_failures_leave_only_the_passcode() {
    let mut ctx = AuthContext::new(Policy::BiometricsOnly, phone());
    for _ in 0..3 {
        ctx.begin().unwrap();
        ctx.finish(Err(Failure::BiometryUnavailable)).unwrap();
    }
    assert_eq!(ctx.device().failures, 3);
    assert_eq!(ctx.can_evaluate(), Err(Failure::BiometryLockout));
    assert_eq!(
        AuthContext::new(Policy::DeviceOwner, ctx.device().clone()).can_evaluate(),
        Ok(Method::Passcode)
    );
}

#[test]
fn context_passcode_result_does_not_touch_the_biometric_count() {
    let mut d = phone();
    d.failures = 3;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    assert_eq!(ctx.begin(), Ok(Method::Passcode));
    ctx.finish(Ok(())).unwrap();
    assert_eq!(ctx.phase(), Phase::Succeeded(Method::Passcode));
    assert_eq!(ctx.device().failures, 3);
}

#[test]
fn context_without_enrolled_identities_falls_back_to_the_passcode() {
    let d = Device { enrolled_sets: Vec::new(), ..phone() };
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    assert_eq!(ctx.begin(), Ok(Method::Passcode));
}

#[test]
fn context_unavailable_policy_refuses_to_start_and_keeps_the_phase() {
    let mut d = phone();
    d.failures = 3;
    d.passcode_set = false;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    assert_eq!(ctx.begin(), Err(ContextError::Unavailable(Failure::PasscodeNotSet)));
    assert_eq!(ctx.phase(), Phase::Ready);
}

#[test]
fn context_invalidated_context_refuses_every_call() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.invalidate();
    assert_eq!(ctx.phase(), Phase::Invalidated);
    assert_eq!(ctx.begin(), Err(ContextError::Invalidated));
    assert_eq!(ctx.finish(Ok(())), Err(ContextError::Invalidated));
}

#[test]
fn context_finish_without_a_running_evaluation_is_an_error() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    assert_eq!(ctx.finish(Ok(())), Err(ContextError::NotEvaluating));
    assert_eq!(ctx.phase(), Phase::Ready);
}
