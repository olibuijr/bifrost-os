//! The `notify_bus` service: named notifications that apps post to each other, and that the shell posts when the
//! system changes. The model is `bifrost_core::notify_center` (name-keyed notifications with integer tokens, state
//! values and suspend and resume). The SDK client is `bifrost_app::notify_bus`.
//!
//! Calls (JSON params). A token call is answered only for the app that owns the token; another app's token is an error:
//! - `post` `{name}` marks every live, unsuspended token on `name`; replies `{delivered: n}`. A name that starts with
//!   `system.` belongs to the shell and is refused.
//! - `register` `{name, mode?}` makes a token on `name`; replies `{token}`. `mode` `"event"` (the default) pushes a
//!   `notified` event to the app after each post; `"poll"` keeps the mark for `check`. An app holds at most 64 tokens.
//!   The store-change names are refused unless the app holds that store's access (see [`may_hear`]).
//! - `check` `{token}` (poll tokens only) replies `{pending}` and clears the mark.
//! - `suspend` / `resume` `{token}` reply `{ok}`; `cancel` `{token}` removes the token and replies `{ok}`.
//! - `valid` `{token}` replies `{valid}`, which is false for a token that is not the caller's.
//! - `set_state` `{name, state}` stores a whole number for `name` (not a `system.` name); `state` `{name}` replies
//!   `{state}`, zero when none was set.
//!
//! Event `notified` to the owning app, body `{token, name}`: sent on the tick after a post. Posts that arrive before one
//! tick are coalesced into one event per token, as the mark is a flag, not a count.
//!
//! System names the shell posts, once per change, after the first tick (which only records the state):
//! `system.appearance.change` (the scheme went Light or Dark), `system.screen.locked` and `system.screen.unlocked`,
//! `system.network.change` (the Wi-Fi or wired link went up or down), `system.contacts.change` (the contact change log grew: each saved change appends to it),
//! `system.calendar.change` (the events file changed), `system.timezone.change` (the local time zone file
//! changed), `system.hostname.change` (the host name file changed) and `system.volume.mount` or
//! `system.volume.unmount` (the number of removable volumes in the mount table went up or down). Each reaches the apps
//! that hold a token on it and may read that store: a read grant for contacts, the `calendars` permission for events.
//! The clock-set, low-disk-space, disk-cache and log-database names have no source in the shell yet and are not posted.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_core::notify_center::{volume_change, NotifyCenter, HOSTNAME_CHANGE, TIME_ZONE_CHANGE};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

/// The name the service is registered under.
pub const SERVICE: &str = "notify_bus";
/// The event pushed to the owner of an event token after a post.
pub const EVENT: &str = "notified";
/// Posted when the scheme changes between Light and Dark.
pub const APPEARANCE_CHANGE: &str = "system.appearance.change";
/// Posted when the screen locks.
pub const SCREEN_LOCKED: &str = "system.screen.locked";
/// Posted when the screen unlocks.
pub const SCREEN_UNLOCKED: &str = "system.screen.unlocked";
/// Posted when the connection goes up or down (Wi-Fi or wired).
pub const NETWORK_CHANGE: &str = "system.network.change";
/// Posted when the contacts file changes (the Contacts app saved, or the file appeared or went).
pub const CONTACTS_CHANGE: &str = "system.contacts.change";
/// Posted when the events file changes (the Calendar app saved, or the file appeared or went).
pub const CALENDAR_CHANGE: &str = "system.calendar.change";
/// How often the store files are stat'ed, in seconds of shell time.
const STORE_STAT_INTERVAL: f64 = 1.0;
/// Longest name, in bytes.
pub const MAX_NAME: usize = 128;
/// Most tokens one app may hold.
pub const MAX_TOKENS_PER_APP: usize = 64;
/// Names with this prefix are the shell's.
const SYSTEM_PREFIX: &str = "system.";
/// The local time zone. Its target file changes when the zone changes.
const LOCAL_TIME_ZONE: &str = "/etc/localtime";
/// The host name file, rewritten when the host name changes.
const HOST_NAME_FILE: &str = "/etc/hostname";
/// The kernel's mount table.
const MOUNT_TABLE: &str = "/proc/mounts";

/// The file whose change means the contacts changed: the contact change log, which every saved change appends to.
fn contacts_change_log(data_dir: &Path) -> std::path::PathBuf {
    crate::shell::contactchanges::path_for(&crate::shell::contactstore::store_path(data_dir))
}

/// Whether `app` may hear `name`. The store-change names say that a person's private data changed, so only an app
/// with that store's access hears them (the reference platform sends the contacts and event-store change notices only to apps that can
/// read the store). Every other name is open to all apps.
fn may_hear(app: &str, name: &str) -> bool {
    match name {
        CONTACTS_CHANGE => super::contacts::reads(app),
        CALENDAR_CHANGE => super::app_has_permission(app, super::calendar::PERMISSION) || calendar_granted_in_tests(app),
        _ => true,
    }
}

