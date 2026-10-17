//! One-field text prompts for Settings panes (a computer name, a new account's name and password, a group name).
//! The sheet is the Wi-Fi password sheet drawn with a different title, label and validity rule (`Sheet::form`); the
//! `Purpose` it carries says what the typed text is for and holds what earlier prompts of the same flow collected.
use crate::privd::accounts;
use bifrost_ui::internet_accounts::{self as shared, Kind};

/// What a prompt's text is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    ComputerName,
    LocalHostname,
    /// step 1 of a new account: the full name
    NewUserFull { admin: bool },
    /// step 2: the account name
    NewUserName { admin: bool, full: String },
    /// step 3: the password
    NewUserPassword { admin: bool, full: String, name: String },
    /// the installed account proves its current password first
    CurrentPassword { name: String },
    /// `old` is the current password when the account is the installed one
    ChangePassword { name: String, old: Option<String> },
    /// the new password typed again: `then` (the account or password change that asked for it) runs only when the
    /// text equals `first`, the password typed before
    ConfirmPassword { first: String, then: Box<Purpose> },
    NewGroup,
    /// Print & Scan: the device address of a printer to add (`ipp://`, `socket://`, `usb://`)
    PrinterAddress,
    /// Internet Accounts step 1 of 3: the email address (mail) or the server URL (CalDAV, CardDAV)
    NewAccountAddress { kind: Kind },
    /// step 2: the user name of the account
    NewAccountUser { kind: Kind, address: String },
    /// step 3: the password, which is saved in Passwords when the account is added
    NewAccountPassword { kind: Kind, address: String, username: String },
    /// Location Services > Set Location, step 1 of 2: the latitude in degrees (see `location_page::step`)
    SetLatitude,
    /// step 2: the longitude; `latitude` is the text typed in step 1
    SetLongitude { latitude: String },
    /// Settings > BifrOSt Account step 1 of 2: the email address of the account (see `account_page`)
    AccountEmail,
    /// step 2 of 2: the password for `email`. It goes to the cloud's sign-in route and is not kept.
    AccountPassword { email: String },
    /// Bluetooth pairing: the PIN (up to 16 characters) or the six-digit passkey `addr` asks for. Not kept; it is
    /// sent to the radio as `Cmd::AnswerPairing`.
    BluetoothPairing { addr: String, passkey: bool },
    /// the password an app asked for through the `local_auth` service (the sheet is `auth_sheet.rs`); the typed text
    /// goes to the service's answer and is never stored or sent to the app
    AppAuthentication { app: String },
    /// the passphrase of a locked encrypted volume (the sheet is `volume_sheet.rs`); it goes to privd's unlock and is never kept
    VolumeUnlock { device: String },
}

/// What the sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub purpose: Purpose,
    pub title: String,
    pub message: String,
    pub label: &'static str,
    pub ok: &'static str,
    pub secure: bool,
    pub max: usize,
    pub initial: String,
}

impl Prompt {
    fn new(purpose: Purpose, title: &str, message: &str, label: &'static str, ok: &'static str) -> Prompt {
        Prompt { purpose, title: title.into(), message: message.into(), label, ok, secure: false, max: 64, initial: String::new() }
    }

    pub fn computer_name(current: &str) -> Prompt {
        Prompt { initial: current.into(), ..Prompt::new(Purpose::ComputerName, "Computer Name", "The name other people see for this computer.", "Name", "Done") }
    }

    pub fn local_hostname(current: &str) -> Prompt {
        Prompt { max: 63, initial: current.into(), ..Prompt::new(Purpose::LocalHostname, "Local Hostname", "Other computers on the network reach this one as this name followed by .local. Letters, digits and hyphens.", "Hostname", "Done") }
    }

    pub fn new_user(admin: bool) -> Prompt {
        let title = if admin { "New Administrator" } else { "New Standard User" };
        Prompt::new(Purpose::NewUserFull { admin }, title, "Enter the person's full name.", "Full Name", "Next")
    }

