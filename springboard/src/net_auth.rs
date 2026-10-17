//! The network-authentication agent's prompt: what the user is asked when an SMB share or an 802.1X Wi-Fi network
//! wants a login, how the answer is checked, and where it goes.
//!
//! The decision "saved login or prompt" lives with the stores (`keychain::share_access` for shares,
//! `WifiSecrets::eap_access` in the SDK for 802.1X networks). This module owns the other half: the wording and
//! fields of the sheet ([`Prompt`]), the checks on what was typed ([`validate`]) and, for shares, saving the answer
//! only when the user chose to remember it ([`complete_share`]). It draws nothing; the shell builds the sheet from
//! the [`Prompt`].
//!
//! The shell sheet (`shell/network_auth_sheet.rs`) draws the prompt for an app's request (`shell/services/network_auth.rs`)
//! and finishes it with [`complete`]. The logins live in the keychain group [`GROUP`]: a share's login under
//! `keychain::SHARE_LOGIN_SERVICE`, an 802.1X network's under [`WIFI_LOGIN_SERVICE`], both as `user NUL password`.

use crate::keychain::{self, Error, ShareAccess, Store};

/// The keychain group that holds the logins this agent keeps (opened with `Store::open_group` on the shell data folder).
pub const GROUP: &str = "is.bifrost.network-auth";

/// The keychain service that holds 802.1X network logins; the account is the network name.
pub const WIFI_LOGIN_SERVICE: &str = "bifrost.network-auth.wifi";

/// Longest network name (SSID) a login is kept for, in bytes.
pub const MAX_SSID_BYTES: usize = 32;

/// Longest user name or password the sheet accepts, in bytes (the Wi-Fi secret limit, so one answer fits both stores).
pub const MAX_FIELD_BYTES: usize = 63;

/// What asks for a login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An SMB share on a host.
    Share { host: String, share: String },
    /// An 802.1X (enterprise) Wi-Fi network.
    Wifi { ssid: String },
}

impl Target {
    /// A stable text for the target, lower-case for shares (one share is one target whatever case it is named in):
    /// `share:host/share` or `wifi:ssid`. Requests and answers are matched on it.
    pub fn key(&self) -> String {
        match self {
            Target::Share { host, share } => format!("share:{}", keychain::share_login_account(host, share)),
            Target::Wifi { ssid } => format!("wifi:{ssid}"),
        }
    }
}

/// The sheet shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub target: Target,
    pub title: String,
    pub message: String,
    /// The user name to put in the field first (the one that was just refused), if any.
    pub user_hint: Option<String>,
    /// True when an earlier login for this target was refused, so the sheet says so.
    pub retry: bool,
    /// Whether "remember this login" starts checked.
    pub remember: bool,
    pub ok_label: String,
}

/// What the user typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub user: String,
    pub password: String,
    pub remember: bool,
}

/// Why an answer is not accepted; the sheet keeps the user in the field named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    EmptyUser,
    EmptyPassword,
    /// A field is longer than [`MAX_FIELD_BYTES`].
    TooLong,
    /// A field holds a NUL or other control character.
    Control,
}

/// The prompt for a share. `previous` is the login that was refused, if the share had one.
pub fn share_prompt(host: &str, share: &str, previous: Option<&str>) -> Prompt {
    let (host, share) = (host.trim(), share.trim());
    let retry = previous.is_some();
    Prompt {
        target: Target::Share { host: host.to_string(), share: share.to_string() },
        title: format!("Connect to \"{share}\""),
        message: if retry {
            format!("The login for \"{share}\" on {host} was not accepted. Enter the name and password again.")
        } else {
            format!("Enter a name and password to use the shared folder \"{share}\" on {host}.")
        },
        user_hint: previous.map(str::to_string),
        retry,
        remember: true,
        ok_label: "Connect".to_string(),
    }
}

/// The prompt for an 802.1X network. `previous` is the user name that was refused, if the network had a login.
pub fn wifi_prompt(ssid: &str, previous: Option<&str>) -> Prompt {
    let retry = previous.is_some();
    Prompt {
        target: Target::Wifi { ssid: ssid.to_string() },
        title: format!("Sign in to \"{ssid}\""),
        message: if retry {
            format!("The login for the network \"{ssid}\" was not accepted. Enter the name and password again.")
        } else {
            format!("The network \"{ssid}\" needs a name and password.")
        },
        user_hint: previous.map(str::to_string),
        retry,
        remember: true,
        ok_label: "Join".to_string(),
    }
}

fn field_ok(v: &str) -> Result<(), Invalid> {
    if v.len() > MAX_FIELD_BYTES {
        Err(Invalid::TooLong)
    } else if v.chars().any(|c| c.is_control()) {
        Err(Invalid::Control)
    } else {
        Ok(())
    }
}