// Installed manifests (where the calendars permission lives) do not exist in unit tests, so tests grant it here.
#[cfg(test)]
thread_local! {
    static TEST_CALENDAR_GRANTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn calendar_granted_in_tests(app: &str) -> bool {
    TEST_CALENDAR_GRANTS.with(|g| g.borrow().iter().any(|a| a == app))
}

#[cfg(not(test))]
fn calendar_granted_in_tests(_: &str) -> bool {
    false
}

/// Who owns a token, the name it is on, and whether it is pushed (`true`) or polled.
struct Owner {
    app: String,
    name: String,
    push: bool,
}

/// A store file's identity: its modification time and its length. Both are kept because `write_atomic` renames a new
/// file into place, and two saves in one timestamp tick leave the time unchanged, so the length tells them apart.
type Stamp = (SystemTime, u64);

/// The shell state the bus watches for system names.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seen {
    light: bool,
    locked: bool,
    /// Wi-Fi or a wired link is up.
    reachable: bool,
    /// The contacts file's stamp, `None` when there is no file.
    contacts: Option<Stamp>,
    /// The events file's stamp, `None` when there is no file.
    calendar: Option<Stamp>,
    /// The local time zone file's stamp, `None` when there is no file.
    time_zone: Option<Stamp>,
    /// The host name file's stamp, `None` when there is no file.
    hostname: Option<Stamp>,
    /// How many removable volumes are mounted.
    volumes: usize,
}

/// The system files the shell watches, as of the last stat.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SystemFiles {
    time_zone: Option<Stamp>,
    hostname: Option<Stamp>,
    volumes: usize,
}

#[derive(Default)]
pub struct NotifyBus {
    center: NotifyCenter,
    owners: BTreeMap<u32, Owner>,
    last: Option<Seen>,
    /// The shell time of the last stat of the store files, `None` before the first tick.
    stat_at: Option<f64>,
    /// The store files' stamps as of `stat_at`: contacts, then calendar.
    stores: (Option<Stamp>, Option<Stamp>),
    /// The system files as of `stat_at`.
    system_files: SystemFiles,
}

impl NotifyBus {
    /// The token, if `app` owns it.
    fn owned(&self, app: &str, token: u32) -> Result<&Owner, String> {
        match self.owners.get(&token) {
            Some(o) if o.app == app => Ok(o),
            _ => Err(format!("notify_bus: token {token} is not one of yours")),
        }
    }

    /// How many tokens `app` holds.
    fn held_by(&self, app: &str) -> usize {
        self.owners.values().filter(|o| o.app == app).count()
    }

    /// Cancels the tokens of every app that `running` does not name. An app that exits never cancels its tokens,
    /// so without this its tokens would stay posted to and count against [`MAX_TOKENS_PER_APP`] on every later launch.
    fn prune(&mut self, running: impl Fn(&str) -> bool) {
        // the command line has no app to run; its tokens are cancelled by the client when it exits (see `control`)
        let gone: Vec<u32> = self.owners.iter().filter(|(_, o)| o.app != CONTROL_OWNER && !running(&o.app)).map(|(&t, _)| t).collect();
        for token in gone {
            self.center.cancel(token);
            self.owners.remove(&token);
        }
    }

