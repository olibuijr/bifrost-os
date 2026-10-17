//! The `app_firewall` service: the per-app incoming connection choices (allow or block, with the ports an app listens
//! on) and the nftables rules that carry them out. The model is `bifrost_core::app_firewall` (policy file, ruleset
//! text); this service keeps the file, builds the ruleset and loads it with `nft -f -`. The shell runs as root until
//! the privilege split, so `nft` runs directly; as any other user the load fails and the status says so.
//!
//! The rules load once at the first tick (from the saved file, so the choices outlive a restart) and again after each
//! change. The load runs on a thread; `status` reports `applying`, `applied` or `failed` with the reason. A ruleset
//! with a conflict (a port allowed by one app and blocked by another) is refused and the loaded rules stay.
//!
//! Calls (JSON params):
//! - `policies {}` -> `{policies: [{app_id, policy, ports: ["tcp/80"]}]}`
//! - `decision {app_id}` -> `{decision}`: `allow`, `block`, or `ask` when nothing is remembered for the app
//! - `set {app_id, policy, ports?}` -> `{ok}`: remembers the choice (`allow` or `block`; the ports default to the ones
//!   already known for the app) and loads the rules
//! - `remove {app_id}` -> `{removed}`: forgets the choice and loads the rules
//! - `status {}` -> `{state, error?}`
//!
//! The choices are saved in `app-firewall.conf` under the data directory, in the policy file format
//! (`bifrost_core::app_firewall::format_policies`).
use super::{Event, Shell, SystemService};
use bifrost_core::app_firewall::{self, AppPolicy, Policy, Port, Proto};
use bifrost_core::json::Json;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The name the service is registered under.
pub const SERVICE: &str = "app_firewall";
/// The saved choices, in the data directory.
pub const FILE: &str = "app-firewall.conf";

/// Where a load of the rules stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotLoaded,
    Applying,
    Applied,
    Failed(String),
}

/// Loads a ruleset into the kernel.
pub trait Applier: Send + Sync {
    fn apply(&self, ruleset: &str) -> Result<(), String>;
}

/// The real loader: `nft -f -` with the ruleset on its standard input.
struct Nft;

impl Applier for Nft {
    fn apply(&self, ruleset: &str) -> Result<(), String> {
        let mut child = Command::new("nft")
            .args(["-f", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("app firewall: nft: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(ruleset.as_bytes()).map_err(|e| format!("app firewall: nft: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("app firewall: nft: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("app firewall: nft refused the rules: {}", String::from_utf8_lossy(&out.stderr).trim()))
        }
    }
}

pub struct AppFirewall {
    store: PathBuf,
    policies: Vec<AppPolicy>,
    applier: Arc<dyn Applier>,
    status: Arc<Mutex<Status>>,
    worker: Option<JoinHandle<()>>,
    loaded_at_start: bool,
}

impl AppFirewall {
    /// The service with its choices saved under `data_dir`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with(data_dir.join(FILE), Arc::new(Nft))
    }

    /// The service with its choices in `store` and `applier` loading the rules.
    pub(crate) fn with_applier(store: PathBuf, applier: Arc<dyn Applier>) -> Self {
        Self::with(store, applier)
    }

    fn with(store: PathBuf, applier: Arc<dyn Applier>) -> Self {
        let policies = std::fs::read_to_string(&store).ok().and_then(|t| app_firewall::parse_policies(&t).ok()).unwrap_or_default();
        AppFirewall { store, policies, applier, status: Arc::new(Mutex::new(Status::NotLoaded)), worker: None, loaded_at_start: false }
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or(Status::NotLoaded)
    }

    /// Builds the ruleset for the choices and loads it on a thread. A conflict or a bad id is `Failed` at once.
    pub fn apply(&mut self) {
        let ruleset = match app_firewall::ruleset(&self.policies) {
            Ok(r) => r,
            Err(e) => {
                self.set_status(Status::Failed(format!("app firewall: {e:?}")));
                return;
            }
        };
        self.set_status(Status::Applying);
        if let Some(previous) = self.worker.take() {
            let _ = previous.join();
        }
        let (applier, status) = (Arc::clone(&self.applier), Arc::clone(&self.status));
        self.worker = Some(std::thread::spawn(move || {
            let result = applier.apply(&ruleset);
            if let Ok(mut s) = status.lock() {
                *s = match result {
                    Ok(()) => Status::Applied,
                    Err(e) => Status::Failed(e),
                };
            }
        }));
    }

    /// Waits for a load in progress (tests; the shell never waits).
    pub fn wait(&mut self) {
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }

    fn set_status(&self, s: Status) {
        if let Ok(mut st) = self.status.lock() {
            *st = s;
        }
    }

    fn save(&self) -> Result<(), String> {
        if let Some(dir) = self.store.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("app firewall: {e}"))?;
        }
        std::fs::write(&self.store, app_firewall::format_policies(&self.policies)).map_err(|e| format!("app firewall: {e}"))
    }