/// Checks what was typed. The password is taken as typed.
pub fn validate(a: &Answer) -> Result<(), Invalid> {
    if a.user.is_empty() {
        return Err(Invalid::EmptyUser);
    }
    if a.password.is_empty() {
        return Err(Invalid::EmptyPassword);
    }
    field_ok(&a.user)?;
    field_ok(&a.password)
}

/// Whether one typed field is acceptable on its own, for the step-by-step sheet: not empty, within
/// [`MAX_FIELD_BYTES`], no control character.
pub fn valid_field(v: &str) -> bool {
    !v.is_empty() && field_ok(v).is_ok()
}

fn save_wifi_login(store: &Store, ssid: &str, user: &str, password: &str) -> Result<(), Error> {
    if ssid.is_empty() || ssid.len() > MAX_SSID_BYTES || user.contains('\0') {
        return Err(Error::Param);
    }
    let value = format!("{user}\0{password}");
    match store.add(WIFI_LOGIN_SERVICE, ssid, value.as_bytes()) {
        Err(Error::DuplicateItem) => store.update(WIFI_LOGIN_SERVICE, ssid, value.as_bytes()),
        other => other,
    }
}

fn wifi_login(store: &Store, ssid: &str) -> Result<(String, String), Error> {
    let bytes = store.get(WIFI_LOGIN_SERVICE, ssid)?;
    let at = bytes.iter().position(|&b| b == 0).ok_or(Error::ItemNotFound)?;
    let user = String::from_utf8(bytes[..at].to_vec()).map_err(|_| Error::ItemNotFound)?;
    let password = String::from_utf8(bytes[at + 1..].to_vec()).map_err(|_| Error::ItemNotFound)?;
    Ok((user, password))
}

/// The access a join of an 802.1X network starts from: the stored login, or `Prompt` when none is stored.
pub fn wifi_access(store: &Store, ssid: &str) -> Result<ShareAccess, Error> {
    match wifi_login(store, ssid) {
        Ok((user, password)) => Ok(ShareAccess::Saved { user, password }),
        Err(Error::ItemNotFound) => Ok(ShareAccess::Prompt),
        Err(e) => Err(e),
    }
}

/// The access a mount or join of `target` starts from: its stored login, or `Prompt`.
pub fn access(store: &Store, target: &Target) -> Result<ShareAccess, Error> {
    match target {
        Target::Share { host, share } => keychain::share_access(store, host, share),
        Target::Wifi { ssid } => wifi_access(store, ssid),
    }
}

/// Forgets the stored login of `target`, for instance after the server refused it. Forgetting a login that is not
/// there is not an error.
pub fn forget(store: &Store, target: &Target) -> Result<(), Error> {
    match target {
        Target::Share { host, share } => keychain::forget_share_login(store, host, share),
        Target::Wifi { ssid } => match store.delete(WIFI_LOGIN_SERVICE, ssid) {
            Err(Error::ItemNotFound) => Ok(()),
            other => other,
        },
    }
}

/// The prompt for `target`; `previous` is the user name that was refused, if the target had a login.
pub fn prompt_for(target: &Target, previous: Option<&str>) -> Prompt {
    match target {
        Target::Share { host, share } => share_prompt(host, share, previous),
        Target::Wifi { ssid } => wifi_prompt(ssid, previous),
    }
}

/// Finishes a prompt for a share or a network: checks the answer, saves it only when "remember" is on, and returns the
/// login to use now. `Err(Error::Param)` for a bad answer.
pub fn complete(store: &Store, prompt: &Prompt, a: &Answer) -> Result<ShareAccess, Error> {
    match &prompt.target {
        Target::Share { .. } => complete_share(store, prompt, a),
        Target::Wifi { ssid } => {
            validate(a).map_err(|_| Error::Param)?;
            if a.remember {
                save_wifi_login(store, ssid, &a.user, &a.password)?;
            }
            Ok(ShareAccess::Saved { user: a.user.clone(), password: a.password.clone() })
        }
    }
}

