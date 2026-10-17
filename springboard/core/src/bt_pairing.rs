//! Pairing input (`IOBluetoothDevicePair` user-entry parity, see `docs/parity/iobluetooth.md`).
//!
//! The radio asks for one of four things while a device pairs: nothing (Just Works), a six-digit number to compare
//! with the device's screen (numeric comparison), a legacy PIN the user types, or a six-digit passkey the user types.
//! This module checks what the user typed and turns it into the answer the pairing step needs. It is pure: it does not
//! reach the radio. The shell asks the user through `Status::prompt` (`springboard/src/bare/bluetooth.rs`).

use std::fmt;

/// The longest legacy PIN the Bluetooth core allows, in bytes.
pub const MAX_PIN_LEN: usize = 16;

/// The highest passkey value: six decimal digits.
pub const MAX_PASSKEY: u32 = 999_999;

/// What the device asks for while pairing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingMethod {
    /// No user input: the user confirms on the device side or not at all.
    JustWorks,
    /// Show this six-digit number; the user confirms that the device shows the same one.
    NumericComparison(u32),
    /// The user types a legacy PIN.
    PinCode,
    /// The user types a six-digit passkey.
    Passkey,
}

/// The answer the pairing step receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingAnswer {
    Pin(String),
    Passkey(u32),
    Confirmed(bool),
}

/// Why the typed input was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingError {
    InvalidPin,
    InvalidPasskey,
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PairingError::InvalidPin => "a PIN is 1 to 16 letters or digits",
            PairingError::InvalidPasskey => "a passkey is exactly 6 digits",
        })
    }
}

/// Checks a legacy PIN: 1 to 16 ASCII letters or digits.
pub fn validate_pin(pin: &str) -> Result<String, PairingError> {
    let ok = !pin.is_empty() && pin.len() <= MAX_PIN_LEN && pin.bytes().all(|b| b.is_ascii_alphanumeric());
    if ok {
        Ok(pin.to_string())
    } else {
        Err(PairingError::InvalidPin)
    }
}

/// Checks a passkey: exactly six decimal digits, with a value up to [`MAX_PASSKEY`]. Leading zeros are kept in the
/// text, so `"000042"` is 42.
pub fn validate_passkey(text: &str) -> Result<u32, PairingError> {
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PairingError::InvalidPasskey);
    }
    let value: u32 = text.parse().map_err(|_| PairingError::InvalidPasskey)?;
    if value > MAX_PASSKEY {
        return Err(PairingError::InvalidPasskey);
    }
    Ok(value)
}

/// The six-digit comparison code as the user reads it on screen, with leading zeros. None above [`MAX_PASSKEY`].
pub fn numeric_code(value: u32) -> Option<String> {
    (value <= MAX_PASSKEY).then(|| format!("{value:06}"))
}

/// The answer for `method` given what the user typed. Just Works and numeric comparison take no text: the user's
/// button press is passed as `confirmed`.
pub fn answer_for(method: &PairingMethod, typed: &str, confirmed: bool) -> Result<PairingAnswer, PairingError> {
    match method {
        PairingMethod::JustWorks | PairingMethod::NumericComparison(_) => Ok(PairingAnswer::Confirmed(confirmed)),
        PairingMethod::PinCode => validate_pin(typed).map(PairingAnswer::Pin),
        PairingMethod::Passkey => validate_passkey(typed).map(PairingAnswer::Passkey),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_one_to_sixteen_letters_or_digits() {
        assert_eq!(validate_pin("0000"), Ok("0000".to_string()));
        assert_eq!(validate_pin("abc123"), Ok("abc123".to_string()));
        assert_eq!(validate_pin(""), Err(PairingError::InvalidPin));
        assert_eq!(validate_pin(&"1".repeat(17)), Err(PairingError::InvalidPin));
        assert_eq!(validate_pin("12 34"), Err(PairingError::InvalidPin));
        assert_eq!(validate_pin("12-34"), Err(PairingError::InvalidPin));
    }

    #[test]
    fn a_passkey_is_six_digits_and_keeps_its_value() {
        assert_eq!(validate_passkey("000042"), Ok(42));
        assert_eq!(validate_passkey("999999"), Ok(MAX_PASSKEY));
        assert_eq!(validate_passkey("42"), Err(PairingError::InvalidPasskey));
        assert_eq!(validate_passkey("1234567"), Err(PairingError::InvalidPasskey));
        assert_eq!(validate_passkey("12a456"), Err(PairingError::InvalidPasskey));
        assert_eq!(validate_passkey("+12345"), Err(PairingError::InvalidPasskey));
    }

    #[test]
    fn a_numeric_code_is_padded_to_six_digits_and_bounded() {
        assert_eq!(numeric_code(42).as_deref(), Some("000042"));
        assert_eq!(numeric_code(MAX_PASSKEY).as_deref(), Some("999999"));
        assert_eq!(numeric_code(MAX_PASSKEY + 1), None);
    }

    #[test]
    fn answers_follow_the_method_the_device_asked_for() {
        assert_eq!(answer_for(&PairingMethod::JustWorks, "", true), Ok(PairingAnswer::Confirmed(true)));
        assert_eq!(
            answer_for(&PairingMethod::NumericComparison(123456), "", false),
            Ok(PairingAnswer::Confirmed(false))
        );
        assert_eq!(answer_for(&PairingMethod::PinCode, "1234", false), Ok(PairingAnswer::Pin("1234".into())));
        assert_eq!(answer_for(&PairingMethod::PinCode, "", false), Err(PairingError::InvalidPin));
        assert_eq!(answer_for(&PairingMethod::Passkey, "000007", false), Ok(PairingAnswer::Passkey(7)));
        assert_eq!(answer_for(&PairingMethod::Passkey, "7", false), Err(PairingError::InvalidPasskey));
    }

    #[test]
    fn errors_read_as_plain_sentences() {
        assert_eq!(PairingError::InvalidPasskey.to_string(), "a passkey is exactly 6 digits");
    }
}
