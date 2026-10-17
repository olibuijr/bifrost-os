//! The `accounts` service: an app asks for access to one account type (`requestAccessToAccounts`), and the person
//! answers Allow or Don't Allow on a sheet the shell draws (`shell/account_access_sheet.rs`). The answer is written to
//! the accounts store file (`bifrost_ui::account_store::set_access`), where the SDK's `AccountStore` reads it.
//!
//! Methods (params are a JSON object):
//! - `request_access {type}` answers `{ok: true, granted}` when the person already allowed the type, or the answer
//!   the sheet recorded (once). While the sheet is up it answers `{ok: false, error: "pending", pending: true}`: the app
//!   calls again. A type the store does not hold is `account_type_invalid`.
//! - `answer {app, type, granted}` is the shell's own call. An app's id is never empty, so an app cannot make it.
//!
//! The queue and the answers live on the shell thread (one thread-local copy), like the privacy service's sheets.
//! A refused type asks again the next time an app asks: the store keeps only the grant, not the refusal.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::account_store;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

/// Requests waiting behind the sheet on screen. A request past this is refused; the app asks again later.
pub const MAX_QUEUED: usize = 8;

/// One request for the sheet: the app, its name for the title, and the type with its description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub app: String,
    pub app_name: String,
    pub account_type: String,
    pub type_description: String,
}

#[derive(Default)]
struct AskState {
    queue: VecDeque<Request>,
    shown: Option<Request>,
    answers: BTreeMap<(String, String), bool>,
}

thread_local! {
    static ASK: RefCell<AskState> = RefCell::new(AskState::default());
}

fn key(app: &str, account_type: &str) -> (String, String) {
    (app.into(), account_type.into())
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

/// Whether the app already has a sheet queued or on screen for this type.
pub fn is_waiting(app: &str, account_type: &str) -> bool {
    ASK.with(|a| {
        let a = a.borrow();
        a.queue.iter().any(|r| r.app == app && r.account_type == account_type)
            || a.shown.as_ref().is_some_and(|r| r.app == app && r.account_type == account_type)
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

/// Records the answer and takes the sheet down.
pub fn finish(app: &str, account_type: &str, granted: bool) {
    ASK.with(|a| {
        let mut a = a.borrow_mut();
        if a.shown.as_ref().is_some_and(|r| r.app == app && r.account_type == account_type) {
            a.shown = None;
        }
        a.answers.insert(key(app, account_type), granted);
    })
}

/// The recorded answer for the app and type, once.
pub fn take_answer(app: &str, account_type: &str) -> Option<bool> {
    ASK.with(|a| a.borrow_mut().answers.remove(&key(app, account_type)))
}

/// The `{ok, granted}` reply for a decided request.
fn granted_reply(granted: bool) -> Json {
    Json::Obj(vec![("ok".into(), Json::Bool(true)), ("granted".into(), Json::Bool(granted))])
}

/// The reply while the sheet is up: the app calls again.
fn pending_reply() -> Json {
    Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str("pending".into())), ("pending".into(), Json::Bool(true))])
}

fn store_file() -> Result<std::path::PathBuf, String> {
    account_store::store_path().ok_or_else(|| "accounts: the store has no home folder".to_string())
}

/// What a request for an account type gets from the store: the type is unknown, already granted, refused before, or
/// it needs the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Invalid,
    Granted,
    Refused,
    Ask,
}

/// The gate for one app's request, decided from the store's contents alone.
pub fn gate(contents: &account_store::Contents, app: &str, account_type: &str) -> Gate {
    match contents.account_type(account_type) {
        None => Gate::Invalid,
        Some(t) if t.access_granted => Gate::Granted,
        Some(_) if contents.refused(app, account_type) => Gate::Refused,
        Some(_) => Gate::Ask,
    }
}

fn request_access(app_id: &str, params: &Json) -> Result<Json, String> {
    let account_type = params.get("type").and_then(Json::as_str).ok_or("accounts: request_access needs a type")?.to_string();
    if let Some(granted) = take_answer(app_id, &account_type) {
        return Ok(granted_reply(granted));
    }
    let contents = account_store::read(&store_file()?).map_err(|e| format!("accounts: the store could not be read: {e}"))?;
    match gate(&contents, app_id, &account_type) {
        Gate::Invalid => Ok(Json::Obj(vec![("ok".into(), Json::Bool(false)), ("error".into(), Json::Str("account_type_invalid".into()))])),
        Gate::Granted => Ok(granted_reply(true)),
        // a refusal stands until the person clears it in Settings > Internet Accounts
        Gate::Refused => Ok(granted_reply(false)),
        Gate::Ask => {
            if !is_waiting(app_id, &account_type) {
                let app_name = crate::shell::apphost::installed().iter().find(|a| a.manifest.id == app_id).map(|a| a.manifest.name.clone()).unwrap_or_else(|| app_id.to_string());
                let type_description = contents.type_description(&account_type).to_string();
                let request = Request { app: app_id.into(), app_name, account_type: account_type.clone(), type_description };
                if !queue(request) {
                    return Ok(granted_reply(false));
                }
            }
            Ok(pending_reply())
        }
    }
}