    /// What is remembered for an app: allow, block, or ask.
    pub fn decision(&self, app_id: &str) -> &'static str {
        match self.policies.iter().find(|p| p.app_id == app_id).map(|p| p.policy) {
            Some(Policy::Allow) => "allow",
            Some(Policy::Block) => "block",
            None => "ask",
        }
    }

    /// Answers one call.
    pub fn handle(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        let app_id = || params.str_field("app_id").filter(|a| app_firewall::valid_app_id(a)).ok_or_else(|| "app firewall: a valid app_id is needed".to_string());
        match method {
            "policies" => {
                let list = self.policies.iter().map(|p| {
                    let ports = p.ports.iter().map(|port| Json::Str(port_text(*port))).collect();
                    obj(vec![
                        ("app_id", Json::Str(p.app_id.clone())),
                        ("policy", Json::Str(if p.policy == Policy::Allow { "allow" } else { "block" }.to_string())),
                        ("ports", Json::Arr(ports)),
                    ])
                });
                Ok(obj(vec![("policies", Json::Arr(list.collect()))]))
            }
            "decision" => Ok(obj(vec![("decision", Json::Str(self.decision(&app_id()?).to_string()))])),
            "set" => {
                let id = app_id()?;
                let policy = match params.str_field("policy").as_deref() {
                    Some("allow") => Policy::Allow,
                    Some("block") => Policy::Block,
                    _ => return Err("app firewall: policy must be allow or block".into()),
                };
                let known = self.policies.iter().find(|p| p.app_id == id).map(|p| p.ports.clone()).unwrap_or_default();
                let ports = match params.get("ports").and_then(Json::as_array) {
                    Some(items) => items.iter().map(|i| i.as_str().ok_or_else(|| "app firewall: ports are strings like tcp/80".to_string()).and_then(parse_port)).collect::<Result<Vec<_>, _>>()?,
                    None => known,
                };
                let candidate = AppPolicy { app_id: id.clone(), policy, ports };
                // the whole set must still make a ruleset before the choice is kept
                let mut next: Vec<AppPolicy> = self.policies.iter().filter(|p| p.app_id != id).cloned().collect();
                next.push(candidate);
                app_firewall::ruleset(&next).map_err(|e| format!("app firewall: {e:?}"))?;
                self.policies = next;
                self.save()?;
                self.apply();
                Ok(obj(vec![("ok", Json::Bool(true))]))
            }
            "remove" => {
                let id = app_id()?;
                let before = self.policies.len();
                self.policies.retain(|p| p.app_id != id);
                let removed = self.policies.len() != before;
                if removed {
                    self.save()?;
                    self.apply();
                }
                Ok(obj(vec![("removed", Json::Bool(removed))]))
            }
            "status" => {
                let (state, error) = match self.status() {
                    Status::NotLoaded => ("not_loaded", None),
                    Status::Applying => ("applying", None),
                    Status::Applied => ("applied", None),
                    Status::Failed(e) => ("failed", Some(e)),
                };
                let mut fields = vec![("state", Json::Str(state.to_string()))];
                if let Some(e) = error {
                    fields.push(("error", Json::Str(e)));
                }
                Ok(obj(fields))
            }
            _ => Err(format!("app_firewall has no method {method}")),
        }
    }
}

impl SystemService for AppFirewall {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(method, params)
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        if !std::mem::replace(&mut self.loaded_at_start, true) && !self.policies.is_empty() {
            self.apply();
        }
        Vec::new()
    }
}

fn port_text(p: Port) -> String {
    format!("{}/{}", if p.proto == Proto::Tcp { "tcp" } else { "udp" }, p.number)
}

