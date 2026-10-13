//! Autofill content types for text fields: what a field expects, so the system can offer the right suggestion.
//!
//! A field declares one [`ContentType`]. The `OneTimeCode` type is for a code sent by SMS or an authenticator app:
//! a suggestion for it is a run of 4 to 8 digits, and [`ContentType::accepts`] checks a typed or suggested value
//! against that rule. The wire names are lowercase and hyphenated so they can travel in a manifest or a shell
//! message without a mapping table.

/// What a text field holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentType {
    Username,
    Password,
    NewPassword,
    /// A one-time code from a text message or an authenticator app.
    OneTimeCode,
}

/// Shortest and longest code accepted for [`ContentType::OneTimeCode`], in digits.
pub const CODE_DIGITS: std::ops::RangeInclusive<usize> = 4..=8;

impl ContentType {
    /// The lowercase hyphenated name used on the wire.
    pub fn wire_name(self) -> &'static str {
        match self {
            ContentType::Username => "username",
            ContentType::Password => "password",
            ContentType::NewPassword => "new-password",
            ContentType::OneTimeCode => "one-time-code",
        }
    }

    /// The content type for a wire name, or `None` when the name is not known.
    pub fn from_wire_name(name: &str) -> Option<ContentType> {
        match name {
            "username" => Some(ContentType::Username),
            "password" => Some(ContentType::Password),
            "new-password" => Some(ContentType::NewPassword),
            "one-time-code" => Some(ContentType::OneTimeCode),
            _ => None,
        }
    }

    /// True when `value` is acceptable for this content type. Only one-time codes have a rule: surrounding
    /// whitespace is ignored and the rest must be ASCII digits, 4 to 8 of them. Other types accept any value.
    pub fn accepts(self, value: &str) -> bool {
        match self {
            ContentType::OneTimeCode => {
                let code = value.trim();
                CODE_DIGITS.contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit())
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip() {
        for content in [
            ContentType::Username,
            ContentType::Password,
            ContentType::NewPassword,
            ContentType::OneTimeCode,
        ] {
            assert_eq!(ContentType::from_wire_name(content.wire_name()), Some(content));
        }
        assert_eq!(ContentType::from_wire_name("one_time_code"), None);
        assert_eq!(ContentType::OneTimeCode.wire_name(), "one-time-code");
    }

    #[test]
    fn a_one_time_code_is_four_to_eight_digits() {
        assert!(ContentType::OneTimeCode.accepts("1234"));
        assert!(ContentType::OneTimeCode.accepts(" 987654 "));
        assert!(ContentType::OneTimeCode.accepts("12345678"));
        assert!(!ContentType::OneTimeCode.accepts("123"));
        assert!(!ContentType::OneTimeCode.accepts("123456789"));
        assert!(!ContentType::OneTimeCode.accepts("12a4"));
        assert!(!ContentType::OneTimeCode.accepts(""));
    }

    #[test]
    fn other_content_types_accept_any_value() {
        assert!(ContentType::Password.accepts("anything at all"));
        assert!(ContentType::Username.accepts(""));
    }
}
