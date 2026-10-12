//! Authorization requests (parity: Authentication Services, "Authorization requests"): a controller holds the requests a
//! sign-in provider created, runs them once, and ends with one result, a failure or a cancel.
//!
//! Mirrors `ASAuthorizationController` (`init(authorizationRequests:)`, `performRequests()`, `cancel()`) and the
//! result types it reports. Pure model: no sign-in sheet, no provider, no credential store and no network. The shell
//! or the caller shows the choice to the person and reports the outcome with [`AuthorizationController::complete`],
//! [`AuthorizationController::fail`] or [`AuthorizationController::cancel`].

/// The kind of credential a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// A password credential, as `ASAuthorizationPasswordRequest` asks for one.
    Password,
    /// A passkey credential, as `ASAuthorizationPlatformPublicKeyCredentialRequest` asks for one.
    Passkey,
}

/// The outcome of a successful request. Mirrors the cases of `ASAuthorizationResult` that this model carries.
#[derive(Clone, PartialEq, Eq)]
pub enum AuthorizationResult {
    /// A password the person chose or typed for `user`.
    Password { user: String, password: String },
    /// A passkey assertion: the credential identifier and the user handle the relying party stored.
    Passkey { credential_id: Vec<u8>, user_id: Vec<u8> },
}

/// Debug output never shows the password.
impl std::fmt::Debug for AuthorizationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorizationResult::Password { user, .. } => {
                f.debug_struct("Password").field("user", user).field("password", &"<redacted>").finish()
            }
            AuthorizationResult::Passkey { credential_id, user_id } => {
                f.debug_struct("Passkey").field("credential_id", credential_id).field("user_id", user_id).finish()
            }
        }
    }
}

impl AuthorizationResult {
    /// The request kind this result answers.
    pub fn kind(&self) -> RequestKind {
        match self {
            AuthorizationResult::Password { .. } => RequestKind::Password,
            AuthorizationResult::Passkey { .. } => RequestKind::Passkey,
        }
    }
}

/// Where a controller is in its life. A controller runs its requests once and then stays in a final phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Created, not yet performed.
    Idle,
    /// Performed; waiting for the person to answer one of the requests.
    Performing,
    /// Answered with a result.
    Completed(AuthorizationResult),
    /// The provider reported a failure, with a short reason for logs.
    Failed(String),
    /// Cancelled while performing.
    Cancelled,
}

/// Why a controller refused a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerError {
    /// The controller was created with no requests.
    NoRequests,
    /// `perform` ran more than once.
    AlreadyPerformed,
    /// The call needs a performing controller and this one is not performing.
    NotPerforming,
    /// The result is of a kind that none of the requests asked for.
    ResultNotRequested(RequestKind),
}

/// Manages a set of authorization requests from one provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationController {
    requests: Vec<RequestKind>,
    phase: Phase,
}

