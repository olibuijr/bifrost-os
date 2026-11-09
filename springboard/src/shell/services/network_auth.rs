//! The `network_auth` service: the network-authentication agent an app asks when an SMB share or an 802.1X Wi-Fi network
//! needs a login (`NetAuth`). A saved login is handed back at once to an app the person gave it to; otherwise the shell
//! asks the person on a sheet (`shell/network_auth_sheet.rs`: user name, password, and whether to remember them) and the
//! answer goes back to the app.
//!
//! A saved login belongs to the apps whose sheet the person answered with "remember": the item carries a binding that
//! lists them (`keychain::Store::grant`). Another app with the `network` permission gets a sheet of its own, never the
//! stored login or its user name, and cannot make the agent forget the login.
//!
//! An app may ask only when its manifest lists the `network` permission. Without it the answer is `permission_denied`
//! and no sheet is shown.
//!
//! Methods (params are a JSON object):
//! - `request_login {kind: "share", host, share}` or `{kind: "wifi", ssid}`, optionally `refused: true` (the server or
//!   network rejected the login it was given: the saved login is forgotten when this app was given it, and the sheet
//!   says it was not accepted) and `user` (the name that was rejected, for the sheet to start from). Answers
//!   `{ok: true, user, password, saved: true}` when a login is stored and this app was given it,
//!   `{ok: false, error: "pending", pending: true}` while the sheet is up (the app calls again),
//!   then once `{ok: true, user, password, saved: false}` with what the person typed, or `{ok: false, error:
//!   "cancelled"}`.
//! - `answer {app, key, cancel}` or `{app, key, user, password, remember}` is the shell's own call, made by the sheet.
//!   An app's id is never empty, so an app cannot make it. The login is saved in the keychain group
//!   `net_auth::GROUP` only when `remember` is set, and the app that asked is then listed on it. A login that differs
//!   from the stored one replaces it and drops the apps that were given the old one.
//!
//! The queue and the answers live on the shell thread (one thread-local copy), like the accounts service.
use super::{Shell, SystemService};
use crate::keychain::{self, ShareAccess, Store};
use crate::net_auth::{self, Answer, Prompt, Target};
use bifrost_core::json::Json;
use bifrost_ui::credential_acl::Binding;
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

/// The name the agent's items are bound under. It has no dot, so it is never an app id.
const AGENT: &str = "network_auth";

/// The keychain item that holds the login of `target`.
fn item_of(target: &Target) -> (&'static str, String) {
    match target {
        Target::Share { host, share } => (keychain::SHARE_LOGIN_SERVICE, keychain::share_login_account(host, share)),
        Target::Wifi { ssid } => (net_auth::WIFI_LOGIN_SERVICE, ssid.clone()),
    }
}

/// Whether the person gave `app` the stored login of `target`: the item's binding lists the app. A login with no
/// binding (or one that cannot be read) was given to no app.
fn handed_to(store: &Store, target: &Target, app: &str) -> bool {
    let (service, account) = item_of(target);
    matches!(store.binding(service, &account), Ok(Some(b)) if b.allowed().iter().any(|a| a == app))
}

/// Lists `app` on the stored login of `target`, after the person answered its sheet with "remember". With `replaced`
/// (the login is not the one that was stored before) the apps listed until now are dropped first: they were given
/// the old login, not this one. A failure lists nobody, so the app is asked again next time.
fn hand_to(store: &Store, target: &Target, app: &str, replaced: bool) {
    let (service, account) = item_of(target);
    if replaced {
        if let Ok(fresh) = Binding::new(AGENT, service, &account, None) {
            let _ = store.bind(service, &account, &fresh);
        }
    }
    let _ = store.grant(AGENT, service, &account, app);
}

