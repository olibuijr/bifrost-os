//! The network-authentication agent's prompt: what the user is asked when an SMB share or an 802.1X Wi-Fi network
//! wants a login, how the answer is checked, and where it goes.
//!
//! The decision "saved login or prompt" lives with the stores (`keychain::share_access` for shares,
//! `WifiSecrets::eap_access` in the SDK for 802.1X networks). This module owns the other half: the wording and
//! fields of the sheet ([`Prompt`]), the checks on what was typed ([`validate`]) and, for shares, saving the answer
//! only when the user chose to remember it ([`complete_share`]). It draws nothing; the shell builds the sheet from
//! the [`Prompt`].

use crate::keychain::{self, Error, ShareAccess, Store};

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
}
