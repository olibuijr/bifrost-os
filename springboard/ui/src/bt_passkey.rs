//! Passkey display: the model behind the panel that shows a six-digit pairing passkey, so the user can compare
//! it with the number on the other device or type it there. It follows the shape of
//! the reference passkey display: `setPasskey(_:for:usingSSP:)` (a passkey, whether the request came from the
//! other device, and whether it is a numeric comparison), the passkey itself, and advance, retreat and reset
//! of the indicator that marks how many digits have been entered on the other device.
//! Renderer-independent: the shell draws the digits and the indicator.

/// A passkey has six decimal digits.
pub const PASSKEY_DIGITS: usize = 6;
/// The largest value a passkey can take (999999).
pub const PASSKEY_MAX: u32 = 999_999;

/// "123 456": the passkey with a space after the third digit. None when the value has more than six digits.
pub fn format_passkey(value: u32) -> Option<String> {
    if value > PASSKEY_MAX {
        return None;
    }
    Some(format!("{:03} {:03}", value / 1000, value % 1000))
}

/// Reads a typed passkey. Spaces are ignored; the rest must be exactly six ASCII digits.
pub fn parse_entered(text: &str) -> Option<u32> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() != PASSKEY_DIGITS || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasskeyDisplay {
    passkey: Option<u32>,
    incoming: bool,
    ssp: bool,
    /// How many digits the other device has confirmed, 0 to `PASSKEY_DIGITS`.
    indicator: usize,
}

impl PasskeyDisplay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows a passkey and restarts the indicator. `incoming` means the other device started the pairing.
    /// `ssp` means a numeric comparison (both sides show the same number). Returns false, and keeps the
    /// previous state, when the value is above `PASSKEY_MAX`.
    pub fn set_passkey(&mut self, value: u32, incoming: bool, ssp: bool) -> bool {
        if value > PASSKEY_MAX {
            return false;
        }
        self.passkey = Some(value);
        self.incoming = incoming;
        self.ssp = ssp;
        self.indicator = 0;
        true
    }

    pub fn passkey(&self) -> Option<u32> {
        self.passkey
    }

    /// The passkey as shown to the user, "123 456".
    pub fn text(&self) -> Option<String> {
        self.passkey.and_then(format_passkey)
    }

    pub fn is_incoming_request(&self) -> bool {
        self.incoming
    }

    pub fn uses_ssp(&self) -> bool {
        self.ssp
    }

    pub fn indicator(&self) -> usize {
        self.indicator
    }

    /// One more digit confirmed on the other device. Stops at `PASSKEY_DIGITS`.
    pub fn advance_indicator(&mut self) {
        if self.indicator < PASSKEY_DIGITS {
            self.indicator += 1;
        }
    }

    /// One digit taken back. Stops at zero.
    pub fn retreat_indicator(&mut self) {
        self.indicator = self.indicator.saturating_sub(1);
    }

    pub fn reset_indicator(&mut self) {
        self.indicator = 0;
    }

    /// True when a typed passkey equals the shown one. Nothing shown, or text that is not six digits, is false.
    pub fn matches(&self, entered: &str) -> bool {
        match (self.passkey, parse_entered(entered)) {
            (Some(shown), Some(typed)) => shown == typed,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passkey_is_shown_with_a_space_after_three_digits() {
        assert_eq!(format_passkey(123456).as_deref(), Some("123 456"));
        assert_eq!(format_passkey(42).as_deref(), Some("000 042"));
        assert_eq!(format_passkey(999999).as_deref(), Some("999 999"));
        assert_eq!(format_passkey(1_000_000), None, "seven digits do not fit");
    }

    #[test]
    fn a_typed_passkey_needs_exactly_six_digits() {
        assert_eq!(parse_entered("123 456"), Some(123456));
        assert_eq!(parse_entered(" 000042 "), Some(42));
        assert_eq!(parse_entered("12345"), None, "five digits");
        assert_eq!(parse_entered("1234567"), None, "seven digits");
        assert_eq!(parse_entered("12a456"), None, "a letter is not a digit");
    }

    #[test]
    fn a_value_above_the_maximum_is_refused_and_leaves_the_display_alone() {
        let mut d = PasskeyDisplay::new();
        assert!(d.set_passkey(123456, true, true));
        assert!(!d.set_passkey(1_000_000, false, false));
        assert_eq!(d.text().as_deref(), Some("123 456"));
        assert!(d.is_incoming_request());
        assert!(d.uses_ssp());
    }

    #[test]
    fn the_indicator_stops_at_six_and_at_zero() {
        let mut d = PasskeyDisplay::new();
        for _ in 0..7 {
            d.advance_indicator();
        }
        assert_eq!(d.indicator(), 6);
        d.retreat_indicator();
        d.retreat_indicator();
        assert_eq!(d.indicator(), 4);
        for _ in 0..9 {
            d.retreat_indicator();
        }
        assert_eq!(d.indicator(), 0);
        d.advance_indicator();
        d.reset_indicator();
        assert_eq!(d.indicator(), 0);
    }

    #[test]
    fn showing_a_new_passkey_restarts_the_indicator() {
        let mut d = PasskeyDisplay::new();
        assert_eq!(d.passkey(), None);
        d.advance_indicator();
        d.advance_indicator();
        assert!(d.set_passkey(5, false, false));
        assert_eq!(d.indicator(), 0);
        assert_eq!(d.passkey(), Some(5));
        assert!(!d.is_incoming_request());
        assert!(!d.uses_ssp());
    }

    #[test]
    fn a_typed_passkey_is_checked_against_the_shown_one() {
        let mut d = PasskeyDisplay::new();
        assert!(!d.matches("123456"), "nothing shown yet");
        d.set_passkey(123456, false, true);
        assert!(d.matches("123 456"));
        assert!(!d.matches("123455"));
        assert!(!d.matches("12"), "an incomplete entry never matches");
    }
}
