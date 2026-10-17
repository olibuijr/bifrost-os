//! Settings > BifrOSt Account: the signed-in account of this computer, at the top of Settings.
//!
//! Signed out, the page offers Sign In. Its sheet asks for the email address, then the password (two one-field prompts,
//! [`super::forms`]). The password goes to the cloud's sign-in route and is kept nowhere: the answer's `bc_session`
//! cookie is stored as the session (`cloud_session`, the `is.bifrost.account` keychain group), with its lifetime from the
//! cookie's `Max-Age`. A refused sign-in reopens the password prompt with the cloud's reason in its message line.
//!
//! Signed in, the page shows the name and email (`GET /me`), the account's devices (`GET /devices`), and Sign Out. Sign
//! Out deletes the session from the keychain and asks the cloud to end it (`POST /signout`).
//!
//! Accounts are made with an invitation (`POST /signup` takes an invite), so the page offers no sign-up. Every network
//! call runs on a worker thread (the Settings loads, `settings_load`); the functions here take the keychain root and
//! return the answer, so the pure parts are tested without a socket.

use super::forms::{Prompt, Purpose};
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use crate::cloud_account::{checked_token, cookie_expiry, endpoint, failure, normalize_email, parse_sign_in, parse_sign_out, CloudAccount, CloudError, SESSION_COOKIE};
use crate::cloud_session::{self as session, GroupVault, Session, Vault, SERVER};
use crate::deildu::http::{Client, Jar, Response, Tls};
use bifrost_core::json::{self, Json};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The largest answer read from the cloud, in bytes.
const LIMIT: usize = 64 * 1024;

const EMAIL_MESSAGE: &str = "Enter the email address of your BifrOSt Account.";
const PASSWORD_MESSAGE: &str = "Enter your password. It is sent to BifrOSt Cloud and is not kept on this computer.";

/// A tap on the account page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Opens the sign-in sheet.
    SignIn,
    /// Ends the session on this computer and in the cloud.
    SignOut,
}

/// What the account page shows: whether a session is stored, and the account behind it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub signed_in: bool,
    pub name: String,
    pub email: String,
    pub devices: Vec<DeviceRow>,
    /// Why the account or its devices could not be read (empty when they were).
    pub error: String,
}

/// One device of the account, as the cloud lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    pub name: String,
    pub model: String,
    /// Unix seconds when the device last reported in.
    pub last_seen: i64,
}

/// A sign-in or sign-out that finished on its worker thread, for `Settings` to act on.
#[derive(Clone, Debug, Default)]
pub enum Landed {
    #[default]
    Idle,
    /// The sign-in for `email` answered: the account, or why it was refused.
    SignedIn { email: String, result: Result<CloudAccount, String> },
    /// The session was removed here; the result says whether the cloud ended it too.
    SignedOut(Result<(), String>),
}

/// What the next step of the sign-in sheet is.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Show this prompt next.
    Ask(Prompt),
    /// Both answers are in: sign in with them on a worker thread.
    SignIn { email: String, password: String },
}

/// What the page is built from. `view` is `None` until the first read lands.
pub struct PageState<'a> {
    pub view: Option<&'a View>,
    /// Why the last sign-in or sign-out did not happen (empty when nothing failed).
    pub note: &'a str,
    /// A sign-in or sign-out is running.
    pub busy: bool,
}

/// The Settings page.
pub fn spec(s: &PageState) -> Spec {
    let mut groups = Vec::new();
    let view_error = s.view.map_or("", |v| v.error.as_str());
    for text in [s.note, view_error] {
        if !text.is_empty() {
            groups.push(Group::new(vec![Item::new(text).tint(Tint::Red)]));
        }
    }
    if s.busy {
        groups.push(Group::new(vec![Item::new("Working…").tint(Tint::Dim)]));
    }
    match s.view {
        None => groups.push(Group::new(vec![Item::new("Loading").tint(Tint::Dim)])),
        Some(v) if !v.signed_in => {
            let sign_in = Item::new("Sign In").tint(Tint::Blue).action(Action::Account(Act::SignIn));
            groups.push(Group::new(vec![sign_in]).footer("Sign in with the email and password of your BifrOSt Account. Accounts are made with an invitation."));
        }
        Some(v) => {
            groups.push(Group::new(vec![Item::new("Name").value(v.name.as_str()), Item::new("Email").value(v.email.as_str())]));
            let devices: Vec<Item> = if v.devices.is_empty() {
                vec![Item::new("No Devices").tint(Tint::Dim)]
            } else {
                v.devices.iter().map(|d| Item::new(d.name.as_str()).value(d.model.as_str())).collect()
            };
            groups.push(Group::new(devices).header("DEVICES").footer("The devices signed in to this account."));
            groups.push(Group::new(vec![Item::new("Sign Out").tint(Tint::Red).action(Action::Account(Act::SignOut))]));
        }
    }
    Spec { id: PageId::Account, title: "BifrOSt Account".into(), large: false, groups }
}