impl AuthorizationController {
    /// A controller for `requests`, in the order given. Duplicate kinds are kept once, in first-seen order.
    pub fn new(requests: &[RequestKind]) -> Result<AuthorizationController, ControllerError> {
        let mut kinds: Vec<RequestKind> = Vec::new();
        for kind in requests {
            if !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
        if kinds.is_empty() {
            return Err(ControllerError::NoRequests);
        }
        Ok(AuthorizationController { requests: kinds, phase: Phase::Idle })
    }

    /// The request kinds this controller manages, in order.
    pub fn requests(&self) -> &[RequestKind] {
        &self.requests
    }

    /// The current phase.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Starts the requests. Only an idle controller can be performed, and only once.
    pub fn perform(&mut self) -> Result<(), ControllerError> {
        match self.phase {
            Phase::Idle => {
                self.phase = Phase::Performing;
                Ok(())
            }
            _ => Err(ControllerError::AlreadyPerformed),
        }
    }

    /// Ends the performing controller with a result. The result must answer one of the requests.
    pub fn complete(&mut self, result: AuthorizationResult) -> Result<(), ControllerError> {
        if self.phase != Phase::Performing {
            return Err(ControllerError::NotPerforming);
        }
        let kind = result.kind();
        if !self.requests.contains(&kind) {
            return Err(ControllerError::ResultNotRequested(kind));
        }
        self.phase = Phase::Completed(result);
        Ok(())
    }

    /// Ends the performing controller with a failure and a reason.
    pub fn fail(&mut self, reason: &str) -> Result<(), ControllerError> {
        if self.phase != Phase::Performing {
            return Err(ControllerError::NotPerforming);
        }
        self.phase = Phase::Failed(reason.to_string());
        Ok(())
    }

    /// Cancels the active requests. Only a performing controller can be cancelled.
    pub fn cancel(&mut self) -> Result<(), ControllerError> {
        if self.phase != Phase::Performing {
            return Err(ControllerError::NotPerforming);
        }
        self.phase = Phase::Cancelled;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password(user: &str) -> AuthorizationResult {
        AuthorizationResult::Password { user: user.to_string(), password: "secret".to_string() }
    }

    fn passkey() -> AuthorizationResult {
        AuthorizationResult::Passkey { credential_id: vec![1, 2], user_id: vec![9] }
    }

    #[test]
    fn a_controller_needs_at_least_one_request() {
        assert_eq!(AuthorizationController::new(&[]), Err(ControllerError::NoRequests));
    }

    #[test]
    fn duplicate_kinds_are_kept_once_in_first_seen_order() {
        let c = AuthorizationController::new(&[RequestKind::Passkey, RequestKind::Password, RequestKind::Passkey]).unwrap();
        assert_eq!(c.requests(), &[RequestKind::Passkey, RequestKind::Password]);
        assert_eq!(c.phase(), &Phase::Idle);
    }

    #[test]
    fn perform_runs_once_and_a_second_perform_is_refused() {
        let mut c = AuthorizationController::new(&[RequestKind::Password]).unwrap();
        assert_eq!(c.perform(), Ok(()));
        assert_eq!(c.phase(), &Phase::Performing);
        assert_eq!(c.perform(), Err(ControllerError::AlreadyPerformed));
    }

    #[test]
    fn a_result_for_a_requested_kind_completes_the_controller() {
        let mut c = AuthorizationController::new(&[RequestKind::Password, RequestKind::Passkey]).unwrap();
        c.perform().unwrap();
        assert_eq!(c.complete(password("anna")), Ok(()));
        assert_eq!(c.phase(), &Phase::Completed(password("anna")));
        // A completed controller takes no more answers.
        assert_eq!(c.complete(passkey()), Err(ControllerError::NotPerforming));
    }

    #[test]
    fn a_result_of_an_unrequested_kind_is_refused_and_leaves_the_controller_performing() {
        let mut c = AuthorizationController::new(&[RequestKind::Password]).unwrap();
        c.perform().unwrap();
        assert_eq!(c.complete(passkey()), Err(ControllerError::ResultNotRequested(RequestKind::Passkey)));
        assert_eq!(c.phase(), &Phase::Performing);
    }

    #[test]
    fn answers_before_perform_are_refused() {
        let mut c = AuthorizationController::new(&[RequestKind::Password]).unwrap();
        assert_eq!(c.complete(password("anna")), Err(ControllerError::NotPerforming));
        assert_eq!(c.fail("late"), Err(ControllerError::NotPerforming));
        assert_eq!(c.cancel(), Err(ControllerError::NotPerforming));
    }

    #[test]
    fn cancel_and_fail_end_a_performing_controller_with_their_phase() {
        let mut cancelled = AuthorizationController::new(&[RequestKind::Passkey]).unwrap();
        cancelled.perform().unwrap();
        assert_eq!(cancelled.cancel(), Ok(()));
        assert_eq!(cancelled.phase(), &Phase::Cancelled);

        let mut failed = AuthorizationController::new(&[RequestKind::Passkey]).unwrap();
        failed.perform().unwrap();
        assert_eq!(failed.fail("no authenticator"), Ok(()));
        assert_eq!(failed.phase(), &Phase::Failed("no authenticator".to_string()));
    }

    #[test]
    fn a_result_reports_the_kind_it_answers() {
        assert_eq!(password("anna").kind(), RequestKind::Password);
        assert_eq!(passkey().kind(), RequestKind::Passkey);
    }
}
