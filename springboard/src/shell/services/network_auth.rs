//! The `network_auth` service: the network-authentication agent an app asks when an SMB share or an 802.1X Wi-Fi network
//! needs a login (`NetAuth`). A saved login is handed back at once; otherwise the shell asks the person on a sheet
//! (`shell/network_auth_sheet.rs`: user name, password, and whether to remember them) and the answer goes back to the app.
//!
//! An app may ask only when its manifest lists the `network` permission. Without it the answer is `permission_denied`
//! and no sheet is shown.
//!
//! Methods (params are a JSON object):
//! - `request_login {kind: "share", host, share}` or `{kind: "wifi", ssid}`, optionally `refused: true` (the server or
//!   network rejected the login it was given: the saved login is forgotten and the sheet says it was not accepted) and
//!   `user` (the name that was rejected, for the sheet to start from). Answers `{ok: true, user, password, saved: true}`
//!   when a login is stored, `{ok: false, error: "pending", pending: true}` while the sheet is up (the app calls again),
//!   then once `{ok: true, user, password, saved: false}` with what the person typed, or `{ok: false, error:
//!   "cancelled"}`.
//! - `answer {app, key, cancel}` or `{app, key, user, password, remember}` is the shell's own call, made by the sheet.
//!   An app's id is never empty, so an app cannot make it. The login is saved in the keychain group
//!   `net_auth::GROUP` only when `remember` is set.
//!
//! The queue and the answers live on the shell thread (one thread-local copy), like the accounts service.
use super::{Shell, SystemService};
use crate::keychain::{self, ShareAccess, Store};
use crate::net_auth::{self, Answer, Prompt, Target};
use bifrost_core::json::Json;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

/// Requests waiting behind the sheet on screen. A request past this is cancelled; the app asks again later.
pub const MAX_QUEUED: usize = 8;

/// One request for the sheet: the app that asked and the prompt to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub app: String,
    pub prompt: Prompt,
}

impl Request {
    /// The key requests and answers are matched on.
    pub fn key(&self) -> String {
        self.prompt.target.key()
    }
}

/// What the person decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Login { user: String, password: String },
    Cancelled,
}

#[derive(Default)]
struct AskState {
    queue: VecDeque<Request>,
    shown: Option<Request>,
    answers: BTreeMap<(String, String), Outcome>,
}

thread_local! {
    static ASK: RefCell<AskState> = RefCell::new(AskState::default());
}

/// Queues a sheet. Returns false when the queue is full.
pub fn queue(request: Request) -> bool {
    ASK.with(|a| {
        let mut a = a.borrow_mut();
        if a.queue.len() >= MAX_QUEUED {
            return false;
        }
        a.queue.push_back(request);
        true
    })
}

/// Whether the app already has a sheet queued or on screen for this target.
pub fn is_waiting(app: &str, key: &str) -> bool {
    ASK.with(|a| {
        let a = a.borrow();
        a.queue.iter().chain(a.shown.iter()).any(|r| r.app == app && r.key() == key)
    })
}

/// The next request to draw when no sheet is on screen. The request then counts as shown.
pub fn take_request() -> Option<Request> {
    ASK.with(|a| {
        let mut a = a.borrow_mut();
        if a.shown.is_some() {
            return None;
        }
        let next = a.queue.pop_front()?;
        a.shown = Some(next.clone());
        Some(next)
    })
}

/// The request on screen for this app and target, if it is the one shown.
pub fn shown(app: &str, key: &str) -> Option<Request> {
    ASK.with(|a| a.borrow().shown.clone().filter(|r| r.app == app && r.key() == key))
}

/// Records the outcome and takes the sheet down.
pub fn finish(app: &str, key: &str, outcome: Outcome) {
    ASK.with(|a| {
        let mut a = a.borrow_mut();
        if a.shown.as_ref().is_some_and(|r| r.app == app && r.key() == key) {
            a.shown = None;
        }
        a.answers.insert((app.into(), key.into()), outcome);
    })
}

/// The recorded outcome for the app and target, once.
pub fn take_answer(app: &str, key: &str) -> Option<Outcome> {
    ASK.with(|a| a.borrow_mut().answers.remove(&(app.to_string(), key.to_string())))
}