/// The value of the Settings root row: the account's name, `Sign In` when signed out, blank until the first read lands.
pub fn label(view: Option<&View>) -> String {
    match view {
        None => String::new(),
        Some(v) if !v.signed_in => "Sign In".into(),
        Some(v) if !v.name.is_empty() => v.name.clone(),
        Some(v) if !v.email.is_empty() => v.email.clone(),
        Some(_) => "Signed In".into(),
    }
}

/// Whether `purpose` is one of the two sign-in prompts.
pub fn owns(purpose: &Purpose) -> bool {
    matches!(purpose, Purpose::AccountEmail | Purpose::AccountPassword { .. })
}

/// The prompt for the email address, with `error` as its message when there is one.
pub fn email_prompt(error: &str) -> Prompt {
    Prompt::account_email(if error.is_empty() { EMAIL_MESSAGE } else { error })
}

/// The prompt for the password of `email`, with `error` (the cloud's reason) as its message when there is one.
pub fn password_prompt(email: &str, error: &str) -> Prompt {
    Prompt::account_password(email, if error.is_empty() { PASSWORD_MESSAGE } else { error })
}

/// The step after one accepted sign-in prompt.
pub fn next_prompt(purpose: &Purpose, text: &str) -> Step {
    match purpose {
        Purpose::AccountEmail => match normalize_email(text) {
            Ok(email) => Step::Ask(password_prompt(&email, "")),
            Err(e) => Step::Ask(email_prompt(&e.to_string())),
        },
        Purpose::AccountPassword { email } => Step::SignIn { email: email.clone(), password: text.to_string() },
        _ => Step::Ask(email_prompt("")),
    }
}

/// The HTTP client of the cloud calls: TLS only, no cookies kept (the session is in the keychain, not the jar).
fn client() -> Client {
    Client::new(Arc::new(Tls::new()), Jar::default()).require_https()
}