    pub fn change_password(name: &str) -> Prompt {
        Prompt { secure: true, max: 128, ..Prompt::new(Purpose::ChangePassword { name: name.into(), old: None }, "Change Password", &format!("Enter a new password for “{name}”."), "Password", "Change") }
    }

    pub fn current_password(name: &str) -> Prompt {
        Prompt { secure: true, max: 128, ..Prompt::new(Purpose::CurrentPassword { name: name.into() }, "Current Password", &format!("Enter the current password for “{name}”."), "Password", "Next") }
    }

    pub fn printer_address() -> Prompt {
        Prompt { max: 255, ..Prompt::new(Purpose::PrinterAddress, "Add Printer", "Enter the address of the printer, such as ipp://printer.local/ipp/print or socket://192.168.1.20.", "Address", "Add") }
    }

    /// Location Services > Set Location, step 1: the latitude, prefilled with the current one.
    pub fn set_latitude(current: &str) -> Prompt {
        Prompt {
            max: 16,
            initial: current.into(),
            ..Prompt::new(Purpose::SetLatitude, "Set Location", "Enter the latitude in degrees, from -90 (south) to 90 (north). A decimal comma works too.", "Latitude", "Next")
        }
    }

    /// Location Services > Set Location, step 2: the longitude.
    pub fn set_longitude(latitude: &str) -> Prompt {
        Prompt {
            max: 16,
            ..Prompt::new(Purpose::SetLongitude { latitude: latitude.into() }, "Set Location", "Enter the longitude in degrees, from -180 (west) to 180 (east).", "Longitude", "Set")
        }
    }

    /// BifrOSt Account step 1: the email address. `message` is the hint, or the cloud's reason after a refusal.
    pub fn account_email(message: &str) -> Prompt {
        Prompt { max: crate::cloud_account::MAX_EMAIL_BYTES, ..Prompt::new(Purpose::AccountEmail, "BifrOSt Account", message, "Email", "Next") }
    }

    /// BifrOSt Account step 2: the password of `email`. Never stored.
    pub fn account_password(email: &str, message: &str) -> Prompt {
        Prompt {
            secure: true,
            max: 256,
            ..Prompt::new(Purpose::AccountPassword { email: email.to_string() }, "Sign In", message, "Password", "Sign In")
        }
    }

    /// The Bluetooth pairing prompt for the device `addr`: a six-digit passkey, or a PIN of up to 16 characters.
    pub fn bluetooth_pairing(addr: &str, passkey: bool) -> Prompt {
        let (message, placeholder, max) = if passkey {
            ("Type the six-digit passkey shown on the device.", "Passkey", 6)
        } else {
            ("Type the PIN for the device: up to 16 letters or digits.", "PIN", 16)
        };
        Prompt { secure: true, max, ..Prompt::new(Purpose::BluetoothPairing { addr: addr.to_string(), passkey }, "Pair Device", message, placeholder, "Pair") }
    }

    pub fn new_group() -> Prompt {
        Prompt { max: 32, ..Prompt::new(Purpose::NewGroup, "New Group", "Lowercase letters, digits, hyphens and underscores.", "Name", "Add") }
    }

    /// Internet Accounts step 1: the address of the account.
    pub fn new_account_address(kind: Kind) -> Prompt {
        let (title, message, label) = match kind {
            Kind::Mail => ("Add Mail Account", "Enter the email address. The IMAP and SMTP servers are taken from its domain.", "Email Address"),
            Kind::CalDav => ("Add Calendar Account", "Enter the CalDAV server address, such as https://dav.example.com/.", "Server Address"),
            Kind::CardDav => ("Add Contacts Account", "Enter the CardDAV server address, such as https://dav.example.com/.", "Server Address"),
        };
        Prompt { max: 255, ..Prompt::new(Purpose::NewAccountAddress { kind }, title, message, label, "Next") }
    }

