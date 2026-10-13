//! One-field text prompts for Settings panes (a computer name, a new account's name and password, a group name).
//! The sheet is the Wi-Fi password sheet drawn with a different title, label and validity rule (`Sheet::form`); the
//! `Purpose` it carries says what the typed text is for and holds what earlier prompts of the same flow collected.
use crate::privd::accounts;

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
    NewGroup,
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

    pub fn new_group() -> Prompt {
        Prompt { max: 32, ..Prompt::new(Purpose::NewGroup, "New Group", "Lowercase letters, digits, hyphens and underscores.", "Name", "Add") }
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
        Purpose::NewUserPassword { .. } | Purpose::ChangePassword { .. } | Purpose::CurrentPassword { .. } => accounts::valid_password(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