fn send(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Response, CloudError> {
    client().request_with_headers(method, url, headers, body, LIMIT).map_err(|e| CloudError::Transport(e.to_string()))
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Stores the answer to a sign-in request as the session and returns the account. The password is not in here.
/// A refused answer, or a session that cannot be stored, changes nothing and is the error.
pub fn save_answer(vault: &dyn Vault, status: u16, set_cookie: Option<&str>, body: &str, now: i64) -> Result<CloudAccount, String> {
    let signed = parse_sign_in(status, set_cookie, body).map_err(|e| e.to_string())?;
    let expires = set_cookie.and_then(|c| cookie_expiry(c, now));
    let stored = Session::new(&signed.session_token, &signed.account.id, SERVER, expires).ok_or_else(|| CloudError::InvalidSession.to_string())?;
    session::store(vault, &stored).map_err(|e| format!("The session could not be saved in the keychain: {e}"))?;
    Ok(signed.account)
}

/// Signs in with the email and password, and stores the session under `root` (the shell data directory).
pub fn sign_in(root: &Path, email: &str, password: &str) -> Result<CloudAccount, String> {
    let email = normalize_email(email).map_err(|e| e.to_string())?;
    if password.is_empty() {
        return Err(CloudError::EmptyPassword.to_string());
    }
    let url = endpoint(SERVER, "signin").map_err(|e| e.to_string())?;
    let body = crate::cloud_account::sign_in_body(&email, password);
    let reply = send("POST", &url, &[("Content-Type", "application/json"), ("Accept", "application/json")], Some(&body)).map_err(|e| e.to_string())?;
    let set_cookie = reply.header("Set-Cookie").map(str::to_string);
    save_answer(&GroupVault::new(root), reply.status, set_cookie.as_deref(), &reply.text(), now_unix())
}

/// Removes the session from the keychain, then asks the cloud to end it. A session that is not stored is not an error.
pub fn sign_out(root: &Path) -> Result<(), String> {
    let vault = GroupVault::new(root);
    let stored = session::load(&vault).map_err(|e| e.to_string())?;
    session::sign_out(&vault).map_err(|e| format!("The session could not be removed from the keychain: {e}"))?;
    let Some(stored) = stored else { return Ok(()) };
    end_in_cloud(&stored).map_err(|e| format!("Signed out on this computer. The cloud did not end the session: {e}"))
}

fn end_in_cloud(stored: &Session) -> Result<(), CloudError> {
    let url = endpoint(&stored.server, "signout")?;
    let cookie = format!("{SESSION_COOKIE}={}", checked_token(&stored.token)?);
    let reply = send("POST", &url, &[("Cookie", cookie.as_str())], None)?;
    parse_sign_out(reply.status, &reply.text())
}

/// Reads the page's account: the stored session's name and email, and its devices. A session the cloud no longer
/// accepts is removed here, and the page says so.
pub fn load(root: &Path) -> View {
    let vault = GroupVault::new(root);
    let stored = match session::load(&vault) {
        Ok(Some(stored)) => stored,
        _ => return View::default(),
    };
    match fetch(&stored) {
        Ok(view) => view,
        Err(CloudError::Rejected { status: 401, .. }) => {
            let _ = session::sign_out(&vault);
            View { error: "The session has ended. Sign in again.".into(), ..View::default() }
        }
        Err(e) => View { signed_in: true, error: e.to_string(), ..View::default() },
    }
}

fn fetch(stored: &Session) -> Result<View, CloudError> {
    let auth = format!("Bearer {}", checked_token(&stored.token)?);
    let headers = [("Authorization", auth.as_str()), ("Accept", "application/json")];
    let me = send("GET", &endpoint(&stored.server, "me")?, &headers, None)?;
    if me.status != 200 {
        return Err(failure(me.status, &me.text()));
    }
    let account = json::parse(&me.text()).ok().and_then(|j| j.get("account").cloned()).ok_or(CloudError::MalformedAnswer("no account".into()))?;
    let devices = send("GET", &endpoint(&stored.server, "devices")?, &headers, None)?;
    if devices.status != 200 {
        return Err(failure(devices.status, &devices.text()));
    }
    Ok(View {
        signed_in: true,
        name: account.str_field("name").unwrap_or_default(),
        email: account.str_field("email").unwrap_or_default(),
        devices: device_rows(&devices.text()),
        error: String::new(),
    })
}

/// The devices in a `GET /devices` answer. A malformed row is skipped.
pub fn device_rows(body: &str) -> Vec<DeviceRow> {
    let Ok(parsed) = json::parse(body) else { return Vec::new() };
    let Some(list) = parsed.get("devices").and_then(Json::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|d| {
            Some(DeviceRow {
                name: d.str_field("name")?,
                model: d.str_field("model").unwrap_or_default(),
                last_seen: d.get("last_seen").and_then(Json::number).unwrap_or(0.0) as i64,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        item: RefCell<Option<Vec<u8>>>,
    }

    impl Vault for Fake {
        fn read(&self) -> Result<Option<Vec<u8>>, crate::keychain::Error> {
            Ok(self.item.borrow().clone())
        }
        fn write(&self, value: &[u8]) -> Result<(), crate::keychain::Error> {
            *self.item.borrow_mut() = Some(value.to_vec());
            Ok(())
        }
        fn remove(&self) -> Result<(), crate::keychain::Error> {
            self.item.borrow_mut().take();
            Ok(())
        }
    }

    const ACCOUNT: &str = r#"{"account":{"id":"acc1","email":"ada@example.com","name":"Ada","avatar":"","created_at":1700000000,"is_admin":false}}"#;

    fn signed_in() -> View {
        View { signed_in: true, name: "Ada".into(), email: "ada@example.com".into(), devices: Vec::new(), error: String::new() }
    }

    fn labels(s: &Spec) -> Vec<String> {
        s.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.clone())).collect()
    }

    #[test]
    fn signed_out_offers_sign_in_and_no_sign_out() {
        let view = View::default();
        let s = spec(&PageState { view: Some(&view), note: "", busy: false });
        assert_eq!(s.id, PageId::Account);
        assert_eq!(s.groups[0].items[0].action, Action::Account(Act::SignIn));
        assert!(!labels(&s).iter().any(|l| l == "Sign Out"));
    }

    #[test]
    fn signed_in_shows_the_account_its_devices_and_sign_out() {
        let mut view = signed_in();
        view.devices = device_rows(r#"{"devices":[{"name":"Midget","model":"M900","last_seen":1700000000}],"this_device":null}"#);
        let s = spec(&PageState { view: Some(&view), note: "", busy: false });
        let all = labels(&s);
        assert!(!all.contains(&"Ada".to_string()), "the name is a value, not a title");
        assert!(all.contains(&"Midget".to_string()));
        let last = s.groups.last().unwrap();
        assert_eq!(last.items[0].action, Action::Account(Act::SignOut));
    }

    #[test]
    fn a_note_or_an_error_shows_first_and_loading_waits_for_the_read() {
        let s = spec(&PageState { view: None, note: "The session could not be saved.", busy: true });
        assert_eq!(s.groups[0].items[0].label, "The session could not be saved.");
        assert!(labels(&s).contains(&"Loading".to_string()));
        let mut view = signed_in();
        view.error = "The cloud service answered 500".into();
        let s = spec(&PageState { view: Some(&view), note: "", busy: false });
        assert_eq!(s.groups[0].items[0].label, "The cloud service answered 500");
    }

    #[test]
    fn the_root_row_says_sign_in_or_names_the_account() {
        assert_eq!(label(None), "");
        assert_eq!(label(Some(&View::default())), "Sign In");
        assert_eq!(label(Some(&signed_in())), "Ada");
        let mut no_name = signed_in();
        no_name.name.clear();
        assert_eq!(label(Some(&no_name)), "ada@example.com");
    }

    #[test]
    fn the_email_then_the_password_prompt_then_the_sign_in() {
        assert!(owns(&Purpose::AccountEmail));
        let Step::Ask(password) = next_prompt(&Purpose::AccountEmail, " Ada@Example.com ") else { panic!("the email step asks for the password") };
        assert!(password.secure);
        assert_eq!(password.purpose, Purpose::AccountPassword { email: "ada@example.com".into() });
        assert_eq!(next_prompt(&password.purpose, "pw"), Step::SignIn { email: "ada@example.com".into(), password: "pw".into() });
        let Step::Ask(again) = next_prompt(&Purpose::AccountEmail, "nope") else { panic!("a bad email asks again") };
        assert_eq!(again.purpose, Purpose::AccountEmail);
        assert!(!again.message.is_empty());
    }

    #[test]
    fn a_sign_in_answer_stores_the_session_with_its_lifetime_and_no_password() {
        let vault = Fake::default();
        let cookie = "bc_session=tok99; Path=/; Max-Age=2592000; HttpOnly";
        let account = save_answer(&vault, 200, Some(cookie), ACCOUNT, 1_000).unwrap();
        assert_eq!(account.email, "ada@example.com");
        let stored = session::load(&vault).unwrap().unwrap();
        assert_eq!(stored.token, "tok99");
        assert_eq!(stored.account_id, "acc1");
        assert_eq!(stored.server, SERVER);
        assert_eq!(stored.expires, Some(1_000 + 2_592_000));
        let raw = String::from_utf8(vault.item.borrow().clone().unwrap()).unwrap();
        assert!(!raw.contains("password"), "the password is never stored");
    }

    #[test]
    fn a_refused_sign_in_stores_nothing_and_says_why() {
        let vault = Fake::default();
        let err = save_answer(&vault, 401, None, r#"{"error":"bad_credentials"}"#, 1_000).unwrap_err();
        assert_eq!(err, CloudError::BadCredentials.to_string());
        assert!(vault.item.borrow().is_none());
        assert!(save_answer(&vault, 200, None, ACCOUNT, 1_000).is_err(), "an answer without the cookie is not a session");
    }

    #[test]
    fn devices_read_their_name_model_and_last_seen() {
        let rows = device_rows(r#"{"devices":[{"name":"Midget","model":"M900","last_seen":5},{"model":"no name"}],"this_device":null}"#);
        assert_eq!(rows, vec![DeviceRow { name: "Midget".into(), model: "M900".into(), last_seen: 5 }]);
        assert!(device_rows("not json").is_empty());
    }

    #[test]
    fn signing_out_without_a_session_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("account-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(sign_out(&dir), Ok(()));
        assert_eq!(load(&dir), View::default(), "no session reads as signed out");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