/// A request from an app: the stored login when the person gave it to this app, the outcome of an earlier sheet, or a
/// new sheet.
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
    let access = net_auth::access(&store, &target).map_err(|e| format!("network_auth: the saved login could not be read: {e}"))?;
    // a stored login is this app's to use, or to report as refused, only when the person gave it to this app
    let handed = matches!(access, ShareAccess::Saved { .. }) && handed_to(&store, &target, app_id);
    let saved = match access {
        ShareAccess::Saved { user, password } if handed && !refused => return Ok(ok_login(&user, &password, true)),
        ShareAccess::Saved { user, .. } if handed => Some(user),
        // no login, or one this app was never given: not even its user name leaves the store
        _ => None,
    };
    let previous = refused.then(|| saved.or_else(|| params.get("user").and_then(Json::as_str).map(str::to_string)).unwrap_or_default());
    if refused && handed {
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
        let store = open_store(shell);
        let before = store.as_ref().ok().and_then(|s| net_auth::access(s, &request.prompt.target).ok());
        let done = store.as_ref().map(|s| net_auth::complete(s, &request.prompt, &typed));
        if let (true, Ok(s), Ok(Ok(now))) = (typed.remember, &store, &done) {
            // the login was saved for this app: later requests from it are answered without a sheet
            hand_to(s, &request.prompt.target, app, before.as_ref() != Some(now));
        }
        match done {
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

    fn typed(app: &str, key: &str, user: &str, password: &str) -> Json {
        Json::Obj(vec![
            ("app".into(), Json::Str(app.into())),
            ("key".into(), Json::Str(key.into())),
            ("user".into(), Json::Str(user.into())),
            ("password".into(), Json::Str(password.into())),
            ("remember".into(), Json::Bool(true)),
        ])
    }

    fn refusing(params: &Json) -> Json {
        let mut p = params.clone();
        if let Json::Obj(f) = &mut p {
            f.push(("refused".into(), Json::Bool(true)));
        }
        p
    }

    #[test]
    fn a_saved_login_goes_only_to_the_app_the_person_gave_it_to() {
        let mut shell = sh("scoped");
        let params = share("nas.scoped.test", "Docs");
        let target = target_from(&params).unwrap();
        let key = target.key();
        request_login(&shell, "app.files", &params).unwrap();
        take_request().unwrap();
        NetworkAuth.call(&mut shell, "", "answer", &typed("app.files", &key, "ola", "pw"), 0.0).unwrap();
        take_answer("app.files", &key).unwrap();
        // another app asks for the same share: a sheet of its own, and nothing from the store
        let other = request_login(&shell, "app.other", &params).unwrap();
        assert_eq!(other.get("pending"), Some(&Json::Bool(true)));
        assert!(other.get("password").is_none() && other.get("user").is_none());
        let asked = take_request().expect("the other app is asked");
        assert_eq!(asked.app, "app.other");
        assert!(!asked.prompt.retry && asked.prompt.user_hint.is_none());
        finish("app.other", &key, Outcome::Cancelled);
        take_answer("app.other", &key);
        // it cannot make the agent forget the login either, nor learn the saved user name from the retry sheet
        let reply = request_login(&shell, "app.other", &refusing(&params)).unwrap();
        assert_eq!(reply.get("pending"), Some(&Json::Bool(true)));
        let retry = take_request().unwrap();
        assert_eq!(retry.prompt.user_hint.as_deref(), Some(""), "the saved name is not shown to an app that never had it");
        finish("app.other", &key, Outcome::Cancelled);
        take_answer("app.other", &key);
        let store = Store::open_group(&shell.data_dir, net_auth::GROUP).unwrap();
        assert_eq!(net_auth::access(&store, &target), Ok(ShareAccess::Saved { user: "ola".into(), password: "pw".into() }));
        // the app that was given the login still gets it with no sheet
        let again = request_login(&shell, "app.files", &params).unwrap();
        assert_eq!(again.get("saved"), Some(&Json::Bool(true)));
        assert_eq!(again.get("password").and_then(Json::as_str), Some("pw"));
        assert!(take_request().is_none());
    }

    #[test]
    fn answering_another_apps_sheet_shares_the_same_login_and_a_new_login_drops_the_old_apps() {
        let mut shell = sh("shared");
        let params = share("nas.shared.test", "Docs");
        let key = target_from(&params).unwrap().key();
        let ask = |shell: &mut Shell, app: &str, password: &str| {
            request_login(shell, app, &params).unwrap();
            take_request().unwrap();
            NetworkAuth.call(shell, "", "answer", &typed(app, &key, "ola", password), 0.0).unwrap();
            take_answer(app, &key).unwrap();
        };
        let saved = |shell: &Shell, app: &str| request_login(shell, app, &params).unwrap().get("saved") == Some(&Json::Bool(true));
        ask(&mut shell, "app.files", "pw");
        // the person types the same login for a second app: both are answered from the store
        ask(&mut shell, "app.other", "pw");
        assert!(saved(&shell, "app.files") && saved(&shell, "app.other"));
        // a different login typed for a third app is that app's alone
        ask(&mut shell, "app.third", "changed");
        assert!(saved(&shell, "app.third"));
        for app in ["app.files", "app.other"] {
            let reply = request_login(&shell, app, &params).unwrap();
            assert_eq!(reply.get("pending"), Some(&Json::Bool(true)), "{app} was given the old login only");
            take_request().unwrap();
            finish(app, &key, Outcome::Cancelled);
            take_answer(app, &key);
        }
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