    /// Answers one call from `app`. The shell's [`SystemService::call`] delegates here, so the rules can be tested
    /// without a shell.
    fn handle(&mut self, app: &str, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "post" => {
                let name = name_param(params)?;
                if is_system(&name) {
                    return Err(format!("notify_bus: {name} is posted by the shell"));
                }
                let delivered = self.center.post(&name);
                Ok(obj(vec![("delivered", Json::Num(delivered as f64))]))
            }
            "register" => {
                let name = name_param(params)?;
                if !may_hear(app, &name) {
                    return Err(format!("notify_bus: {name} is only for apps with access to that store"));
                }
                let push = match params.str_field("mode").as_deref() {
                    None | Some("event") => true,
                    Some("poll") => false,
                    Some(other) => return Err(format!("notify_bus: mode {other} is not event or poll")),
                };
                if self.held_by(app) >= MAX_TOKENS_PER_APP {
                    return Err(format!("notify_bus: an app holds at most {MAX_TOKENS_PER_APP} tokens"));
                }
                let token = self.center.register(&name);
                self.owners.insert(token, Owner { app: app.to_string(), name, push });
                Ok(obj(vec![("token", Json::Num(token as f64))]))
            }
            "check" => {
                let token = token_param(params)?;
                let owner = self.owned(app, token)?;
                if owner.push {
                    return Err("notify_bus: an event token is not polled; wait for the notified event".into());
                }
                let heard = may_hear(app, &owner.name);
                let pending = self.center.check(token) && heard;
                Ok(obj(vec![("pending", Json::Bool(pending))]))
            }
            "suspend" | "resume" => {
                let token = token_param(params)?;
                self.owned(app, token)?;
                let ok = if method == "suspend" { self.center.suspend(token) } else { self.center.resume(token) };
                Ok(obj(vec![("ok", Json::Bool(ok))]))
            }
            "cancel" => {
                let token = token_param(params)?;
                self.owned(app, token)?;
                self.owners.remove(&token);
                Ok(obj(vec![("ok", Json::Bool(self.center.cancel(token)))]))
            }
            "valid" => {
                let token = token_param(params)?;
                let valid = matches!(self.owners.get(&token), Some(o) if o.app == app);
                Ok(obj(vec![("valid", Json::Bool(valid))]))
            }
            "set_state" => {
                let name = name_param(params)?;
                if is_system(&name) {
                    return Err(format!("notify_bus: {name} is set by the shell"));
                }
                let state = state_param(params)?;
                self.center.set_state(&name, state);
                Ok(Json::Null)
            }
            "state" => {
                let name = name_param(params)?;
                Ok(obj(vec![("state", Json::Num(self.center.state(&name) as f64))]))
            }
            _ => Err(format!("notify_bus has no method {method}")),
        }
    }

    /// Stats the store files under `data_dir` when the interval since the last stat has passed.
    fn stat_stores(&mut self, data_dir: &Path, t: f64) {
        match self.stat_at {
            Some(at) if t - at < STORE_STAT_INTERVAL => {}
            _ => {
                self.stat_at = Some(t);
                let contacts = contacts_change_log(data_dir);
                let calendar = super::calendar::store_path(data_dir);
                self.stores = (stamp(&contacts), stamp(&calendar));
                self.system_files = SystemFiles {
                    time_zone: stamp(Path::new(LOCAL_TIME_ZONE)),
                    hostname: stamp(Path::new(HOST_NAME_FILE)),
                    volumes: mounted_volumes(&std::fs::read_to_string(MOUNT_TABLE).unwrap_or_default()),
                };
            }
        }
    }

    /// Posts the system name for each change since the last call. The first call only records the state.
    fn system_changes(&mut self, now: Seen) {
        if let Some(prev) = self.last.replace(now) {
            if prev.light != now.light {
                self.center.post(APPEARANCE_CHANGE);
            }
            if prev.locked != now.locked {
                self.center.post(if now.locked { SCREEN_LOCKED } else { SCREEN_UNLOCKED });
            }
            if prev.reachable != now.reachable {
                self.center.post(NETWORK_CHANGE);
            }
            if prev.contacts != now.contacts {
                self.center.post(CONTACTS_CHANGE);
            }
            if prev.calendar != now.calendar {
                self.center.post(CALENDAR_CHANGE);
            }
            if prev.time_zone != now.time_zone {
                self.center.post(TIME_ZONE_CHANGE);
            }
            if prev.hostname != now.hostname {
                self.center.post(HOSTNAME_CHANGE);
            }
            if let Some(name) = volume_change(prev.volumes, now.volumes) {
                self.center.post(name);
            }
        }
    }

    /// The `notified` events for the pushed tokens whose mark is set. Each mark is cleared.
    fn deliver(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        for (&token, owner) in self.owners.iter() {
            // the mark is cleared even when the owner may not hear the name, so a later grant does not replay it
            if owner.push && self.center.check(token) && may_hear(&owner.app, &owner.name) {
                let body = obj(vec![("token", Json::Num(token as f64)), ("name", Json::Str(owner.name.clone()))]);
                out.push(Event { app: Some(owner.app.clone()), event: EVENT.to_string(), body });
            }
        }
        out
    }
}

impl SystemService for NotifyBus {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let running = |app: &str| crate::shell::apphost::slot_of(app).is_some_and(|slot| shell.ext.get(slot).is_some_and(|h| h.is_some()));
        self.prune(running);
        self.stat_stores(&shell.data_dir, t);
        let now = Seen {
            light: bifrost_render::appearance::target_is_light(),
            locked: shell.is_locked(),
            reachable: shell.info.online || shell.info.wired,
            contacts: self.stores.0,
            calendar: self.stores.1,
            time_zone: self.system_files.time_zone,
            hostname: self.system_files.hostname,
            volumes: self.system_files.volumes,
        };
        self.system_changes(now);
        self.deliver()
    }
}

/// The owner of the calls the command line makes through the shell's control socket (`notifyutil`).
pub const CONTROL_OWNER: &str = "control";

