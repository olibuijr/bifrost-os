//! Blocking client for `bifrost-privd` (see `privd`). The user session talks to the daemon over its socket;
//! a process that is itself root (host tests, the installer, `bifrost pc smoke`, a lab root shell) uses
//! `Local`, which runs the same methods in-process, so nothing there changes.
//!
//! `connect()` picks the socket from `SPRINGBOARD_PRIVD` (the session script sets it). The session token from
//! `login.authenticate` is kept in memory with `set_token` and added to every authenticated call.

use crate::privd::{self, methods, PErr};
use bifrost_core::json::Json;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// Stores (or clears, on lock and logout) the session token.
pub fn set_token(t: Option<String>) {
    if let Ok(mut g) = TOKEN.lock() {
        *g = t;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClientError {
    /// a privd error code, or `unavailable` when the daemon cannot be reached
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ClientError {}

impl From<PErr> for ClientError {
    fn from(e: PErr) -> Self {
        ClientError { code: e.code.into(), message: e.message }
    }
}


/// What `update.check` found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UpdateInfo {
    pub installed: Option<String>,
    pub version: Option<String>,
    pub size: u64,
    pub notes: Option<String>,
}

/// Result of `system.boot_attempt {event: begin}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootAttempt {
    pub attempt: u32,
    pub crash_loop: bool,
}

#[derive(Clone, Debug)]
pub struct Client {
    /// `None` runs the methods in this process (we are root)
    sock: Option<PathBuf>,
    timeout: Duration,
}

/// The client for this process: `Local` for root, otherwise the socket in `SPRINGBOARD_PRIVD`
/// (default `/run/bifrost/privd.sock`).
pub fn connect() -> Client {
    // the user session's script always sets SPRINGBOARD_PRIVD; root, host tests and tools run the methods in-process
    // (a non-root process gains nothing there: the privileged steps fail on permissions)
    match std::env::var("SPRINGBOARD_PRIVD") {
        Ok(sock) if !sock.is_empty() && privd::server::euid() != 0 => Client::remote(sock),
        _ => Client::local(),
    }
}

/// The user's own data directory (`SPRINGBOARD_DATA`, else `~/.local/share/springboard`).
pub fn user_data_dir() -> PathBuf {
    match std::env::var("SPRINGBOARD_DATA") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(crate::bare::platform::data_dir(false, std::env::var("XDG_DATA_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())),
    }
}

impl Client {
    pub fn local() -> Client {
        Client { sock: None, timeout: Duration::from_secs(30) }
    }
    pub fn remote(path: impl Into<PathBuf>) -> Client {
        Client { sock: Some(path.into()), timeout: Duration::from_secs(30) }
    }
    pub fn with_timeout(mut self, t: Duration) -> Client {
        self.timeout = t;
        self
    }
    pub fn is_local(&self) -> bool {
        self.sock.is_none()
    }

    /// One request, one reply. Parameters get the session token as `t`.
    pub fn call(&self, method: &str, params: Vec<(&str, Json)>) -> Result<Json, ClientError> {
        self.call_timeout(method, params, self.timeout)
    }

    pub fn call_timeout(&self, method: &str, params: Vec<(&str, Json)>, timeout: Duration) -> Result<Json, ClientError> {
        let fields: Vec<(String, Json)> = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let Some(sock) = &self.sock else {
            return methods::call(method, &Json::Obj(fields)).map_err(Into::into);
        };
        let token = TOKEN.lock().ok().and_then(|g| g.clone());
        bifrost_core::privd_wire::call(sock, token.as_deref(), method, fields, timeout).map_err(|e| ClientError { code: e.code, message: e.message })
    }

    /// Runs one privileged disk request (`disk.*`, see `bifrost_core::disk_request`) and returns the tools that
    /// completed, in order. The helper plans it against its own snapshot of the disks; a refusal comes back as
    /// `bad_params` with the reason as the message.
    pub fn disk_request(&self, request: &bifrost_core::disk_request::DiskRequest) -> Result<Vec<String>, ClientError> {
        let fields = match request.to_params() {
            Json::Obj(fields) => fields,
            _ => Vec::new(),
        };
        let params = fields.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let r = self.call_timeout(request.method(), params, Duration::from_secs(3600))?;
        Ok(r.get("steps").and_then(Json::as_array).map(|a| a.iter().filter_map(|j| j.as_str().map(str::to_string)).collect()).unwrap_or_default())
    }

    pub fn ping(&self) -> Result<String, ClientError> {
        Ok(self.call("system.ping", vec![])?.str_field("version").unwrap_or_default())
    }

    /// `power.*` by systemctl verb: poweroff, reboot or suspend.
    pub fn power(&self, verb: &str) -> std::io::Result<()> {
        let m = match verb {
            "poweroff" => "power.shutdown",
            "reboot" => "power.restart",
            "suspend" => "power.sleep",
            v => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("unknown power verb {v}"))),
        };
        // suspend returns after resume
        self.call_timeout(m, vec![], Duration::from_secs(3600)).map(|_| ()).map_err(std::io::Error::other)
    }

    pub fn set_timezone(&self, name: &str) -> Result<(), ClientError> {
        self.call("time.set_timezone", vec![("name", privd::s(name))]).map(|_| ())
    }

    /// Changes the account password (`account.set_password`); `old` is empty for a first password.
    pub fn set_password(&self, old: &str, new: &str) -> Result<(), ClientError> {
        self.call("account.set_password", vec![("old", privd::s(old)), ("new", privd::s(new))]).map(|_| ())
    }

    pub fn set_ntp(&self, on: bool) -> Result<(), ClientError> {
        self.call("time.set_ntp", vec![("on", Json::Bool(on))]).map(|_| ())
    }

    /// Settings > Users & Groups: creates an account (`admin` adds it to the administrators).
    pub fn user_add(&self, name: &str, full_name: &str, admin: bool, password: &str) -> Result<(), ClientError> {
        self.call("user.add", vec![("name", privd::s(name)), ("full_name", privd::s(full_name)), ("admin", Json::Bool(admin)), ("password", privd::s(password))]).map(|_| ())
    }

    pub fn user_delete(&self, name: &str, remove_home: bool) -> Result<(), ClientError> {
        self.call("user.delete", vec![("name", privd::s(name)), ("remove_home", Json::Bool(remove_home))]).map(|_| ())
    }

    pub fn user_set_admin(&self, name: &str, admin: bool) -> Result<(), ClientError> {
        self.call("user.set_admin", vec![("name", privd::s(name)), ("admin", Json::Bool(admin))]).map(|_| ())
    }

    /// Another account's password (the installed account changes its own through `account.set_password`).
    pub fn user_set_password(&self, name: &str, new: &str) -> Result<(), ClientError> {
        self.call("user.set_password", vec![("name", privd::s(name)), ("new", privd::s(new))]).map(|_| ())
    }

    /// The installed account's own password; the login service checks `old`.
    pub fn account_set_password(&self, old: &str, new: &str) -> Result<(), ClientError> {
        self.call("account.set_password", vec![("old", privd::s(old)), ("new", privd::s(new))]).map(|_| ())
    }

    pub fn group_add(&self, name: &str) -> Result<(), ClientError> {
        self.call("group.add", vec![("name", privd::s(name))]).map(|_| ())
    }

    pub fn group_delete(&self, name: &str) -> Result<(), ClientError> {
        self.call("group.delete", vec![("name", privd::s(name))]).map(|_| ())
    }

    pub fn group_member(&self, group: &str, user: &str, add: bool) -> Result<(), ClientError> {
        self.call("group.member", vec![("group", privd::s(group)), ("user", privd::s(user)), ("add", Json::Bool(add))]).map(|_| ())
    }

    /// Settings > Sharing: the computer name people see and/or the local host name.
    pub fn set_names(&self, computer: Option<&str>, local: Option<&str>) -> Result<(), ClientError> {
        let mut p = vec![];
        if let Some(c) = computer {
            p.push(("computer", privd::s(c)));
        }
        if let Some(l) = local {
            p.push(("local", privd::s(l)));
        }
        self.call("system.set_names", p).map(|_| ())
    }

    /// Settings > Sharing: starts and enables (or stops and disables) a sharing service by id.
    pub fn set_service(&self, id: &str, on: bool) -> Result<(), ClientError> {
        self.call("service.set", vec![("id", privd::s(id)), ("on", Json::Bool(on))]).map(|_| ())
    }

    pub fn update_check(&self) -> Result<UpdateInfo, ClientError> {
        let r = self.call_timeout("update.check", vec![], Duration::from_secs(300))?;
        Ok(UpdateInfo { installed: r.str_field("installed"), version: r.str_field("version"), size: r.get("size").and_then(Json::as_u64).unwrap_or(0), notes: r.str_field("notes") })
    }

    /// Runs the update guard; the caller exits 3 on success so the session script applies it.
    pub fn update_apply(&self, kind: &str) -> Result<(), ClientError> {
        self.call_timeout("update.apply", vec![("kind", privd::s(kind))], Duration::from_secs(3600)).map(|_| ())
    }

    pub fn update_rollback(&self) -> Result<(), ClientError> {
        self.call_timeout("update.rollback", vec![], Duration::from_secs(300)).map(|_| ())
    }

    pub fn update_status(&self) -> Result<String, ClientError> {
        Ok(self.call("update.status", vec![])?.str_field("status").unwrap_or_default())
    }

    /// Asks privd to install store app `id`. privd fetches the catalogue itself and checks the package against
    /// the catalogue hash; the caller supplies no path and no hash. Returns the app id.
    pub fn store_install(&self, id: &str) -> Result<String, ClientError> {
        let r = self.call_timeout("store.install", vec![("id", privd::s(id))], Duration::from_secs(900))?;
        r.str_field("id").ok_or(ClientError { code: "internal".into(), message: "reply has no app id".into() })
    }

    pub fn store_uninstall(&self, id: &str) -> Result<(), ClientError> {
        self.call("store.uninstall", vec![("id", privd::s(id))]).map(|_| ())
    }

    /// The crash-loop latch: `begin` counts a start, `stable` clears the counter.
    pub fn boot_attempt(&self, event: &str) -> Result<BootAttempt, ClientError> {
        let r = self.call("system.boot_attempt", vec![("event", privd::s(event))])?;
        Ok(BootAttempt { attempt: r.get("attempt").and_then(Json::as_u64).unwrap_or(0) as u32, crash_loop: r.get("crash_loop") == Some(&Json::Bool(true)) })
    }

    pub fn restore_apply(&self) -> Result<u64, ClientError> {
        Ok(self.call("backup.restore_apply", vec![])?.get("files").and_then(Json::as_u64).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privd::server::{self, Config, Exec};
    use std::sync::Arc;

    struct Echo;
    impl Exec for Echo {
        fn call(&self, method: &str, params: &Json) -> Result<Json, PErr> {
            match method {
                "update.check" => Ok(privd::obj(vec![("installed", privd::s("1-1")), ("version", privd::s("2-1")), ("size", Json::Num(9.0))])),
                "time.set_ntp" => Ok(params.clone()),
                _ => Err(PErr::bad("nope")),
            }
        }
    }

    struct Open;
    impl privd::Gate for Open {
        fn account(&self) -> Result<Json, PErr> {
            Ok(Json::Null)
        }
        fn authenticate(&self, _: &str) -> Result<String, PErr> {
            Ok("t0".into())
        }
        fn require_unlocked(&self, t: &str) -> Result<(), PErr> {
            if t == "t0" { Ok(()) } else { Err(PErr::new("locked", "locked")) }
        }
        fn lock(&self) {}
        fn set_password(&self, _: Option<&str>, _: privd::Peer, _: &str, _: &str) -> Result<(), PErr> {
            Ok(())
        }
    }

    #[test]
    fn typed_wrappers_over_a_temp_socket() {
        let dir = std::env::temp_dir().join(format!("privd-client-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("c.sock");
        let cfg = Config { sock: sock.clone(), root_sock: None, group_gid: None, allowed_uid: Some(server::euid()), check_exe: false, exes: vec![] };
        server::serve(cfg, Arc::new(Open), Arc::new(Echo)).unwrap();
        let c = Client::remote(&sock);
        assert!(!c.is_local());
        // no token yet
        assert_eq!(c.update_check().unwrap_err().code, "unauthorized");
        set_token(Some("t0".into()));
        let u = c.update_check().unwrap();
        assert_eq!((u.installed.as_deref(), u.version.as_deref(), u.size), (Some("1-1"), Some("2-1"), 9));
        assert_eq!(c.store_uninstall("x").unwrap_err().code, "bad_params");
        set_token(None);
        assert_eq!(Client::remote(dir.join("missing.sock")).ping().unwrap_err().code, "unavailable");
    }

    #[test]
    fn local_runs_methods_in_process() {
        let c = Client::local();
        assert!(c.is_local());
        assert_eq!(c.call("nope", vec![]).unwrap_err().code, "bad_params");
        assert_eq!(c.power("explode").unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }
}
