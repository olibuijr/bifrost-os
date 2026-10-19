//! The `process-owners` service: who owns each process of the machine, for Activity Monitor.
//!
//! Activity Monitor runs in a user namespace (`--unshare-user`, required by `--uid`), so it sees the host `/proc` but every
//! uid the namespace does not map reads as the overflow uid 65534: its User column showed "nobody" for everything but its
//! own account. The shell runs unsandboxed and reads the real owners itself.
//!
//! Who may use it: only an app that holds the `system-monitor` entitlement (a first-party system app that declares it).
//!
//! Methods:
//! - `owners {}` -> `{users: [{uid, user, pids: [..]}]}`: the real uid and account name of each process, grouped by user so
//!   the answer stays small (at most [`MAX_PIDS`] processes). `user` is the uid as text when `/etc/passwd` has no entry.
use super::{app_has_entitlement, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::wire_json::*;
use std::collections::{BTreeMap, HashMap};

/// The name the service is registered under.
pub const SERVICE: &str = "process-owners";
/// The entitlement an app needs.
pub const ENTITLEMENT: &str = "system-monitor";
/// The most processes one answer lists (a packet is at most 16 KiB).
pub const MAX_PIDS: usize = 2000;

pub struct ProcessOwners;

/// The real uid: the first number of the `Uid:` line of `/proc/<pid>/status`.
fn parse_uid(status: &str) -> Option<u32> {
    status.lines().find(|l| l.starts_with("Uid:"))?.split_whitespace().nth(1)?.parse().ok()
}

/// Account names by uid, from `/etc/passwd`.
fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            Some((f.get(2)?.parse::<u32>().ok()?, f.first()?.to_string()))
        })
        .collect()
}

/// `(pid, uid)` of every process in the host `/proc`.
fn read_owners() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let uid = parse_uid(&std::fs::read_to_string(e.path().join("status")).ok()?)?;
            Some((pid, uid))
        })
        .collect()
}

/// The answer for `owners`: the processes grouped by user, lowest uid first, pids ascending.
fn encode(owners: &[(u32, u32)], names: &HashMap<u32, String>) -> Json {
    let mut by_uid: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(pid, uid) in owners.iter().take(MAX_PIDS) {
        by_uid.entry(uid).or_default().push(pid);
    }
    let users = by_uid
        .into_iter()
        .map(|(uid, mut pids)| {
            pids.sort_unstable();
            let user = names.get(&uid).cloned().unwrap_or_else(|| uid.to_string());
            obj(vec![("uid", num(uid)), ("user", text(&user)), ("pids", Json::Arr(pids.into_iter().map(num).collect()))])
        })
        .collect();
    obj(vec![("users", Json::Arr(users))])
}

impl SystemService for ProcessOwners {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        if !app_has_entitlement(app_id, ENTITLEMENT) {
            return Err(format!("process-owners needs the {ENTITLEMENT} entitlement"));
        }
        match method {
            "owners" => {
                let names = std::fs::read_to_string("/etc/passwd").map(|t| parse_passwd(&t)).unwrap_or_default();
                Ok(encode(&read_owners(), &names))
            }
            other => Err(format!("process-owners has no method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_are_grouped_by_user_with_names_from_passwd() {
        let names = parse_passwd("root:x:0:0:root:/root:/bin/bash\nolibuijr:x:1000:1000::/home/o:/bin/zsh\n");
        let j = encode(&[(30, 1000), (1, 0), (7, 1000), (9, 4242)], &names);
        let users = j.get("users").and_then(|u| u.array()).unwrap();
        assert_eq!(users.len(), 3);
        assert_eq!(users[0].str_field("user").as_deref(), Some("root"));
        assert_eq!(users[1].get("uid").and_then(|u| u.as_u64()), Some(1000));
        let pids: Vec<u64> = users[1].get("pids").and_then(|p| p.array()).unwrap().iter().filter_map(|p| p.as_u64()).collect();
        assert_eq!(pids, [7, 30], "pids ascend");
        assert_eq!(users[2].str_field("user").as_deref(), Some("4242"), "an unknown uid reads as its number");
    }

    #[test]
    fn the_real_uid_is_read_from_the_status_file() {
        assert_eq!(parse_uid("Name:\tbash\nUid:\t1000\t1000\t1000\t1000\n"), Some(1000));
        assert_eq!(parse_uid("Name:\tbash\n"), None);
    }

    #[test]
    fn an_app_without_the_entitlement_is_refused() {
        let mut sh = crate::shell::tests::desktop_shell();
        let r = ProcessOwners.call(&mut sh, "com.example.top", "owners", &Json::Null, 0.0);
        assert!(r.unwrap_err().contains("entitlement"));
    }
}
