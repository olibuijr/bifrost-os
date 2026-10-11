//! The real [`Gate`]: [`Auth`] behind a mutex. The mutex is never held across a PAM check (pam_unix sleeps
//! about two seconds on a wrong password), so `session.lock`, `login.account` and `power.*` stay responsive
//! while a login is being verified. Overlapping checks are refused by `Auth`'s `verifying` flag instead.

use super::auth::{AccountInfo, Auth, AuthError, Begin, PwPlan};
use super::pam::Verify;
use super::{obj, s, Gate, PErr, Peer};
use bifrost_core::json::Json;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard};

pub struct AuthGate {
    auth: Mutex<Auth>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Maps an auth error to the protocol error, with the `fails` / `retry_secs` extras.
pub fn perr(e: AuthError) -> PErr {
    let mut p = PErr::new(e.code(), format!("{e:?}"));
    match e {
        AuthError::WrongPassword { fails, retry_secs } => {
            p.extra.push(("fails".into(), Json::Num(fails as f64)));
            p.extra.push(("retry_secs".into(), Json::Num(retry_secs as f64)));
        }
        AuthError::LockedOut { retry_secs } => p.extra.push(("retry_secs".into(), Json::Num(retry_secs as f64))),
        AuthError::BadParams(m) | AuthError::Internal(m) => p.message = m,
        _ => {}
    }
    p
}

impl AuthGate {
    /// The system gate: PAM `bifrost-login`, the installed account. Probes PAM once (see `Auth::probe_boot`).
    pub fn system() -> AuthGate {
        let mut a = Auth::new();
        a.probe_boot();
        AuthGate::new(a)
    }

    pub fn new(auth: Auth) -> AuthGate {
        AuthGate { auth: Mutex::new(auth) }
    }

    /// A poisoned lock means a panic mid-update; the state is still consistent enough to stay locked.
    fn lock_auth(&self) -> MutexGuard<'_, Auth> {
        self.auth.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs a password check with the mutex released; a panic inside PAM becomes an internal error.
    fn verify(&self, v: std::sync::Arc<dyn super::pam::Verifier>, user: &str, password: &str) -> Verify {
        catch_unwind(AssertUnwindSafe(|| v.verify(user, password))).unwrap_or_else(|_| Verify::Internal("verifier panicked".into()))
    }
}

fn info_json(i: AccountInfo) -> Json {
    obj(vec![
        ("username", s(i.username)),
        ("full_name", s(i.full_name)),
        ("uid", Json::Num(i.uid as f64)),
        ("has_password", Json::Bool(i.has_password)),
        ("locked_out_secs", Json::Num(i.locked_out_secs as f64)),
    ])
}

impl Gate for AuthGate {
    fn account(&self) -> Result<Json, PErr> {
        self.lock_auth().account_info().map(info_json).map_err(perr)
    }

    fn authenticate(&self, password: &str) -> Result<String, PErr> {
        let at = now();
        let (begin, verifier) = {
            let mut a = self.lock_auth();
            (a.begin_authenticate(password, at).map_err(perr)?, a.verifier())
        };
        match begin {
            Begin::Done(t) => Ok(t),
            Begin::Verify(user) => {
                let r = self.verify(verifier, &user, password);
                self.lock_auth().finish_authenticate(r, at).map_err(perr)
            }
        }
    }

    fn require_unlocked(&self, token: &str) -> Result<(), PErr> {
        self.lock_auth().require_unlocked(Some(token)).map_err(perr)
    }

    fn lock(&self) {
        self.lock_auth().lock();
    }

    fn set_password(&self, token: Option<&str>, peer: Peer, old: &str, new: &str) -> Result<(), PErr> {
        let at = now();
        let (plan, verifier) = {
            let mut a = self.lock_auth();
            (a.plan_set_password(token, peer.console_shell && !peer.root_socket, new, at).map_err(perr)?, a.verifier())
        };
        let user = match plan {
            PwPlan::First(u) | PwPlan::Skip(u) => u,
            PwPlan::Verify(u) => {
                let r = self.verify(verifier, &u, old);
                self.lock_auth().finish_verify_old(r, at).map_err(perr)?;
                u
            }
        };
        self.lock_auth().run_chpasswd(&user, new).map_err(perr)
    }
}