    /// Internet Accounts step 2: the user name. A mail account starts with its address.
    pub fn new_account_user(kind: Kind, address: &str) -> Prompt {
        let initial = if kind == Kind::Mail { address.to_string() } else { String::new() };
        Prompt {
            max: 128,
            initial,
            ..Prompt::new(Purpose::NewAccountUser { kind, address: address.to_string() }, "Add Account", "Enter the user name for the account.", "User Name", "Next")
        }
    }

    /// Internet Accounts step 3: the password, kept in Passwords.
    pub fn new_account_password(kind: Kind, address: &str, username: &str) -> Prompt {
        Prompt {
            secure: true,
            max: 128,
            ..Prompt::new(
                Purpose::NewAccountPassword { kind, address: address.to_string(), username: username.to_string() },
                "Add Account",
                "Enter the password. It is saved in Passwords, not in the account list.",
                "Password",
                "Add",
            )
        }
    }

    /// The prompt that follows `purpose` once `text` was accepted, when the flow has more steps.
    pub fn next(purpose: &Purpose, text: &str) -> Option<Prompt> {
        match purpose {
            Purpose::NewUserFull { admin } => {
                let full = text.trim().to_string();
                // the suggestion is the first word, lowercased
                let suggestion: String = full.split_whitespace().next().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
                Some(Prompt {
                    max: 32,
                    initial: if accounts::valid_user(&suggestion) { suggestion } else { String::new() },
                    ..Prompt::new(Purpose::NewUserName { admin: *admin, full }, "Account Name", "Lowercase letters, digits, hyphens and underscores. It names the home folder and cannot be changed later.", "Account Name", "Next")
                })
            }
            Purpose::NewUserName { admin, full } => Some(Prompt {
                secure: true,
                max: 128,
                ..Prompt::new(Purpose::NewUserPassword { admin: *admin, full: full.clone(), name: text.to_string() }, "Password", &format!("Choose a password for “{text}”."), "Password", "Create")
            }),
            Purpose::CurrentPassword { name } => Some(Prompt {
                secure: true,
                max: 128,
                ..Prompt::new(Purpose::ChangePassword { name: name.clone(), old: Some(text.to_string()) }, "New Password", &format!("Enter a new password for “{name}”."), "Password", "Change")
            }),
            Purpose::NewUserPassword { .. } | Purpose::ChangePassword { .. } => {
                let ok = if matches!(purpose, Purpose::NewUserPassword { .. }) { "Create" } else { "Change" };
                Some(Prompt {
                    secure: true,
                    max: 128,
                    ..Prompt::new(Purpose::ConfirmPassword { first: text.to_string(), then: Box::new(purpose.clone()) }, "Confirm Password", "Type the password again to confirm it.", "Password", ok)
                })
            }
            _ => None,
        }
    }
}