/// Finishes a share prompt: checks the answer, saves it in the keychain only when "remember" is on, and returns the
/// login to mount with now. Nothing is stored for a bad answer or for an answer the user did not ask to remember.
/// `Err(Error::Param)` for a bad answer or a prompt that is not for a share.
pub fn complete_share(store: &Store, prompt: &Prompt, a: &Answer) -> Result<ShareAccess, Error> {
    let Target::Share { host, share } = &prompt.target else { return Err(Error::Param) };
    validate(a).map_err(|_| Error::Param)?;
    if a.remember {
        keychain::answer_share_prompt(store, host, share, &a.user, &a.password)
    } else {
        Ok(ShareAccess::Saved { user: a.user.clone(), password: a.password.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> Store {
        let d = std::env::temp_dir().join(format!("bifrost-netauth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Store::with_master(d.join("a.kc"), &[7u8; 32], "netauth")
    }

    fn ans(user: &str, pw: &str, remember: bool) -> Answer {
        Answer { user: user.into(), password: pw.into(), remember }
    }

    #[test]
    fn a_first_prompt_and_a_retry_say_different_things() {
        let first = share_prompt("nas.local", "Music", None);
        assert!(!first.retry && first.user_hint.is_none() && first.remember);
        assert!(first.message.contains("Music") && first.message.contains("nas.local"));
        let again = share_prompt("nas.local", "Music", Some("ola"));
        assert!(again.retry && again.message.contains("not accepted"));
        assert_eq!(again.user_hint.as_deref(), Some("ola"));
        let w = wifi_prompt("Campus", Some("ola"));
        assert_eq!(w.target, Target::Wifi { ssid: "Campus".into() });
        assert!(w.retry && w.title.contains("Campus"));
    }

    #[test]
    fn an_answer_is_checked_before_it_is_used() {
        assert_eq!(validate(&ans("ola", "pw", true)), Ok(()));
        assert_eq!(validate(&ans("", "pw", true)), Err(Invalid::EmptyUser));
        assert_eq!(validate(&ans("ola", "", true)), Err(Invalid::EmptyPassword));
        assert_eq!(validate(&ans("o\0la", "pw", true)), Err(Invalid::Control));
        assert_eq!(validate(&ans("ola", &"p".repeat(64), true)), Err(Invalid::TooLong));
        assert_eq!(validate(&ans("ola", &"p".repeat(63), true)), Ok(()));
    }

    #[test]
    fn a_remembered_answer_is_saved_and_a_one_off_answer_is_not() {
        let s = store("complete");
        let p = share_prompt("NAS.local", "Music", None);
        let used = complete_share(&s, &p, &ans("ola", "pw1", false)).unwrap();
        assert_eq!(used, ShareAccess::Saved { user: "ola".into(), password: "pw1".into() });
        assert_eq!(keychain::share_access(&s, "nas.local", "music"), Ok(ShareAccess::Prompt), "not remembered");
        complete_share(&s, &p, &ans("ola", "pw2", true)).unwrap();
        assert_eq!(
            keychain::share_access(&s, "nas.local", "music"),
            Ok(ShareAccess::Saved { user: "ola".into(), password: "pw2".into() })
        );
    }

    #[test]
    fn a_bad_answer_or_a_wifi_prompt_stores_nothing() {
        let s = store("bad");
        let p = share_prompt("nas.local", "Music", None);
        assert_eq!(complete_share(&s, &p, &ans("", "pw", true)), Err(Error::Param));
        assert_eq!(keychain::share_access(&s, "nas.local", "music"), Ok(ShareAccess::Prompt));
        let w = wifi_prompt("Campus", None);
        assert_eq!(complete_share(&s, &w, &ans("ola", "pw", true)), Err(Error::Param));
    }

    #[test]
    fn a_wifi_login_is_kept_only_when_asked_and_found_again() {
        let s = store("wifi");
        let target = Target::Wifi { ssid: "Campus".into() };
        let p = wifi_prompt("Campus", None);
        assert_eq!(access(&s, &target), Ok(ShareAccess::Prompt));
        complete(&s, &p, &ans("ola", "pw1", false)).unwrap();
        assert_eq!(access(&s, &target), Ok(ShareAccess::Prompt), "not remembered");
        let used = complete(&s, &p, &ans("ola", "pw2", true)).unwrap();
        assert_eq!(used, ShareAccess::Saved { user: "ola".into(), password: "pw2".into() });
        assert_eq!(access(&s, &target), Ok(ShareAccess::Saved { user: "ola".into(), password: "pw2".into() }));
        complete(&s, &p, &ans("ola", "pw3", true)).unwrap();
        assert_eq!(access(&s, &target), Ok(ShareAccess::Saved { user: "ola".into(), password: "pw3".into() }), "a new login replaces the old");
        forget(&s, &target).unwrap();
        assert_eq!(access(&s, &target), Ok(ShareAccess::Prompt));
        forget(&s, &target).unwrap();
    }

    #[test]
    fn a_target_has_one_key_whatever_the_case_and_a_field_is_checked_alone() {
        let a = Target::Share { host: "NAS.local".into(), share: "Music".into() };
        let b = Target::Share { host: "nas.local".into(), share: "music".into() };
        assert_eq!(a.key(), b.key());
        assert_eq!(Target::Wifi { ssid: "Campus".into() }.key(), "wifi:Campus");
        assert!(valid_field("ola") && !valid_field("") && !valid_field("o\0") && !valid_field(&"p".repeat(64)));
        let s = store("wifi-bad");
        assert_eq!(complete(&s, &wifi_prompt("", None), &ans("ola", "pw", true)), Err(Error::Param), "no network name");
        assert_eq!(prompt_for(&a, Some("ola")), share_prompt("NAS.local", "Music", Some("ola")));
    }
}
