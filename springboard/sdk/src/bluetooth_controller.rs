//! The local Bluetooth controller's name, checked before an app asks for it to be used.
//!
//! [`LocalName::new`] accepts UTF-8 text of 1 to [`MAX_BYTES`] bytes with no NUL character. Those are the limits the
//! Bluetooth core specification sets for the local name, and a NUL would end the name early on the controller.
//!
//! This is the pure model. Nothing is written to the radio from this module, and the shell does not take a name from an
//! app yet. The powered state of the radio is read and set through `bluetooth_service`.

/// Longest local name, in bytes.
pub const MAX_BYTES: usize = 248;

/// Why a local name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalNameError {
    Empty,
    TooLong,
    ContainsNul,
}

impl std::fmt::Display for LocalNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            LocalNameError::Empty => "local name is empty",
            LocalNameError::TooLong => "local name is longer than 248 bytes",
            LocalNameError::ContainsNul => "local name may not contain a NUL character",
        };
        f.write_str(text)
    }
}

/// A checked local name for the Bluetooth controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalName(String);

impl LocalName {
    /// Checks `name` and keeps it when every rule holds. The first failing rule is reported.
    pub fn new(name: &str) -> Result<LocalName, LocalNameError> {
        if name.is_empty() {
            return Err(LocalNameError::Empty);
        }
        if name.len() > MAX_BYTES {
            return Err(LocalNameError::TooLong);
        }
        if name.contains('\0') {
            return Err(LocalNameError::ContainsNul);
        }
        Ok(LocalName(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