/// Answers one `notify` control line (`control::Request::Notify`): `post NAME`, `set NAME VALUE`, `get NAME`,
/// `register NAME`, `check TOKEN`, `cancel TOKEN`. The answer is the reply's JSON, which the client reads. `register`
/// makes a poll token (the client checks it until a post comes), owned by [`CONTROL_OWNER`]; the client cancels it when
/// it exits, so a killed client leaves at most [`MAX_TOKENS_PER_APP`] tokens behind for the control owner.
pub fn control(shell: &mut Shell, words: &[String], t: f64) -> Result<String, String> {
    const USAGE: &str = "usage: notify post NAME | set NAME VALUE | get NAME | register NAME | check TOKEN | cancel TOKEN";
    let (verb, args) = words.split_first().ok_or(USAGE)?;
    let name = |n: &str| bifrost_core::json::quote(n);
    let (method, body) = match (verb.as_str(), args) {
        ("post", [n]) => ("post", format!("{{\"name\":{}}}", name(n))),
        ("get", [n]) => ("state", format!("{{\"name\":{}}}", name(n))),
        ("set", [n, value]) => {
            let state: u64 = value.parse().map_err(|_| format!("notify: {value} is not a whole number"))?;
            ("set_state", format!("{{\"name\":{},\"state\":{state}}}", name(n)))
        }
        ("register", [n]) => ("register", format!("{{\"name\":{},\"mode\":\"poll\"}}", name(n))),
        ("check", [token]) => ("check", format!("{{\"token\":{}}}", token_word(token)?)),
        ("cancel", [token]) => ("cancel", format!("{{\"token\":{}}}", token_word(token)?)),
        _ => return Err(USAGE.into()),
    };
    let reply = shell.service_call(CONTROL_OWNER, SERVICE, method, &body, t)?;
    Ok(reply.encode())
}

fn token_word(word: &str) -> Result<u32, String> {
    word.parse::<u32>().ok().filter(|&n| n >= 1).ok_or_else(|| format!("notify: {word} is not a token"))
}

fn is_system(name: &str) -> bool {
    name.starts_with(SYSTEM_PREFIX)
}

/// How many mount points in a mount table (`/proc/mounts` format) are removable volumes under `/media` or `/run/media`.
fn mounted_volumes(table: &str) -> usize {
    table
        .lines()
        .filter(|line| line.split_whitespace().nth(1).is_some_and(|point| point.starts_with("/media/") || point.starts_with("/run/media/")))
        .count()
}

/// The stamp of the file at `path`; `None` when it is missing or unreadable.
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The `name` in `params`: 1 to `MAX_NAME` bytes with no control characters.
fn name_param(params: &Json) -> Result<String, String> {
    match params.str_field("name") {
        Some(n) if !n.is_empty() && n.len() <= MAX_NAME && !n.chars().any(char::is_control) => Ok(n),
        _ => Err(format!("notify_bus: a name of 1 to {MAX_NAME} bytes without control characters is needed")),
    }
}

/// The `token` in `params`: a whole number from 1 up.
fn token_param(params: &Json) -> Result<u32, String> {
    params
        .get("token")
        .and_then(Json::number)
        .filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n <= u32::MAX as f64)
        .map(|n| n as u32)
        .ok_or_else(|| "notify_bus: a token number is needed".to_string())
}

