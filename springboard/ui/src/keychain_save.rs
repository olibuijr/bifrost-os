//! Keychain save form: the panel that creates a new keychain from a name and a password.
//!
//! The model is pure. The caller sets the name, the password and its confirmation, then calls
//! [`KeychainSavePanel::save`]. A rejected form keeps the typed values so the user can correct them and records the
//! reason in [`KeychainSavePanel::error`]. A successful save records the created name in
//! [`KeychainSavePanel::keychain`] and clears both password fields. The model holds no store and reads no disk.
//! Rules: the name is trimmed; it is 1 to [`MAX_NAME`] bytes and holds no slash, colon or control character. The
//! password is [`MIN_PASSWORD`] to [`MAX_PASSWORD`] bytes and must equal its confirmation.

/// Shortest password a new keychain accepts, in bytes.
pub const MIN_PASSWORD: usize = 8;
/// Longest password a new keychain accepts, in bytes.
pub const MAX_PASSWORD: usize = 1024;
/// Longest keychain name, in bytes, after trimming.
pub const MAX_NAME: usize = 64;

/// Why the form was rejected. Checked in this order: name first, then password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    EmptyName,
    NameTooLong,
    /// The name holds a slash, a colon or a control character.
    BadName,
    EmptyPassword,
    PasswordTooShort,
    PasswordTooLong,
    /// The confirmation does not equal the password.
    PasswordMismatch,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeychainSavePanel {
    name: String,
    password: String,
    confirmation: String,
    error: Option<SaveError>,
    keychain: Option<String>,
}

impl KeychainSavePanel {
    /// An empty form with no error and no created keychain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the name typed into the form, as it was typed.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    /// Sets the password for the keychain that will be created (`setPassword`).
    pub fn set_password(&mut self, password: &str) {
        self.password = password.to_string();
    }

    /// Sets the confirmation field, which must repeat the password.
    pub fn set_confirmation(&mut self, confirmation: &str) {
        self.confirmation = confirmation.to_string();
    }

    /// The name as typed, before trimming.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Checks the form without saving it.
    pub fn validate(&self) -> Result<(), SaveError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(SaveError::EmptyName);
        }
        if name.len() > MAX_NAME {
            return Err(SaveError::NameTooLong);
        }
        if name.chars().any(|c| c == '/' || c == ':' || c.is_control()) {
            return Err(SaveError::BadName);
        }
        if self.password.is_empty() {
            return Err(SaveError::EmptyPassword);
        }
        if self.password.len() < MIN_PASSWORD {
            return Err(SaveError::PasswordTooShort);
        }
        if self.password.len() > MAX_PASSWORD {
            return Err(SaveError::PasswordTooLong);
        }
        if self.password != self.confirmation {
            return Err(SaveError::PasswordMismatch);
        }
        Ok(())
    }

    /// Saves the form. Returns `true` and records the keychain when the form is valid; otherwise records the error
    /// and keeps the typed values. A failed save leaves the previously created keychain in place.
    pub fn save(&mut self) -> bool {
        match self.validate() {
            Ok(()) => {
                self.keychain = Some(self.name.trim().to_string());
                self.password.clear();
                self.confirmation.clear();
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// The last rejection, or `None` after a successful save.
    pub fn error(&self) -> Option<&SaveError> {
        self.error.as_ref()
    }

    /// The name of the keychain the last successful save created.
    pub fn keychain(&self) -> Option<&str> {
        self.keychain.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(name: &str, password: &str, confirmation: &str) -> KeychainSavePanel {
        let mut p = KeychainSavePanel::new();
        p.set_name(name);
        p.set_password(password);
        p.set_confirmation(confirmation);
        p
    }

    #[test]
    fn blank_name_is_rejected_after_trimming() {
        let mut p = form("   ", "password1", "password1");
        assert!(!p.save());
        assert_eq!(p.error(), Some(&SaveError::EmptyName));
        assert_eq!(p.keychain(), None);
    }

    #[test]
    fn name_with_slash_or_colon_is_bad() {
        assert_eq!(form("a/b", "password1", "password1").validate(), Err(SaveError::BadName));
        assert_eq!(form("a:b", "password1", "password1").validate(), Err(SaveError::BadName));
    }

    #[test]
    fn name_limit_is_64_bytes() {
        let long = "a".repeat(MAX_NAME + 1);
        assert_eq!(form(&long, "password1", "password1").validate(), Err(SaveError::NameTooLong));
        let exact = "a".repeat(MAX_NAME);
        assert_eq!(form(&exact, "password1", "password1").validate(), Ok(()));
    }

    #[test]
    fn short_password_is_rejected_and_empty_is_checked_first() {
        assert_eq!(form("Work", "short", "short").validate(), Err(SaveError::PasswordTooShort));
        assert_eq!(form("Work", "", "").validate(), Err(SaveError::EmptyPassword));
    }

    #[test]
    fn confirmation_must_repeat_the_password() {
        assert_eq!(form("Work", "correct-horse", "correct-hors").validate(), Err(SaveError::PasswordMismatch));
    }

    #[test]
    fn password_limit_is_1024_bytes() {
        let long = "p".repeat(MAX_PASSWORD + 1);
        assert_eq!(form("Work", &long, &long).validate(), Err(SaveError::PasswordTooLong));
    }

    #[test]
    fn successful_save_records_trimmed_name_and_clears_passwords() {
        let mut p = form("  Work  ", "password1", "password1");
        assert!(p.save());
        assert_eq!(p.keychain(), Some("Work"));
        assert_eq!(p.error(), None);
        assert_eq!(p.validate(), Err(SaveError::EmptyPassword));
    }

    #[test]
    fn failed_save_keeps_the_previous_keychain() {
        let mut p = form("Work", "password1", "password1");
        assert!(p.save());
        p.set_name("Home");
        p.set_password("tiny");
        p.set_confirmation("tiny");
        assert!(!p.save());
        assert_eq!(p.keychain(), Some("Work"));
        assert_eq!(p.error(), Some(&SaveError::PasswordTooShort));
        assert_eq!(p.name(), "Home");
    }
}