fn parse_port(text: &str) -> Result<Port, String> {
    let (proto, number) = text.split_once('/').ok_or_else(|| format!("app firewall: port {text} needs tcp/ or udp/"))?;
    let proto = match proto {
        "tcp" => Proto::Tcp,
        "udp" => Proto::Udp,
        _ => return Err(format!("app firewall: unknown protocol {proto}")),
    };
    let number: u16 = number.parse().ok().filter(|n| *n != 0).ok_or_else(|| format!("app firewall: port number {number} is not 1 to 65535"))?;
    Ok(Port { proto, number })
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Record(Mutex<Vec<String>>, bool);
    impl Applier for Record {
        fn apply(&self, ruleset: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(ruleset.to_string());
            if self.1 { Err("no root".to_string()) } else { Ok(()) }
        }
    }

    fn service(tag: &str, fail: bool) -> (AppFirewall, Arc<Record>) {
        let dir = std::env::temp_dir().join(format!("app-firewall-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = Arc::new(Record(Mutex::new(Vec::new()), fail));
        (AppFirewall::with(dir.join(FILE), rec.clone()), rec)
    }

    fn call(s: &mut AppFirewall, method: &str, fields: Vec<(&str, Json)>) -> Result<Json, String> {
        s.handle(method, &obj(fields))
    }

    fn set(id: &str, policy: &str, ports: &[&str]) -> Vec<(&'static str, Json)> {
        vec![("app_id", Json::Str(id.into())), ("policy", Json::Str(policy.into())), ("ports", Json::Arr(ports.iter().map(|p| Json::Str((*p).into())).collect()))]
    }

    #[test]
    fn an_app_with_no_choice_is_asked_and_a_choice_is_remembered() {
        let (mut s, _) = service("ask", false);
        let ask = call(&mut s, "decision", vec![("app_id", Json::Str("is.example.chat".into()))]).unwrap();
        assert_eq!(ask.str_field("decision").as_deref(), Some("ask"));
        call(&mut s, "set", set("is.example.chat", "block", &["tcp/8080"])).unwrap();
        s.wait();
        let d = call(&mut s, "decision", vec![("app_id", Json::Str("is.example.chat".into()))]).unwrap();
        assert_eq!(d.str_field("decision").as_deref(), Some("block"));
    }

    #[test]
    fn a_choice_loads_the_ruleset_and_a_blocked_port_is_dropped() {
        let (mut s, rec) = service("load", false);
        call(&mut s, "set", set("is.example.chat", "block", &["tcp/8080", "udp/5353"])).unwrap();
        s.wait();
        assert_eq!(s.status(), Status::Applied);
        let loaded = rec.0.lock().unwrap();
        let rules = loaded.last().unwrap();
        assert!(rules.contains("tcp dport 8080 drop comment \"is.example.chat\""));
        assert!(rules.contains("udp dport 5353 drop"));
    }

    #[test]
    fn a_conflicting_choice_is_refused_and_changes_nothing() {
        let (mut s, rec) = service("conflict", false);
        call(&mut s, "set", set("a.one", "allow", &["tcp/80"])).unwrap();
        s.wait();
        let loads = rec.0.lock().unwrap().len();
        let err = call(&mut s, "set", set("b.two", "block", &["tcp/80"])).unwrap_err();
        assert!(err.contains("Conflict"), "{err}");
        assert_eq!(rec.0.lock().unwrap().len(), loads, "no new load");
        assert_eq!(call(&mut s, "decision", vec![("app_id", Json::Str("b.two".into()))]).unwrap().str_field("decision").as_deref(), Some("ask"));
    }

    #[test]
    fn bad_input_is_refused() {
        let (mut s, _) = service("bad", false);
        assert!(call(&mut s, "set", set("Bad App", "block", &[])).is_err(), "an app id outside a-z 0-9 . - _");
        assert!(call(&mut s, "set", set("a.one", "maybe", &[])).is_err());
        assert!(call(&mut s, "set", set("a.one", "block", &["tcp/0"])).is_err());
        assert!(call(&mut s, "set", set("a.one", "block", &["80"])).is_err());
        assert!(call(&mut s, "nope", vec![]).is_err());
    }

    #[test]
    fn a_failed_load_is_reported_and_the_choices_survive_a_restart() {
        let (mut s, _) = service("fail", true);
        call(&mut s, "set", set("a.one", "block", &["tcp/9"])).unwrap();
        s.wait();
        let st = call(&mut s, "status", vec![]).unwrap();
        assert_eq!(st.str_field("state").as_deref(), Some("failed"));
        assert_eq!(st.str_field("error").as_deref(), Some("no root"));
        let again = AppFirewall::with(s.store.clone(), s.applier.clone());
        assert_eq!(again.decision("a.one"), "block");
        let removed = call(&mut s, "remove", vec![("app_id", Json::Str("a.one".into()))]).unwrap();
        assert_eq!(removed.get("removed"), Some(&Json::Bool(true)));
        s.wait();
        assert_eq!(s.decision("a.one"), "ask");
    }
}