/// Whether `text` may be submitted for `purpose`.
pub fn acceptable(purpose: &Purpose, text: &str) -> bool {
    match purpose {
        Purpose::ComputerName => accounts::valid_computer_name(text),
        Purpose::LocalHostname => accounts::valid_hostname(text),
        Purpose::NewUserFull { .. } => accounts::valid_full_name(text),
        Purpose::NewUserName { .. } | Purpose::NewGroup => accounts::valid_user(text),
        Purpose::PrinterAddress => bifrost_ui::cups::valid_device_uri(text.trim()),
        Purpose::NewUserPassword { .. } | Purpose::ChangePassword { .. } | Purpose::CurrentPassword { .. } | Purpose::ConfirmPassword { .. } => accounts::valid_password(text),
        Purpose::NewAccountAddress { kind: Kind::Mail } => shared::valid_address(text.trim()),
        Purpose::NewAccountAddress { .. } => shared::valid_dav_url(text.trim()),
        Purpose::NewAccountUser { .. } => !text.trim().is_empty() && !text.contains(['\r', '\n']),
        Purpose::NewAccountPassword { .. } => !text.is_empty(),
        Purpose::AccountEmail => crate::cloud_account::normalize_email(text).is_ok(),
        Purpose::AccountPassword { .. } => !text.is_empty(),
        Purpose::AppAuthentication { .. } | Purpose::VolumeUnlock { .. } => !text.is_empty(),
        Purpose::SetLatitude => super::location_page::coordinate(text, 90.0).is_some(),
        Purpose::SetLongitude { .. } => super::location_page::coordinate(text, 180.0).is_some(),
        Purpose::BluetoothPairing { passkey: true, .. } => bifrost_core::bt_pairing::validate_passkey(text).is_ok(),
        Purpose::BluetoothPairing { passkey: false, .. } => bifrost_core::bt_pairing::validate_pin(text).is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_password_is_typed_again_before_it_is_used() {
        let purpose = Purpose::NewUserPassword { admin: false, full: "Kid".into(), name: "kid".into() };
        let confirm = Prompt::next(&purpose, "s3cret-one").expect("confirm step");
        assert_eq!(confirm.purpose, Purpose::ConfirmPassword { first: "s3cret-one".into(), then: Box::new(purpose) });
        assert_eq!(confirm.ok, "Create");
        assert!(Prompt::next(&confirm.purpose, "s3cret-one").is_none());
    }

    #[test]
    fn each_purpose_checks_its_text() {
        assert!(acceptable(&Purpose::ComputerName, "Living Room"));
        assert!(!acceptable(&Purpose::ComputerName, ""));
        assert!(acceptable(&Purpose::LocalHostname, "living-room"));
        assert!(!acceptable(&Purpose::LocalHostname, "living room"));
        assert!(acceptable(&Purpose::NewGroup, "friends") && !acceptable(&Purpose::NewGroup, "Friends"));
        assert!(!acceptable(&Purpose::ChangePassword { name: "kid".into(), old: None }, ""));
    }

    #[test]
    fn a_new_account_asks_for_name_then_account_then_password() {
        let first = Prompt::new_user(true);
        assert_eq!(first.title, "New Administrator");
        let second = Prompt::next(&first.purpose, " Ása Jónsdóttir ").expect("an account name follows");
        assert_eq!(second.purpose, Purpose::NewUserName { admin: true, full: "Ása Jónsdóttir".into() });
        assert_eq!(second.initial, "sa", "the suggestion keeps ASCII letters of the first word");
        let third = Prompt::next(&second.purpose, "asa").expect("a password follows");
        assert!(third.secure);
        assert_eq!(third.purpose, Purpose::NewUserPassword { admin: true, full: "Ása Jónsdóttir".into(), name: "asa".into() });
        assert!(Prompt::next(&third.purpose, "pw").is_none(), "the flow ends with the password");
    }

    #[test]
    fn an_unusable_suggestion_is_left_empty() {
        let second = Prompt::next(&Purpose::NewUserFull { admin: false }, "9 Lives").expect("account name step");
        assert_eq!(second.initial, "", "an account name cannot start with a digit");
    }

    #[test]
    fn bluetooth_pairing_takes_a_pin_or_a_six_digit_passkey() {
        let pin = Prompt::bluetooth_pairing("AA:BB:CC:DD:EE:FF", false);
        assert!(pin.secure && pin.max == 16);
        assert!(acceptable(&pin.purpose, "1234") && acceptable(&pin.purpose, "abc123"));
        assert!(!acceptable(&pin.purpose, "") && !acceptable(&pin.purpose, "12 34"));
        let passkey = Prompt::bluetooth_pairing("AA:BB:CC:DD:EE:FF", true);
        assert_eq!(passkey.max, 6);
        assert!(acceptable(&passkey.purpose, "000042"));
        assert!(!acceptable(&passkey.purpose, "42") && !acceptable(&passkey.purpose, "12a456"));
    }
}
