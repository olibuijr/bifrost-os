//! The `console` service: who is at this machine, the shell's side of `console_user`. The model is
//! `bifrost_core::console_user` (the rules for picking the console user; it reads no files). This service reads the
//! session records systemd-logind keeps in `/run/systemd/sessions` (one `KEY=value` file per session, named by the
//! session id), turns each into the model's record text, and asks the model for the answer.
//!
//! Methods (no parameters):
//! - `user {}` answers `{user}`: `{name, uid, session}` for the account on the console seat, or `null` when the
//!   sign-in screen is showing (no session qualifies).
//!
//! A session record the model cannot read (a session id that is not a number, an empty user name) is left out, so one
//! damaged record never hides the others. The service sends no events.
use super::{Shell, SystemService};
use bifrost_core::console_user::{console_user, parse_sessions, ConsoleUser};
use bifrost_core::json::Json;
use std::collections::BTreeMap;
use std::path::Path;

/// The service name.
pub const SERVICE: &str = "console";

/// Where systemd-logind writes one record per session.
pub const SESSIONS_DIR: &str = "/run/systemd/sessions";

/// The `console` service.
pub struct ConsoleSeat;

impl SystemService for ConsoleSeat {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "user" => {
                let user = match console_user_in(Path::new(SESSIONS_DIR)) {
                    Some(u) => Json::Obj(vec![
                        ("name".into(), Json::Str(u.name)),
                        ("uid".into(), Json::Num(u.uid as f64)),
                        ("session".into(), Json::Num(u.session_id as f64)),
                    ]),
                    None => Json::Null,
                };
                Ok(Json::Obj(vec![("user".into(), user)]))
            }
            _ => Err(format!("console has no method {method}")),
        }
    }
}

/// The console user according to the session records in `dir`.
pub fn console_user_in(dir: &Path) -> Option<ConsoleUser> {
    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(file) = entry.file_name().into_string() else { continue };
        let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
        let Some(block) = session_block(&file, &text) else { continue };
        if let Ok(mut list) = parse_sessions(&block) {
            sessions.append(&mut list);
        }
    }
    console_user(&sessions)
}

/// One systemd session record turned into the model's record text (`Id`, `UID`, `Name`, `Seat`, `Class`, `State`,
/// `Remote`). `file` is the session file name, which is the session id when the record has no `ID` key. A systemd id
/// such as `c1` keeps its number. `None` when the record lacks a field the model needs or the id has no number.
pub fn session_block(file: &str, text: &str) -> Option<String> {
    let fields: BTreeMap<&str, &str> = text.lines().filter_map(|l| l.split_once('=')).collect();
    let id_text = fields.get("ID").copied().unwrap_or(file);
    let id: u32 = id_text.trim_start_matches('c').parse().ok()?;
    let uid = *fields.get("UID")?;
    let name = *fields.get("USER")?;
    if name.is_empty() {
        return None;
    }
    let class = *fields.get("CLASS")?;
    let state = *fields.get("STATE")?;
    let seat = fields.get("SEAT").copied().unwrap_or("");
    let remote = if fields.get("REMOTE").copied() == Some("1") { "yes" } else { "no" };
    Some(format!("Id={id}\nUID={uid}\nName={name}\nSeat={seat}\nClass={class}\nState={state}\nRemote={remote}\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sb-console-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_active_local_session_on_the_seat_is_the_console_user() {
        let dir = scratch("pick");
        std::fs::write(dir.join("c1"), "UID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\nCLASS=user\nSEAT=seat0\nREMOTE=0\nTYPE=wayland\n").unwrap();
        std::fs::write(dir.join("2"), "UID=1001\nUSER=bob\nSTATE=active\nCLASS=user\nSEAT=seat0\nREMOTE=1\n").unwrap();
        std::fs::write(dir.join("3"), "UID=1002\nUSER=carol\nSTATE=closing\nCLASS=user\nSEAT=seat0\nREMOTE=0\n").unwrap();
        let u = console_user_in(&dir).unwrap();
        assert_eq!(u.name, "alice");
        assert_eq!(u.uid, 1000);
        assert_eq!(u.session_id, 1);
    }

    #[test]
    fn no_qualifying_session_means_the_sign_in_screen() {
        let dir = scratch("none");
        std::fs::write(dir.join("4"), "UID=1003\nUSER=dan\nSTATE=online\nCLASS=user\nSEAT=seat1\nREMOTE=0\n").unwrap();
        assert!(console_user_in(&dir).is_none());
        assert!(console_user_in(&dir.join("missing")).is_none());
    }

    #[test]
    fn a_damaged_record_is_left_out() {
        assert!(session_block("x", "UID=1\nUSER=a\nSTATE=active\nCLASS=user\n").is_none());
        assert!(session_block("5", "UID=1\nUSER=\nSTATE=active\nCLASS=user\n").is_none());
        let block = session_block("c7", "UID=7\nUSER=eve\nSTATE=active\nCLASS=user\n").unwrap();
        assert!(block.starts_with("Id=7\n"));
        assert!(block.contains("Seat=\n"));
    }
}