fn answer(params: &Json) -> Result<Json, String> {
    let app = params.get("app").and_then(Json::as_str).ok_or("accounts: answer needs an app")?;
    let account_type = params.get("type").and_then(Json::as_str).ok_or("accounts: answer needs a type")?;
    let granted = matches!(params.get("granted"), Some(Json::Bool(true)));
    if granted {
        account_store::set_access(&store_file()?, account_type, true).map_err(|e| format!("accounts: the grant could not be saved: {e}"))?;
    } else {
        account_store::add_refusal(&store_file()?, app, account_type).map_err(|e| format!("accounts: the refusal could not be saved: {e}"))?;
    }
    finish(app, account_type, granted);
    Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
}

/// The `accounts` service.
#[derive(Default)]
pub struct AccountsAccess;

impl SystemService for AccountsAccess {
    fn name(&self) -> &'static str {
        "accounts"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "request_access" if !app_id.is_empty() => request_access(app_id, params),
            // only the shell answers the sheet: an app's id is never empty
            "answer" if app_id.is_empty() => answer(params),
            "request_access" | "answer" => Err("accounts: this method is not for apps".into()),
            other => Err(format!("accounts: unknown method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(app: &str, account_type: &str) -> Request {
        Request { app: app.into(), app_name: "Notes".into(), account_type: account_type.into(), type_description: "Mail".into() }
    }

    #[test]
    fn a_request_waits_for_the_sheet_and_its_answer_is_taken_once() {
        assert!(queue(request("app.notes", "bifrost.mail")));
        assert!(is_waiting("app.notes", "bifrost.mail"));
        assert!(!is_waiting("app.notes", "bifrost.cloud"), "another type is another request");
        assert_eq!(take_request().map(|r| r.app_name), Some("Notes".to_string()));
        assert!(take_request().is_none(), "one sheet at a time");
        finish("app.notes", "bifrost.mail", true);
        assert!(!is_waiting("app.notes", "bifrost.mail"));
        assert_eq!(take_answer("app.notes", "bifrost.mail"), Some(true));
        assert_eq!(take_answer("app.notes", "bifrost.mail"), None);
    }

    #[test]
    fn a_refusal_is_an_answer_too() {
        finish("app.refused", "bifrost.carddav", false);
        assert_eq!(take_answer("app.refused", "bifrost.carddav"), Some(false));
    }

    #[test]
    fn the_queue_refuses_past_its_limit() {
        for n in 0..MAX_QUEUED {
            assert!(queue(request(&format!("app.many.{n}"), "bifrost.mail")));
        }
        assert!(!queue(request("app.many.over", "bifrost.mail")));
    }

    #[test]
    fn the_gate_grants_refuses_and_asks_from_the_store_alone() {
        use bifrost_ui::account_store::{Contents, RefusalEntry, TypeEntry};
        let mut c = Contents {
            types: vec![
                TypeEntry { identifier: "bifrost.mail".into(), description: "Mail".into(), access_granted: false },
                TypeEntry { identifier: "bifrost.cloud".into(), description: "BifrOSt Account".into(), access_granted: true },
            ],
            ..Contents::default()
        };
        assert_eq!(gate(&c, "app.notes", "bifrost.unknown"), Gate::Invalid);
        assert_eq!(gate(&c, "app.notes", "bifrost.cloud"), Gate::Granted);
        assert_eq!(gate(&c, "app.notes", "bifrost.mail"), Gate::Ask);
        c.refusals.push(RefusalEntry { app: "app.notes".into(), account_type: "bifrost.mail".into() });
        assert_eq!(gate(&c, "app.notes", "bifrost.mail"), Gate::Refused, "a refusal stands for that app");
        assert_eq!(gate(&c, "app.other", "bifrost.mail"), Gate::Ask, "another app is asked");
    }

    #[test]
    fn the_pending_reply_says_the_app_should_call_again() {
        assert_eq!(pending_reply().get("pending"), Some(&Json::Bool(true)));
        assert_eq!(granted_reply(true).get("granted"), Some(&Json::Bool(true)));
    }
}
