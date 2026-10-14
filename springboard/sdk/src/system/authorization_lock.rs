//! The lock of a privileged gate: a locked or unlocked state, the button that changes it, and the reply of the
//! system's password check. The check runs in the privileged daemon (`login.authenticate`, see
//! `docs/privd-protocol.md`): a correct password returns a token that unlocks the gate, and a wrong or refused
//! password returns one of the error codes modelled here with the text the gate shows.

/// Whether the gate is locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockState {
    Locked,
    Unlocked,
}

/// The action the gate's button performs in its current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonType {
    /// Shown while locked: the button unlocks.
    Unlock,
    /// Shown while unlocked: the button locks again.
    Lock,
}

/// Why the password check refused the password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasswordError {
    /// `wrong_password`: the password is incorrect. `retry_secs` is the wait before the next try, 0 when there is none.
    Wrong { failures: u32, retry_secs: u64 },
    /// `locked_out`: too many failures; the check refuses until `retry_secs` have passed.
    LockedOut { retry_secs: u64 },
    /// `no_password`: the account has no password to check.
    NoPassword,
    /// Any other code, kept as written.
    Unclassified(String),
}

impl PasswordError {
    /// Decodes a privileged daemon error code with the fields that come with it. Fields a code does not use are ignored.
    pub fn from_code(code: &str, failures: u32, retry_secs: u64) -> PasswordError {
        match code {
            "wrong_password" => PasswordError::Wrong { failures, retry_secs },
            "locked_out" => PasswordError::LockedOut { retry_secs },
            "no_password" => PasswordError::NoPassword,
            other => PasswordError::Unclassified(other.to_string()),
        }
    }

    /// The text the gate shows for this error.
    pub fn message(&self) -> String {
        match self {
            PasswordError::Wrong { retry_secs: 0, .. } => "Incorrect password.".to_string(),
            PasswordError::Wrong { retry_secs, .. } => format!("Incorrect password. Try again in {}.", seconds(*retry_secs)),
            PasswordError::LockedOut { retry_secs } => format!("Too many failed attempts. Try again in {}.", seconds(*retry_secs)),
            PasswordError::NoPassword => "This account has no password to check.".to_string(),
            PasswordError::Unclassified(_) => "The password could not be checked.".to_string(),
        }
    }
}

fn seconds(n: u64) -> String {
    if n == 1 {
        "1 second".to_string()
    } else {
        format!("{n} seconds")
    }
}

/// The state of one gate: locked or unlocked, the token of the last successful check, and the last error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationLock {
    state: LockState,
    token: Option<String>,
    last_error: Option<PasswordError>,
}

impl Default for AuthorizationLock {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthorizationLock {
    /// A gate that starts locked and has no error to show.
    pub fn new() -> AuthorizationLock {
        AuthorizationLock { state: LockState::Locked, token: None, last_error: None }
    }

    pub fn state(&self) -> LockState {
        self.state
    }

    pub fn is_unlocked(&self) -> bool {
        self.state == LockState::Unlocked
    }

    /// The action the button performs now.
    pub fn button(&self) -> ButtonType {
        match self.state {
            LockState::Locked => ButtonType::Unlock,
            LockState::Unlocked => ButtonType::Lock,
        }
    }

    /// The token of the last successful check, while unlocked.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// The error of the last refused check, until the next attempt succeeds.
    pub fn last_error(&self) -> Option<&PasswordError> {
        self.last_error.as_ref()
    }

    /// The text to show under the gate, when the last check was refused.
    pub fn message(&self) -> Option<String> {
        self.last_error.as_ref().map(PasswordError::message)
    }

    /// Records a successful check. An empty token is refused and leaves the gate as it was; the function reports
    /// whether the gate unlocked.
    pub fn authenticated(&mut self, token: &str) -> bool {
        if token.is_empty() {
            return false;
        }
        self.state = LockState::Unlocked;
        self.token = Some(token.to_string());
        self.last_error = None;
        true
    }

    /// Records a refused check. The gate stays locked and keeps the error for its message.
    pub fn refused(&mut self, error: PasswordError) {
        self.state = LockState::Locked;
        self.token = None;
        self.last_error = Some(error);
    }

    /// Locks the gate again and drops the token.
    pub fn lock(&mut self) {
        self.state = LockState::Locked;
        self.token = None;
    }
}
