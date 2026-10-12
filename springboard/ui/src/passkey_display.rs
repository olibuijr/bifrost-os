//! Passkey display for pairing: the pure model behind the sheet that shows a pairing passkey while the remote device
//! confirms it. It holds the passkey, the device it belongs to, whether it came from a simple secure pairing, whether
//! the request came from the remote side, and an indicator that walks across the digits as the remote side types them.
//! The pairing layer supplies the passkey; the sheet draws the digits and the indicator.
//!
//! Rules: a passkey is exactly six ASCII digits, and it is refused otherwise and left unset. The indicator starts at
//! zero, advances by one up to six, retreats down to zero, and resets to zero. Indicator moves do nothing without a
//! passkey.
//!
//! Remaining gaps: the pairing layer does not show the passkey yet; the passkey is auto-confirmed, not displayed.

/// Digits in a passkey.
pub const PASSKEY_DIGITS: usize = 6;

/// Why a passkey was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasskeyError {
    NotSixDigits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasskeyDisplay {
    passkey: Option<String>,
    device: Option<String>,
    using_ssp: bool,
    incoming: bool,
    indicator: usize,
}

impl Default for PasskeyDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyDisplay {
    pub fn new() -> Self {
        PasskeyDisplay { passkey: None, device: None, using_ssp: false, incoming: false, indicator: 0 }
    }

    /// Sets the passkey for `device`, with the indicator back at the start. Refuses anything that is not exactly
    /// six ASCII digits and keeps the previous passkey.
    pub fn set_passkey(&mut self, passkey: &str, device: &str, using_ssp: bool) -> Result<(), PasskeyError> {
        let valid = passkey.len() == PASSKEY_DIGITS && passkey.bytes().all(|b| b.is_ascii_digit());
        if !valid {
            return Err(PasskeyError::NotSixDigits);
        }
        self.passkey = Some(passkey.to_string());
        self.device = Some(device.to_string());
        self.using_ssp = using_ssp;
        self.indicator = 0;
        Ok(())
    }

    pub fn passkey(&self) -> Option<&str> {
        self.passkey.as_deref()
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    pub fn using_ssp(&self) -> bool {
        self.using_ssp
    }

    pub fn is_incoming_request(&self) -> bool {
        self.incoming
    }

    pub fn set_incoming_request(&mut self, incoming: bool) {
        self.incoming = incoming;
    }

    /// Digits highlighted so far, 0 to six.
    pub fn indicator(&self) -> usize {
        self.indicator
    }

    /// Moves the indicator one digit forward, up to the last digit. Does nothing without a passkey.
    pub fn advance_indicator(&mut self) -> usize {
        if self.passkey.is_some() && self.indicator < PASSKEY_DIGITS {
            self.indicator += 1;
        }
        self.indicator
    }

    /// Moves the indicator one digit back, down to zero.
    pub fn retreat_indicator(&mut self) -> usize {
        self.indicator = self.indicator.saturating_sub(1);
        self.indicator
    }

    pub fn reset_indicator(&mut self) {
        self.indicator = 0;
    }

    /// The passkey split at the indicator: the digits already typed, and the digits still to type.
    pub fn split_at_indicator(&self) -> Option<(&str, &str)> {
        self.passkey.as_deref().map(|p| p.split_at(self.indicator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_passkey_keeps_the_device_and_starts_the_indicator_at_zero() {
        let mut display = PasskeyDisplay::new();
        assert_eq!(display.set_passkey("123456", "Keyboard", true), Ok(()));
        assert_eq!(display.passkey(), Some("123456"));
        assert_eq!(display.device(), Some("Keyboard"));
        assert!(display.using_ssp());
        assert_eq!(display.indicator(), 0);
        assert_eq!(display.split_at_indicator(), Some(("", "123456")));
    }

    #[test]
    fn invalid_passkeys_are_refused_and_keep_the_old_one() {
        let mut display = PasskeyDisplay::new();
        assert_eq!(display.set_passkey("12345", "Keyboard", false), Err(PasskeyError::NotSixDigits));
        assert_eq!(display.set_passkey("12a456", "Keyboard", false), Err(PasskeyError::NotSixDigits));
        assert_eq!(display.set_passkey("1234567", "Keyboard", false), Err(PasskeyError::NotSixDigits));
        assert_eq!(display.passkey(), None);
        display.set_passkey("000001", "Keyboard", false).unwrap();
        assert_eq!(display.set_passkey("", "Keyboard", false), Err(PasskeyError::NotSixDigits));
        assert_eq!(display.passkey(), Some("000001"));
    }

    #[test]
    fn indicator_walks_across_the_digits_and_stops_at_both_ends() {
        let mut display = PasskeyDisplay::new();
        display.set_passkey("123456", "Keyboard", true).unwrap();
        assert_eq!(display.advance_indicator(), 1);
        assert_eq!(display.advance_indicator(), 2);
        assert_eq!(display.split_at_indicator(), Some(("12", "3456")));
        for _ in 0..10 {
            display.advance_indicator();
        }
        assert_eq!(display.indicator(), 6);
        assert_eq!(display.split_at_indicator(), Some(("123456", "")));
        assert_eq!(display.retreat_indicator(), 5);
        for _ in 0..9 {
            display.retreat_indicator();
        }
        assert_eq!(display.indicator(), 0);
    }

    #[test]
    fn indicator_does_nothing_without_a_passkey() {
        let mut display = PasskeyDisplay::new();
        assert_eq!(display.advance_indicator(), 0);
        assert_eq!(display.split_at_indicator(), None);
    }

    #[test]
    fn reset_returns_the_indicator_to_the_start_and_set_passkey_does_too() {
        let mut display = PasskeyDisplay::new();
        display.set_passkey("654321", "Phone", false).unwrap();
        display.advance_indicator();
        display.advance_indicator();
        display.advance_indicator();
        display.reset_indicator();
        assert_eq!(display.indicator(), 0);
        display.advance_indicator();
        display.set_passkey("654321", "Phone", false).unwrap();
        assert_eq!(display.indicator(), 0);
    }

    #[test]
    fn incoming_flag_is_stored() {
        let mut display = PasskeyDisplay::new();
        assert!(!display.is_incoming_request());
        display.set_incoming_request(true);
        assert!(display.is_incoming_request());
    }
}