fn ok_login(user: &str, password: &str, saved: bool) -> Json {
    Json::Obj(vec![
        ("ok".into(), Json::Bool(true)),
        ("user".into(), Json::Str(user.into())),
        ("password".into(), Json::Str(password.into())),
        ("saved".into(), Json::Bool(saved)),
    ])
}

fn failure(error: &str) -> Json {
    Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str(error.into()))])
}

/// The reply while the sheet is up: the app calls again.
fn pending_reply() -> Json {
    Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str("pending".into())), ("pending".into(), Json::Bool(true))])
}

fn outcome_reply(outcome: Outcome) -> Json {
    match outcome {
        Outcome::Login { user, password } => ok_login(&user, &password, false),
        Outcome::Cancelled => failure("cancelled"),
    }
}

/// The target a request names: a share (`host`, `share`) or an 802.1X network (`ssid`).
pub fn target_from(params: &Json) -> Result<Target, String> {
    let text = |name: &str| params.get(name).and_then(Json::as_str).map(str::trim).filter(|s| !s.is_empty());
    match params.get("kind").and_then(Json::as_str) {
        Some("share") => {
            let host = text("host").ok_or("network_auth: a share needs a host")?;
            let share = text("share").ok_or("network_auth: a share needs a share name")?;
            if host.chars().chain(share.chars()).any(char::is_control) {
                return Err("network_auth: the host and share names cannot hold control characters".into());
            }
            Ok(Target::Share { host: host.into(), share: share.into() })
        }
        Some("wifi") => {
            let ssid = params.get("ssid").and_then(Json::as_str).filter(|s| !s.is_empty() && s.len() <= net_auth::MAX_SSID_BYTES).ok_or("network_auth: a network needs a name of 1 to 32 bytes")?;
            Ok(Target::Wifi { ssid: ssid.into() })
        }
        _ => Err("network_auth: kind must be share or wifi".into()),
    }
}

fn open_store(shell: &Shell) -> Result<Store, String> {
    Store::open_group(&shell.data_dir, net_auth::GROUP).map_err(|e| format!("network_auth: the keychain could not be opened: {e}"))
}

/// A request from an app: the stored login, the outcome of an earlier sheet, or a new sheet.
pub fn request_login(shell: &Shell, app_id: &str, params: &Json) -> Result<Json, String> {
    let target = target_from(params)?;
    let key = target.key();
    if let Some(outcome) = take_answer(app_id, &key) {
        return Ok(outcome_reply(outcome));
    }
    if is_waiting(app_id, &key) {
        return Ok(pending_reply());
    }
    let refused = matches!(params.get("refused"), Some(Json::Bool(true)));
    let store = open_store(shell)?;
    let saved = match net_auth::access(&store, &target).map_err(|e| format!("network_auth: the saved login could not be read: {e}"))? {
        ShareAccess::Saved { user, password } if !refused => return Ok(ok_login(&user, &password, true)),
        ShareAccess::Saved { user, .. } => Some(user),
        ShareAccess::Prompt => None,
    };
    let previous = refused.then(|| saved.or_else(|| params.get("user").and_then(Json::as_str).map(str::to_string)).unwrap_or_default());
    if refused {
        // the login that was just refused must not come back from the store
        let _ = net_auth::forget(&store, &target);
    }
    let prompt = net_auth::prompt_for(&target, previous.as_deref());
    if !queue(Request { app: app_id.into(), prompt }) {
        return Ok(failure("cancelled"));
    }
    Ok(pending_reply())
}

/// The sheet's answer: check it, save it when asked, and hand the login to the waiting app.
fn answer(shell: &Shell, params: &Json) -> Result<Json, String> {
    let app = params.get("app").and_then(Json::as_str).ok_or("network_auth: answer needs an app")?;
    let key = params.get("key").and_then(Json::as_str).ok_or("network_auth: answer needs a key")?;
    let request = shown(app, key).ok_or("network_auth: no such question is on screen")?;
    let outcome = if matches!(params.get("cancel"), Some(Json::Bool(true))) {
        Outcome::Cancelled
    } else {
        let field = |name: &str| params.get(name).and_then(Json::as_str).unwrap_or_default().to_string();
        let typed = Answer { user: field("user"), password: field("password"), remember: matches!(params.get("remember"), Some(Json::Bool(true))) };
        match open_store(shell).map(|store| net_auth::complete(&store, &request.prompt, &typed)) {
            Ok(Ok(ShareAccess::Saved { user, password })) => Outcome::Login { user, password },
            Ok(Err(keychain::Error::Param)) | Ok(Ok(ShareAccess::Prompt)) => Outcome::Cancelled,
            // the login could not be saved, but the person typed it: use it this once
            Ok(Err(_)) | Err(_) if net_auth::validate(&typed).is_ok() => Outcome::Login { user: typed.user, password: typed.password },
            _ => Outcome::Cancelled,
        }
    };
    finish(app, key, outcome);
    Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
}

