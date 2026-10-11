//! Login authentication, lockout and the privd session state machine.
//! `AuthGate` (gate.rs) wraps this in a Mutex and releases it around the blocking PAM check.
use super::pam::{ct_eq, shadow_entry, Hash, Pam, Verifier, Verify};
use crate::bare::account::Account;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

/// Seconds a wrong-password streak locks the login (same curve as the old PIN).
pub fn lockout_secs(fails: u32) -> u64 {
    match fails {
        0..=4 => 0,
        5 => 60,
        6 => 300,
        7 => 900,
        _ => 3600,
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AuthError {
    WrongPassword { fails: u32, retry_secs: u64 },
    LockedOut { retry_secs: u64 },
    NoPassword,
    /// Wrong or missing token.
    Unauthorized,
    /// Session is not Unlocked.
    Locked,
    BadParams(String),
    Internal(String),
}
impl AuthError {
    /// Protocol error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::WrongPassword { .. } => "wrong_password",
            Self::LockedOut { .. } => "locked_out",
            Self::NoPassword => "no_password",
            Self::Unauthorized => "unauthorized",
            Self::Locked => "locked",
            Self::BadParams(_) => "bad_params",
            Self::Internal(_) => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub username: String,
    pub full_name: String,
    pub uid: u32,
    pub has_password: bool,
    pub locked_out_secs: u64,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Session {
    Locked,
    Unlocked { token: String },
}

pub struct Config {
    pub account_file: PathBuf,
    pub shadow: PathBuf,
    pub fails_file: PathBuf,
    pub bypass_flag: PathBuf,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            account_file: "/var/lib/springboard/account".into(),
            shadow: "/etc/shadow".into(),
            fails_file: "/run/bifrost/privd-fails".into(),
            bypass_flag: "/etc/bifrost/no-login".into(),
        }
    }
}

type AccountFn = Box<dyn Fn() -> std::io::Result<Account> + Send>;

/// Shortest password `account.set_password` accepts.
pub const MIN_PASSWORD: usize = 8;

pub struct Auth {
    cfg: Config,
    backend: Arc<dyn Verifier>,
    account: Option<AccountFn>,
    session: Session,
    fails: u32,
    until: i64,
    /// a password check is running outside the lock; a second one is refused so attempts cannot overlap
    verifying: bool,
    /// libpam could not start at boot (`probe_boot`): the only automatic bypass. Never set at run time.
    boot_bypass: bool,
}

/// The first half of a login: either done (bypass) or the user name whose password must now be verified.
#[derive(Debug, PartialEq, Eq)]
pub enum Begin {
    Done(String),
    Verify(String),
}

/// What `set_password` needs before it may run chpasswd.
#[derive(Debug, PartialEq, Eq)]
pub enum PwPlan {
    /// the account has no password yet and the caller is the console shell
    First(String),
    /// verify `old` for this user first
    Verify(String),
    /// the explicit bypass is on: no old password to check
    Skip(String),
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 128-bit random token as 32 hex chars.
pub fn new_token() -> std::io::Result<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// `fails=N\nuntil=T` parser for the persisted counter.
pub fn parse_fails(text: &str) -> (u32, i64) {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .and_then(|v| v.trim().parse::<i64>().ok())
    };
    (get("fails").unwrap_or(0).max(0) as u32, get("until").unwrap_or(0))
}

impl Auth {
    pub fn new() -> Self {
        Self::with(Config::default(), Box::new(Pam::default()), None)
    }

    pub fn with(cfg: Config, backend: Box<dyn Verifier>, account: Option<AccountFn>) -> Self {
        let (fails, until) = std::fs::read_to_string(&cfg.fails_file).map(|t| parse_fails(&t)).unwrap_or((0, 0));
        Auth { cfg, backend: Arc::from(backend), account, session: Session::Locked, fails, until, verifying: false, boot_bypass: false }
    }

    /// The verifier, so a caller can run `verify` without holding the lock around this `Auth`.
    pub fn verifier(&self) -> Arc<dyn Verifier> {
        self.backend.clone()
    }

    /// Boot-time check that libpam can start. When it cannot (library or service file missing) the machine
    /// would otherwise be a brick, so the lock screen opens for an empty password and the journal says so
    /// loudly. Runtime PAM errors never come here: they only ever fail the attempt.
    pub fn probe_boot(&mut self) {
        if self.cfg.bypass_flag.exists() {
            return;
        }
        if let Err(e) = self.backend.probe() {
            self.boot_bypass = true;
            eprintln!("privd: WARNING PAM cannot start ({e}); LOGIN BYPASS ENABLED until privd restarts. Fix pam and libpam, or create {} on purpose", self.cfg.bypass_flag.display());
        }
    }

    /// Always the installed account; a client can never name a user.
    fn account(&self) -> Result<Account, AuthError> {
        let r = match &self.account {
            Some(f) => f(),
            None => Account::installed(),
        };
        r.map_err(|e| AuthError::Internal(format!("account: {e}")))
    }

    fn full_name(&self) -> String {
        std::fs::read_to_string(&self.cfg.account_file)
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("full_name=").map(str::to_string)))
            .unwrap_or_default()
    }

    /// The documented opt-in (`/etc/bifrost/no-login`) or the boot-time PAM failure.
    pub fn bypass_active(&self) -> bool {
        self.boot_bypass || self.cfg.bypass_flag.exists()
    }

    fn hash_class(&self, user: &str) -> Hash {
        std::fs::read_to_string(&self.cfg.shadow).map(|t| shadow_entry(&t, user).0).unwrap_or(Hash::Set)
    }

    /// True when a password login is possible.
    pub fn has_password(&self, user: &str) -> bool {
        !self.bypass_active() && self.hash_class(user) == Hash::Set
    }

    fn left(&self, at: i64) -> u64 {
        if self.until > at {
            ((self.until - at) as u64).min(lockout_secs(self.fails))
        } else {
            0
        }
    }

    fn persist(&self) {
        let _ = std::fs::write(&self.cfg.fails_file, format!("fails={}\nuntil={}\n", self.fails, self.until));
    }

    pub fn account_info(&self) -> Result<AccountInfo, AuthError> {
        let a = self.account()?;
        Ok(AccountInfo {
            has_password: self.has_password(&a.name),
            full_name: self.full_name(),
            uid: a.uid,
            locked_out_secs: self.left(now()),
            username: a.name,
        })
    }

    pub fn authenticate(&mut self, password: &str) -> Result<String, AuthError> {
        self.authenticate_at(password, now())
    }

    /// Both halves in one call, for callers that do not need the lock released in between (tests).
    pub fn authenticate_at(&mut self, password: &str, at: i64) -> Result<String, AuthError> {
        match self.begin_authenticate(password, at)? {
            Begin::Done(t) => Ok(t),
            Begin::Verify(user) => {
                let r = self.backend.clone().verify(&user, password);
                self.finish_authenticate(r, at)
            }
        }
    }

    /// First half of a login, under the lock: bypass, hash class and lockout. On `Verify` the caller checks the
    /// password with [`Auth::verifier`] with the lock released (pam_unix sleeps about two seconds on a wrong
    /// password) and must then call [`Auth::finish_authenticate`].
    pub fn begin_authenticate(&mut self, password: &str, at: i64) -> Result<Begin, AuthError> {
        let a = self.account()?;
        if self.bypass_active() {
            if !password.is_empty() {
                return Err(AuthError::BadParams("bypass accepts an empty password".into()));
            }
            return self.unlock().map(Begin::Done);
        }
        match self.hash_class(&a.name) {
            Hash::Set => {}
            Hash::Locked | Hash::Empty => return Err(AuthError::NoPassword),
            Hash::Missing => return Err(AuthError::Internal("no shadow entry".into())),
        }
        let left = self.left(at);
        if left > 0 {
            return Err(AuthError::LockedOut { retry_secs: left });
        }
        if self.verifying {
            return Err(AuthError::LockedOut { retry_secs: 1 });
        }
        self.verifying = true;
        Ok(Begin::Verify(a.name))
    }

    /// Second half: records the outcome. Internal errors only fail the attempt; they never open the lock.
    pub fn finish_authenticate(&mut self, r: Verify, at: i64) -> Result<String, AuthError> {
        self.verifying = false;
        match r {
            Verify::Ok => {
                self.fails = 0;
                self.until = 0;
                self.persist();
                self.unlock()
            }
            Verify::Wrong => Err(self.note_wrong(at)),
            Verify::Internal(m) => {
                eprintln!("privd: PAM error during login ({m}); the session stays locked");
                Err(AuthError::Internal(m))
            }
        }
    }

    fn note_wrong(&mut self, at: i64) -> AuthError {
        self.fails += 1;
        let l = lockout_secs(self.fails);
        self.until = if l > 0 { at + l as i64 } else { 0 };
        self.persist();
        AuthError::WrongPassword { fails: self.fails, retry_secs: l }
    }

    fn unlock(&mut self) -> Result<String, AuthError> {
        let token = new_token().map_err(|e| AuthError::Internal(format!("urandom: {e}")))?;
        self.session = Session::Unlocked { token: token.clone() };
        Ok(token)
    }

    pub fn is_unlocked(&self) -> bool {
        matches!(self.session, Session::Unlocked { .. })
    }

    /// Gate for every privileged method.
    pub fn require_unlocked(&self, token: Option<&str>) -> Result<(), AuthError> {
        match (&self.session, token) {
            (Session::Locked, _) => Err(AuthError::Locked),
            (Session::Unlocked { token: t }, Some(given)) if ct_eq(t.as_bytes(), given.as_bytes()) => Ok(()),
            _ => Err(AuthError::Unauthorized),
        }
    }

    /// `session.lock` / `session.logout`: invalidates the token.
    pub fn lock(&mut self) {
        self.session = Session::Locked;
    }

    /// `account.set_password` in one call (tests); see [`Auth::plan_set_password`] for the rules.
    pub fn set_password(&mut self, token: Option<&str>, console_shell: bool, old: &str, new: &str) -> Result<(), AuthError> {
        let at = now();
        match self.plan_set_password(token, console_shell, new, at)? {
            PwPlan::Verify(user) => {
                let r = self.backend.clone().verify(&user, old);
                self.finish_verify_old(r, at)?;
                self.run_chpasswd(&user, new)
            }
            PwPlan::First(user) | PwPlan::Skip(user) => self.run_chpasswd(&user, new),
        }
    }

    /// The rules, under the lock:
    /// - an empty hash may get its first password without a token, but only from the console shell;
    /// - a usable hash needs the session token and the old password;
    /// - a locked hash (`passwd -l`, `!`, `*`) is an administrator's decision and is never changed here;
    /// - the new password is at least [`MIN_PASSWORD`] bytes and has no newline, colon or NUL.
    pub fn plan_set_password(&mut self, token: Option<&str>, console_shell: bool, new: &str, at: i64) -> Result<PwPlan, AuthError> {
        let a = self.account()?;
        if new.len() < MIN_PASSWORD || new.contains(['\n', ':', '\0']) {
            return Err(AuthError::BadParams(format!("the new password needs at least {MIN_PASSWORD} characters and no colon or line break")));
        }
        match self.hash_class(&a.name) {
            Hash::Empty => {
                if !console_shell {
                    return Err(AuthError::Unauthorized);
                }
                Ok(PwPlan::First(a.name))
            }
            Hash::Locked => Err(AuthError::NoPassword),
            Hash::Missing => Err(AuthError::Internal("no shadow entry".into())),
            Hash::Set => {
                self.require_unlocked(token)?;
                if self.bypass_active() {
                    return Ok(PwPlan::Skip(a.name));
                }
                let left = self.left(at);
                if left > 0 {
                    return Err(AuthError::LockedOut { retry_secs: left });
                }
                if self.verifying {
                    return Err(AuthError::LockedOut { retry_secs: 1 });
                }
                self.verifying = true;
                Ok(PwPlan::Verify(a.name))
            }
        }
    }

    /// Records the check of the old password. A wrong one counts toward the lockout like a wrong login.
    pub fn finish_verify_old(&mut self, r: Verify, at: i64) -> Result<(), AuthError> {
        self.verifying = false;
        match r {
            Verify::Ok => Ok(()),
            Verify::Wrong => Err(self.note_wrong(at)),
            Verify::Internal(m) => Err(AuthError::Internal(m)),
        }
    }

    /// Runs chpasswd as root for the installed account (blocking is fine: it takes milliseconds).
    pub fn run_chpasswd(&self, user: &str, new: &str) -> Result<(), AuthError> {
        let mut c = Command::new("/usr/bin/chpasswd")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| AuthError::Internal(format!("chpasswd: {e}")))?;
        let mut line = format!("{user}:{new}\n").into_bytes();
        let w = c.stdin.take().map(|mut s| s.write_all(&line));
        super::pam::zeroise(&mut line);
        if !matches!(w, Some(Ok(()))) {
            let _ = c.wait();
            return Err(AuthError::Internal("chpasswd stdin".into()));
        }
        match c.wait() {
            Ok(s) if s.success() => Ok(()),
            _ => Err(AuthError::Internal("chpasswd failed".into())),
        }
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