/// The `state` in `params`: a whole number from 0 up to 2^53, the largest that JSON numbers carry exactly.
fn state_param(params: &Json) -> Result<u64, String> {
    params
        .get("state")
        .and_then(Json::number)
        .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= 9_007_199_254_740_992.0)
        .map(|n| n as u64)
        .ok_or_else(|| "notify_bus: state must be a whole number from 0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::services::contacts::{answer_prompt, change_by_user};
    use bifrost_ui::contact_access::Choice;
    use std::time::{Duration, UNIX_EPOCH};

    fn params(fields: Vec<(&str, Json)>) -> Json {
        obj(fields)
    }

    fn name(n: &str) -> Json {
        params(vec![("name", Json::Str(n.into()))])
    }

    fn token(t: u32) -> Json {
        params(vec![("token", Json::Num(t as f64))])
    }

    fn number(r: &Result<Json, String>, key: &str) -> f64 {
        r.as_ref().unwrap().get(key).and_then(Json::number).unwrap()
    }

    fn quiet() -> Seen {
        Seen { light: false, locked: false, reachable: true, contacts: None, calendar: None, time_zone: None, hostname: None, volumes: 0 }
    }

    /// A file last written at `secs` after the epoch, `len` bytes long.
    fn file(secs: u64, len: u64) -> Option<Stamp> {
        Some((UNIX_EPOCH + Duration::from_secs(secs), len))
    }

    /// A quiet shell whose store files were last written at the given seconds after the epoch.
    fn stores(contacts: Option<u64>, calendar: Option<u64>) -> Seen {
        Seen { contacts: contacts.and_then(|secs| file(secs, 0)), calendar: calendar.and_then(|secs| file(secs, 0)), ..quiet() }
    }

    /// A pushed token for `app` on `name`, put in place without the access gate. The calendar permission comes from an
    /// installed manifest, which a unit test cannot install, so the delivery tests use this for that name.
    fn token_past_the_gate(bus: &mut NotifyBus, app: &str, name: &str) {
        if name == CALENDAR_CHANGE {
            TEST_CALENDAR_GRANTS.with(|g| g.borrow_mut().push(app.to_string()));
        }
        let token = bus.center.register(name);
        bus.owners.insert(token, Owner { app: app.into(), name: name.into(), push: true });
    }

    fn event_names(bus: &mut NotifyBus) -> Vec<(Option<String>, String)> {
        bus.deliver().into_iter().map(|e| (e.app, e.body.get("name").and_then(Json::as_str).unwrap().to_string())).collect()
    }

    #[test]
    fn a_register_gives_tokens_from_one_and_a_post_counts_them() {
        let mut bus = NotifyBus::default();
        let a = bus.handle("app.a", "register", &name("app.x.refresh"));
        let b = bus.handle("app.b", "register", &name("app.x.refresh"));
        assert_eq!(number(&a, "token"), 1.0);
        assert_eq!(number(&b, "token"), 2.0);
        let r = bus.handle("app.c", "post", &name("app.x.refresh"));
        assert_eq!(number(&r, "delivered"), 2.0);
        assert_eq!(number(&bus.handle("app.c", "post", &name("app.x.other")), "delivered"), 0.0);
    }

    #[test]
    fn a_pushed_token_gets_one_event_per_tick_that_follows_a_post() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name("app.x.refresh")).unwrap();
        bus.handle("app.a", "post", &name("app.x.refresh")).unwrap();
        bus.handle("app.a", "post", &name("app.x.refresh")).unwrap();
        let events = bus.deliver();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].app.as_deref(), Some("app.a"));
        assert_eq!(events[0].event, EVENT);
        assert_eq!(events[0].body, obj(vec![("token", Json::Num(1.0)), ("name", Json::Str("app.x.refresh".into()))]));
        assert!(bus.deliver().is_empty());
    }

    #[test]
    fn a_polled_token_keeps_its_mark_until_checked_and_is_not_pushed() {
        let mut bus = NotifyBus::default();
        let mode = params(vec![("name", Json::Str("app.x.poll".into())), ("mode", Json::Str("poll".into()))]);
        let t = bus.handle("app.a", "register", &mode).unwrap();
        let id = t.get("token").and_then(Json::number).unwrap() as u32;
        bus.handle("app.b", "post", &name("app.x.poll")).unwrap();
        assert!(bus.deliver().is_empty());
        let first = bus.handle("app.a", "check", &token(id)).unwrap();
        assert_eq!(first.get("pending"), Some(&Json::Bool(true)));
        let second = bus.handle("app.a", "check", &token(id)).unwrap();
        assert_eq!(second.get("pending"), Some(&Json::Bool(false)));
    }

    #[test]
    fn checking_an_event_token_is_refused() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name("app.x.push")).unwrap();
        assert!(bus.handle("app.a", "check", &token(1)).is_err());
    }

    #[test]
    fn only_the_owner_can_suspend_resume_or_cancel_a_token() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name("app.x.own")).unwrap();
        assert!(bus.handle("app.b", "suspend", &token(1)).is_err());
        assert!(bus.handle("app.b", "cancel", &token(1)).is_err());
        assert_eq!(bus.handle("app.b", "valid", &token(1)).unwrap().get("valid"), Some(&Json::Bool(false)));
        assert_eq!(bus.handle("app.a", "valid", &token(1)).unwrap().get("valid"), Some(&Json::Bool(true)));
    }

    #[test]
    fn suspend_drops_posts_and_resume_restores_them() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name("app.x.s")).unwrap();
        bus.handle("app.a", "suspend", &token(1)).unwrap();
        assert_eq!(number(&bus.handle("app.b", "post", &name("app.x.s")), "delivered"), 0.0);
        assert!(bus.deliver().is_empty());
        bus.handle("app.a", "resume", &token(1)).unwrap();
        assert_eq!(number(&bus.handle("app.b", "post", &name("app.x.s")), "delivered"), 1.0);
        assert_eq!(bus.deliver().len(), 1);
    }

    #[test]
    fn cancel_removes_the_token_and_its_events() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name("app.x.c")).unwrap();
        assert_eq!(bus.handle("app.a", "cancel", &token(1)).unwrap().get("ok"), Some(&Json::Bool(true)));
        assert_eq!(bus.handle("app.a", "valid", &token(1)).unwrap().get("valid"), Some(&Json::Bool(false)));
        assert_eq!(number(&bus.handle("app.b", "post", &name("app.x.c")), "delivered"), 0.0);
    }

    #[test]
    fn an_app_holds_at_most_64_tokens() {
        let mut bus = NotifyBus::default();
        for _ in 0..MAX_TOKENS_PER_APP {
            bus.handle("app.a", "register", &name("app.x.many")).unwrap();
        }
        assert!(bus.handle("app.a", "register", &name("app.x.many")).is_err());
        assert!(bus.handle("app.b", "register", &name("app.x.many")).is_ok());
    }

    #[test]
    fn an_exited_app_loses_its_tokens_so_its_next_launch_can_register_again() {
        let mut bus = NotifyBus::default();
        for _ in 0..MAX_TOKENS_PER_APP {
            bus.handle("app.a", "register", &name("app.x.many")).unwrap();
        }
        let kept = number(&bus.handle("app.b", "register", &name("app.x.many")), "token") as u32;
        bus.prune(|app| app == "app.b");
        assert_eq!(bus.owners.len(), 1, "only the running app keeps its token");
        assert!(bus.owners.contains_key(&kept));
        assert!(!bus.center.is_valid(1), "the exited app's token is cancelled in the centre");
        assert_eq!(bus.held_by("app.a"), 0);
        assert!(bus.handle("app.a", "register", &name("app.x.many")).is_ok(), "a relaunch registers again");
        assert_eq!(number(&bus.handle("app.c", "post", &name("app.x.many")), "delivered"), 2.0, "the running app and the relaunched one");
    }

    #[test]
    fn pruning_with_every_app_running_keeps_every_token() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name("app.x.keep")).unwrap();
        bus.handle("app.b", "register", &name("app.x.keep")).unwrap();
        bus.prune(|_| true);
        assert_eq!(bus.owners.len(), 2);
        assert_eq!(number(&bus.handle("app.c", "post", &name("app.x.keep")), "delivered"), 2.0);
    }

    #[test]
    fn a_command_line_poll_token_survives_prune_and_sees_one_post() {
        let mut bus = NotifyBus::default();
        let poll = params(vec![("name", Json::Str("app.x.cli".into())), ("mode", Json::Str("poll".into()))]);
        let id = number(&bus.handle(CONTROL_OWNER, "register", &poll), "token") as u32;
        bus.prune(|_| false);
        assert!(bus.owners.contains_key(&id), "the command line has no app to run, so nothing prunes its token");
        assert_eq!(number(&bus.handle("app.b", "post", &name("app.x.cli")), "delivered"), 1.0);
        assert_eq!(bus.handle(CONTROL_OWNER, "check", &token(id)).unwrap().get("pending"), Some(&Json::Bool(true)));
        assert_eq!(bus.handle(CONTROL_OWNER, "check", &token(id)).unwrap().get("pending"), Some(&Json::Bool(false)), "a check clears the mark");
    }

    #[test]
    fn set_and_get_keep_a_whole_number_state_per_name() {
        let mut bus = NotifyBus::default();
        let set = params(vec![("name", Json::Str("app.x.state".into())), ("state", Json::Num(42.0))]);
        bus.handle(CONTROL_OWNER, "set_state", &set).unwrap();
        assert_eq!(number(&bus.handle(CONTROL_OWNER, "state", &name("app.x.state")), "state"), 42.0);
        assert_eq!(number(&bus.handle(CONTROL_OWNER, "state", &name("app.x.unset")), "state"), 0.0, "an unset name reads 0");
    }

    #[test]
    fn control_words_are_checked_before_the_bus() {
        assert_eq!(token_word("3"), Ok(3));
        assert!(token_word("0").is_err());
        assert!(token_word("x").is_err());
    }

    #[test]
    fn shell_names_cannot_be_posted_or_set_by_apps() {
        let mut bus = NotifyBus::default();
        assert!(bus.handle("app.a", "post", &name("system.network.change")).is_err());
        assert!(bus.handle("app.a", "set_state", &params(vec![("name", Json::Str("system.x".into())), ("state", Json::Num(1.0))])).is_err());
    }

    #[test]
    fn names_and_tokens_are_checked_before_use() {
        let mut bus = NotifyBus::default();
        assert!(bus.handle("app.a", "post", &name("")).is_err());
        assert!(bus.handle("app.a", "post", &name("a\tb")).is_err());
        assert!(bus.handle("app.a", "post", &Json::Null).is_err());
        assert!(bus.handle("app.a", "suspend", &token(0)).is_err());
        assert!(bus.handle("app.a", "suspend", &params(vec![("token", Json::Num(1.5))])).is_err());
        assert!(bus.handle("app.a", "register", &params(vec![("name", Json::Str("n".into())), ("mode", Json::Str("fd".into()))])).is_err());
    }

    #[test]
    fn state_values_are_whole_numbers_and_default_to_zero() {
        let mut bus = NotifyBus::default();
        assert_eq!(number(&bus.handle("app.a", "state", &name("app.x.v")), "state"), 0.0);
        let set = params(vec![("name", Json::Str("app.x.v".into())), ("state", Json::Num(42.0))]);
        assert_eq!(bus.handle("app.a", "set_state", &set), Ok(Json::Null));
        assert_eq!(number(&bus.handle("app.b", "state", &name("app.x.v")), "state"), 42.0);
        let bad = params(vec![("name", Json::Str("app.x.v".into())), ("state", Json::Num(-1.0))]);
        assert!(bus.handle("app.a", "set_state", &bad).is_err());
    }

    #[test]
    fn unknown_methods_are_errors() {
        let mut bus = NotifyBus::default();
        assert_eq!(bus.handle("app.a", "frobnicate", &Json::Null), Err("notify_bus has no method frobnicate".to_string()));
    }

    #[test]
    fn the_first_tick_only_records_the_state() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name(APPEARANCE_CHANGE)).unwrap();
        bus.system_changes(quiet());
        assert!(bus.deliver().is_empty());
    }

    #[test]
    fn a_change_of_scheme_lock_or_link_posts_its_system_name() {
        let mut bus = NotifyBus::default();
        for n in [APPEARANCE_CHANGE, SCREEN_LOCKED, SCREEN_UNLOCKED, NETWORK_CHANGE] {
            bus.handle("app.a", "register", &name(n)).unwrap();
        }
        bus.system_changes(quiet());
        bus.system_changes(Seen { light: true, ..quiet() });
        let names: Vec<String> = bus.deliver().into_iter().map(|e| e.body.get("name").and_then(Json::as_str).unwrap().to_string()).collect();
        assert_eq!(names, vec![APPEARANCE_CHANGE.to_string()]);
        bus.system_changes(Seen { light: true, locked: true, reachable: false, ..quiet() });
        let names: Vec<String> = bus.deliver().into_iter().map(|e| e.body.get("name").and_then(Json::as_str).unwrap().to_string()).collect();
        assert_eq!(names, vec![SCREEN_LOCKED.to_string(), NETWORK_CHANGE.to_string()]);
    }

    #[test]
    fn an_unchanged_state_posts_nothing() {
        let mut bus = NotifyBus::default();
        bus.handle("app.a", "register", &name(SCREEN_LOCKED)).unwrap();
        bus.system_changes(quiet());
        bus.system_changes(quiet());
        assert!(bus.deliver().is_empty());
    }

    #[test]
    fn a_change_of_time_zone_hostname_or_volume_count_posts_its_system_name() {
        let mut bus = NotifyBus::default();
        for n in [TIME_ZONE_CHANGE, HOSTNAME_CHANGE, bifrost_core::notify_center::VOLUME_MOUNT, bifrost_core::notify_center::VOLUME_UNMOUNT] {
            bus.handle("app.a", "register", &name(n)).unwrap();
        }
        bus.system_changes(quiet());
        bus.system_changes(Seen { time_zone: file(10, 1), hostname: file(10, 1), volumes: 1, ..quiet() });
        let names: Vec<String> = event_names(&mut bus).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, vec![TIME_ZONE_CHANGE.to_string(), HOSTNAME_CHANGE.to_string(), bifrost_core::notify_center::VOLUME_MOUNT.to_string()]);
        bus.system_changes(Seen { time_zone: file(10, 1), hostname: file(10, 1), volumes: 0, ..quiet() });
        let names: Vec<String> = event_names(&mut bus).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, vec![bifrost_core::notify_center::VOLUME_UNMOUNT.to_string()]);
    }

    #[test]
    fn the_mount_table_counts_only_removable_volumes() {
        let table = "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /media/olibuijr/USB vfat rw 0 0\ntmpfs /run/media/alice/SD tmpfs rw 0 0\n/dev/sdc1 /home ext4 rw 0 0\n";
        assert_eq!(mounted_volumes(table), 2);
        assert_eq!(mounted_volumes(""), 0);
        assert_eq!(mounted_volumes("garbage\n"), 0);
    }

    #[test]
    fn a_store_change_reaches_only_the_apps_on_that_name() {
        let mut bus = NotifyBus::default();
        answer_prompt("app.a", Choice::All).unwrap();
        bus.handle("app.a", "register", &name(CONTACTS_CHANGE)).unwrap();
        token_past_the_gate(&mut bus, "app.b", CALENDAR_CHANGE);
        bus.system_changes(stores(Some(1), Some(1)));
        assert!(bus.deliver().is_empty(), "the first tick only records the stamps");
        bus.system_changes(stores(Some(2), Some(1)));
        assert_eq!(event_names(&mut bus), vec![(Some("app.a".to_string()), CONTACTS_CHANGE.to_string())]);
        bus.system_changes(stores(Some(2), Some(3)));
        assert_eq!(event_names(&mut bus), vec![(Some("app.b".to_string()), CALENDAR_CHANGE.to_string())]);
        bus.system_changes(stores(Some(2), Some(3)));
        assert!(bus.deliver().is_empty(), "an unchanged store posts nothing");
    }

    #[test]
    fn a_second_save_in_the_same_timestamp_tick_is_a_change() {
        let mut bus = NotifyBus::default();
        answer_prompt("app.a", Choice::All).unwrap();
        bus.handle("app.a", "register", &name(CONTACTS_CHANGE)).unwrap();
        bus.system_changes(Seen { contacts: file(7, 100), ..quiet() });
        bus.system_changes(Seen { contacts: file(7, 120), ..quiet() });
        assert_eq!(event_names(&mut bus).len(), 1, "same mtime, new length");
    }

    #[test]
    fn a_store_file_that_appears_or_goes_is_a_change() {
        let mut bus = NotifyBus::default();
        answer_prompt("app.a", Choice::All).unwrap();
        bus.handle("app.a", "register", &name(CONTACTS_CHANGE)).unwrap();
        bus.system_changes(stores(None, None));
        bus.system_changes(stores(Some(5), None));
        assert_eq!(event_names(&mut bus).len(), 1);
        bus.system_changes(stores(None, None));
        assert_eq!(event_names(&mut bus).len(), 1);
    }

    #[test]
    fn the_contacts_change_needs_a_contacts_grant_to_be_registered() {
        let mut bus = NotifyBus::default();
        assert!(bus.handle("app.a", "register", &name(CONTACTS_CHANGE)).is_err(), "an app with no answer hears nothing");
        answer_prompt("app.a", Choice::Deny).unwrap();
        assert!(bus.handle("app.a", "register", &name(CONTACTS_CHANGE)).is_err(), "a denied app hears nothing");
        answer_prompt("app.b", Choice::All).unwrap();
        assert!(bus.handle("app.b", "register", &name(CONTACTS_CHANGE)).is_ok());
    }

    #[test]
    fn the_calendar_change_needs_the_calendars_permission_to_be_registered() {
        let mut bus = NotifyBus::default();
        assert!(bus.handle("app.a", "register", &name(CALENDAR_CHANGE)).is_err());
    }

    #[test]
    fn an_app_without_contacts_access_is_not_sent_the_change() {
        let mut bus = NotifyBus::default();
        token_past_the_gate(&mut bus, "app.b", CONTACTS_CHANGE);
        bus.system_changes(stores(Some(1), None));
        bus.system_changes(stores(Some(2), None));
        assert!(bus.deliver().is_empty());
    }

    #[test]
    fn an_app_that_loses_contacts_access_is_no_longer_sent_the_change() {
        let mut bus = NotifyBus::default();
        answer_prompt("app.a", Choice::All).unwrap();
        bus.handle("app.a", "register", &name(CONTACTS_CHANGE)).unwrap();
        bus.system_changes(stores(Some(1), None));
        bus.system_changes(stores(Some(2), None));
        change_by_user("app.a", Choice::Deny).unwrap();
        assert!(bus.deliver().is_empty());
    }

    #[test]
    fn apps_cannot_post_the_store_change_names() {
        let mut bus = NotifyBus::default();
        assert!(bus.handle("app.a", "post", &name(CONTACTS_CHANGE)).is_err());
        assert!(bus.handle("app.a", "post", &name(CALENDAR_CHANGE)).is_err());
    }

    /// A fresh data directory for one test, under the crate's `target` folder like the other scratch dirs.
    fn scratch_data_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-notify-bus").join(std::process::id().to_string()).join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_store_files_are_stat_d_at_most_once_a_second_and_a_save_is_seen_after_the_interval() {
        let data = scratch_data_dir("tick");
        // The store is watched through its change log (`contacts_change_log`), not the contacts file itself.
        let contacts = super::contacts_change_log(&data);
        std::fs::create_dir_all(contacts.parent().unwrap()).unwrap();
        let mut bus = NotifyBus::default();
        bus.stat_stores(&data, 0.0);
        assert_eq!(bus.stores.0, None, "no file yet");
        std::fs::write(&contacts, "first\n").unwrap();
        bus.stat_stores(&data, 0.5);
        assert_eq!(bus.stores.0, None, "inside the interval the files are not stat'ed again");
        bus.stat_stores(&data, 1.0);
        let first = bus.stores.0.expect("the file is seen once the interval has passed");
        std::fs::write(&contacts, "second save, longer\n").unwrap();
        bus.stat_stores(&data, 1.5);
        assert_eq!(bus.stores.0, Some(first), "a save inside the interval waits for the next stat");
        bus.stat_stores(&data, 2.0);
        assert_ne!(bus.stores.0, Some(first), "the save is seen after the interval");
    }

    #[test]
    fn the_service_is_registered_under_its_name() {
        assert_eq!(NotifyBus::default().name(), "notify_bus");
    }
}