/// The `network_auth` service.
#[derive(Default)]
pub struct NetworkAuth;

impl SystemService for NetworkAuth {
    fn name(&self) -> &'static str {
        "network_auth"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "request_login" if !app_id.is_empty() => {
                if !super::app_has_permission(app_id, "network") {
                    return Ok(failure("permission_denied"));
                }
                request_login(shell, app_id, params)
            }
            // only the shell answers the sheet: an app's id is never empty
            "answer" if app_id.is_empty() => answer(shell, params),
            "request_login" | "answer" => Err("network_auth: this method is not for apps".into()),
            other => Err(format!("network_auth: unknown method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(host: &str, name: &str) -> Json {
        Json::Obj(vec![("kind".into(), Json::Str("share".into())), ("host".into(), Json::Str(host.into())), ("share".into(), Json::Str(name.into()))])
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("netauth-svc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sh(name: &str) -> Shell {
        let mut sh = crate::shell::tests::shell();
        sh.data_dir = scratch(name);
        sh
    }

    #[test]
    fn a_request_names_a_share_or_a_network() {
        assert_eq!(target_from(&share(" nas.local ", "Music")).unwrap(), Target::Share { host: "nas.local".into(), share: "Music".into() });
        let wifi = Json::Obj(vec![("kind".into(), Json::Str("wifi".into())), ("ssid".into(), Json::Str("Campus".into()))]);
        assert_eq!(target_from(&wifi).unwrap(), Target::Wifi { ssid: "Campus".into() });
        assert!(target_from(&share("", "Music")).is_err());
        assert!(target_from(&share("nas", "Mu\nsic")).is_err());
        assert!(target_from(&Json::Obj(vec![("kind".into(), Json::Str("ftp".into()))])).is_err());
        let long = Json::Obj(vec![("kind".into(), Json::Str("wifi".into())), ("ssid".into(), Json::Str("x".repeat(33)))]);
        assert!(target_from(&long).is_err());
    }

    #[test]
    fn an_unknown_login_waits_for_the_sheet_and_the_answer_reaches_the_app_once() {
        let shell = sh("ask");
        let params = share("nas.svc.test", "Docs");
        let key = target_from(&params).unwrap().key();
        let reply = request_login(&shell, "app.files", &params).unwrap();
        assert_eq!(reply.get("pending"), Some(&Json::Bool(true)));
        assert!(is_waiting("app.files", &key));
        // asking again while the sheet is queued does not queue a second one
        request_login(&shell, "app.files", &params).unwrap();
        let shown_request = take_request().expect("the sheet's request");
        assert_eq!(shown_request.prompt.target, Target::Share { host: "nas.svc.test".into(), share: "Docs".into() });
        assert!(!shown_request.prompt.retry);
        assert!(take_request().is_none(), "one sheet at a time");
        let body = Json::Obj(vec![
            ("app".into(), Json::Str("app.files".into())),
            ("key".into(), Json::Str(key.clone())),
            ("user".into(), Json::Str("ola".into())),
            ("password".into(), Json::Str("pw".into())),
            ("remember".into(), Json::Bool(true)),
        ]);
        let mut service = NetworkAuth;
        let mut shell = shell;
        service.call(&mut shell, "", "answer", &body, 0.0).unwrap();
        let got = request_login(&shell, "app.files", &params).unwrap();
        assert_eq!(got.get("user").and_then(Json::as_str), Some("ola"));
        assert_eq!(got.get("password").and_then(Json::as_str), Some("pw"));
        assert_eq!(got.get("saved"), Some(&Json::Bool(false)), "this is the typed login, delivered once");
        // remembered: the next request is answered from the keychain with no sheet
        let again = request_login(&shell, "app.files", &params).unwrap();
        assert_eq!(again.get("saved"), Some(&Json::Bool(true)));
        assert_eq!(again.get("password").and_then(Json::as_str), Some("pw"));
        assert!(take_request().is_none());
    }

    #[test]
    fn a_refused_login_is_forgotten_and_the_sheet_says_so() {
        let mut shell = sh("refused");
        let params = share("nas.refused.test", "Docs");
        let key = target_from(&params).unwrap().key();
        request_login(&shell, "app.files", &params).unwrap();
        take_request().unwrap();
        let body = |remember: bool| {
            Json::Obj(vec![
                ("app".into(), Json::Str("app.files".into())),
                ("key".into(), Json::Str(key.clone())),
                ("user".into(), Json::Str("ola".into())),
                ("password".into(), Json::Str("old".into())),
                ("remember".into(), Json::Bool(remember)),
            ])
        };
        NetworkAuth.call(&mut shell, "", "answer", &body(true), 0.0).unwrap();
        take_answer("app.files", &key).unwrap();
        let mut retry = share("nas.refused.test", "Docs");
        if let Json::Obj(f) = &mut retry {
            f.push(("refused".into(), Json::Bool(true)));
        }
        let reply = request_login(&shell, "app.files", &retry).unwrap();
        assert_eq!(reply.get("pending"), Some(&Json::Bool(true)), "a refused login is not handed back");
        let again = take_request().unwrap();
        assert!(again.prompt.retry && again.prompt.message.contains("not accepted"));
        assert_eq!(again.prompt.user_hint.as_deref(), Some("ola"));
        // and it is gone from the keychain
        let store = Store::open_group(&shell.data_dir, net_auth::GROUP).unwrap();
        assert_eq!(net_auth::access(&store, &again.prompt.target), Ok(ShareAccess::Prompt));
        finish("app.files", &key, Outcome::Cancelled);
        take_answer("app.files", &key);
    }

    #[test]
    fn a_cancel_and_a_bad_answer_give_the_app_no_login_and_save_nothing() {
        let mut shell = sh("cancel");
        let params = share("nas.cancel.test", "Docs");
        let key = target_from(&params).unwrap().key();
        request_login(&shell, "app.files", &params).unwrap();
        take_request().unwrap();
        let cancel = Json::Obj(vec![("app".into(), Json::Str("app.files".into())), ("key".into(), Json::Str(key.clone())), ("cancel".into(), Json::Bool(true))]);
        NetworkAuth.call(&mut shell, "", "answer", &cancel, 0.0).unwrap();
        let reply = request_login(&shell, "app.files", &params).unwrap();
        assert_eq!(reply.get("error").and_then(Json::as_str), Some("cancelled"));
        // an answer for a question that is not on screen is refused
        assert!(NetworkAuth.call(&mut shell, "", "answer", &cancel, 0.0).is_err());
        // a bad answer (empty password) cancels instead of storing
        request_login(&shell, "app.files", &params).unwrap();
        take_request().unwrap();
        let empty = Json::Obj(vec![
            ("app".into(), Json::Str("app.files".into())),
            ("key".into(), Json::Str(key.clone())),
            ("user".into(), Json::Str("ola".into())),
            ("password".into(), Json::Str(String::new())),
            ("remember".into(), Json::Bool(true)),
        ]);
        NetworkAuth.call(&mut shell, "", "answer", &empty, 0.0).unwrap();
        assert_eq!(take_answer("app.files", &key), Some(Outcome::Cancelled));
        let store = Store::open_group(&shell.data_dir, net_auth::GROUP).unwrap();
        assert_eq!(net_auth::access(&store, &target_from(&params).unwrap()), Ok(ShareAccess::Prompt));
    }

    #[test]
    fn only_the_shell_answers_and_only_apps_ask() {
        let mut shell = sh("who");
        let body = Json::Obj(vec![]);
        assert!(NetworkAuth.call(&mut shell, "app.files", "answer", &body, 0.0).is_err());
        assert!(NetworkAuth.call(&mut shell, "", "request_login", &share("h", "s"), 0.0).is_err());
        // no network permission in the manifest: no sheet
        let denied = NetworkAuth.call(&mut shell, "app.nobody", "request_login", &share("h", "s"), 0.0).unwrap();
        assert_eq!(denied.get("error").and_then(Json::as_str), Some("permission_denied"));
        assert!(take_request().is_none());
    }
}
