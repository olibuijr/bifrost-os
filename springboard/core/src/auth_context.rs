//! Authentication context (task theme-03): an app-facing session that evaluates one policy against the device model in
//! `biometry`. It tracks the phase of the running evaluation and records the outcome, so a biometric failure counts
//! toward the lockout that `biometry` enforces. Pure model: no prompt, no sensor and no secure storage. The shell
//! owns the device facts; the context only decides, starts and finishes an evaluation.

use crate::biometry::{resolve, Device, Failure, Method, Policy};

/// Why a context refused a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextError {
    /// `invalidate` ran; the context cannot be used again.
    Invalidated,
    /// An evaluation is already running on this context.
    Busy,
    /// `finish` ran while no evaluation was running.
    NotEvaluating,
    /// The policy cannot be satisfied on this device right now.
    Unavailable(Failure),
}

/// Where the context is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ready,
    /// An evaluation runs; the method that will answer it is recorded.
    Evaluating(Method),
    Succeeded(Method),
    Failed(Failure),
    Invalidated,
}

/// One authentication session for a policy on a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthContext {
    policy: Policy,
    device: Device,
    phase: Phase,
}

impl AuthContext {
    /// A ready context for `policy` on `device`.
    pub fn new(policy: Policy, device: Device) -> AuthContext {
        AuthContext { policy, device, phase: Phase::Ready }
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The device facts the context evaluates against.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Says which method would answer the policy now, without starting an evaluation.
    pub fn can_evaluate(&self) -> Result<Method, Failure> {
        resolve(self.policy.requirement(), &self.device, None)
    }

    /// Starts an evaluation and returns the method that answers it. A refused start leaves the phase unchanged.
    pub fn begin(&mut self) -> Result<Method, ContextError> {
        match self.phase {
            Phase::Invalidated => return Err(ContextError::Invalidated),
            Phase::Evaluating(_) => return Err(ContextError::Busy),
            _ => {}
        }
        let method = self.can_evaluate().map_err(ContextError::Unavailable)?;
        self.phase = Phase::Evaluating(method);
        Ok(method)
    }

    /// Ends the running evaluation. A biometric success clears the failure count; a biometric failure adds one. A
    /// passcode result never changes the biometric count.
    pub fn finish(&mut self, outcome: Result<(), Failure>) -> Result<Phase, ContextError> {
        let method = match self.phase {
            Phase::Evaluating(method) => method,
            Phase::Invalidated => return Err(ContextError::Invalidated),
            _ => return Err(ContextError::NotEvaluating),
        };
        match outcome {
            Ok(()) => {
                if method == Method::Biometry {
                    self.device.record_success();
                }
                self.phase = Phase::Succeeded(method);
            }
            Err(failure) => {
                if method == Method::Biometry {
                    self.device.record_failure();
                }
                self.phase = Phase::Failed(failure);
            }
        }
        Ok(self.phase)
    }

    /// Ends the context for good. A running evaluation is dropped without an outcome.
    pub fn invalidate(&mut self) {
        self.phase = Phase::Invalidated;
    }
}

/// What the authentication view shows for a context: the prompt it offers, the reason it cannot prompt, or the outcome
/// of the last evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// The policy can be evaluated now; the view offers the method that will answer it.
    Prompt(Method),
    /// The policy cannot be evaluated on this device right now; the view explains why.
    Unavailable(Failure),
    /// The last evaluation succeeded with this method.
    Succeeded(Method),
    /// The last evaluation failed for this reason.
    Failed(Failure),
    /// The context is over and the view closes.
    Closed,
}

impl AuthContext {
    /// The view state for the current phase. It reads the phase and the device facts and changes nothing.
    pub fn presentation(&self) -> Presentation {
        match self.phase {
            Phase::Ready => match self.can_evaluate() {
                Ok(method) => Presentation::Prompt(method),
                Err(failure) => Presentation::Unavailable(failure),
            },
            Phase::Evaluating(method) => Presentation::Prompt(method),
            Phase::Succeeded(method) => Presentation::Succeeded(method),
            Phase::Failed(failure) => Presentation::Failed(failure),
            Phase::Invalidated => Presentation::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biometry::BiometryType;

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
    fn begin_runs_one_evaluation_at_a_time() {
        let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
        assert_eq!(ctx.begin(), Ok(Method::Biometry));
        assert_eq!(ctx.phase(), Phase::Evaluating(Method::Biometry));
        assert_eq!(ctx.begin(), Err(ContextError::Busy));
    }

    #[test]
    fn success_clears_the_biometric_failure_count() {
        let mut d = phone();
        d.failures = 2;
        let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
        ctx.begin().unwrap();
        assert_eq!(ctx.finish(Ok(())), Ok(Phase::Succeeded(Method::Biometry)));
        assert_eq!(ctx.device().failures, 0);
    }

    #[test]
    fn biometric_failures_lock_biometry_and_only_the_passcode_remains() {
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
    fn passcode_result_does_not_touch_the_biometric_count() {
        let mut d = phone();
        d.failures = 3;
        let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
        assert_eq!(ctx.begin(), Ok(Method::Passcode));
        ctx.finish(Ok(())).unwrap();
        assert_eq!(ctx.phase(), Phase::Succeeded(Method::Passcode));
        assert_eq!(ctx.device().failures, 3);
    }

    #[test]
    fn unavailable_policy_refuses_to_start_and_keeps_the_phase() {
        let mut d = phone();
        d.failures = 3;
        d.passcode_set = false;
        let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
        assert_eq!(ctx.begin(), Err(ContextError::Unavailable(Failure::PasscodeNotSet)));
        assert_eq!(ctx.phase(), Phase::Ready);
    }

    #[test]
    fn invalidated_context_refuses_every_call() {
        let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
        ctx.invalidate();
        assert_eq!(ctx.begin(), Err(ContextError::Invalidated));
        assert_eq!(ctx.finish(Ok(())), Err(ContextError::Invalidated));
    }

    #[test]
    fn finish_without_a_running_evaluation_is_an_error() {
        let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
        assert_eq!(ctx.finish(Ok(())), Err(ContextError::NotEvaluating));
    }
}
