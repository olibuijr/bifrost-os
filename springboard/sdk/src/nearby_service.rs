//! Nearby service type: the name an app advertises and browses under, checked before anything is published.
//!
//! [`ServiceType::new`] accepts a name made of lowercase ASCII letters, digits and hyphens. It is 1 to
//! [`MAX_LEN`] bytes long, has at least one letter, does not start or end with a hyphen, and has no two hyphens in a
//! row. These are the rules for a Bonjour service name without its leading underscore, as RFC 6335 section 5.1 states
//! them, except that uppercase letters are refused here so that every name has one spelling.
//!
//! This is the pure model. Nothing is advertised or browsed by this module: the shell's nearby beacon does not yet
//! take an app's service type, so the check is ready for that API and is not called by the shell today.

/// Longest service type, in bytes.
pub const MAX_LEN: usize = 15;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServiceType(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Empty,
    TooLong,
    BadCharacter,
    NoLetter,
    HyphenAtEdge,
    DoubleHyphen,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::Empty => "service type is empty",
            Error::TooLong => "service type is longer than 15 bytes",
            Error::BadCharacter => "service type may hold only lowercase letters, digits and hyphens",
            Error::NoLetter => "service type needs at least one letter",
            Error::HyphenAtEdge => "service type may not start or end with a hyphen",
            Error::DoubleHyphen => "service type may not contain two hyphens in a row",
        };
        f.write_str(text)
    }
}

impl ServiceType {
    /// Checks `name` and keeps it when every rule holds. The first failing rule is reported.
    pub fn new(name: &str) -> Result<ServiceType, Error> {
        if name.is_empty() {
            return Err(Error::Empty);
        }
        if name.len() > MAX_LEN {
            return Err(Error::TooLong);
        }
        if !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err(Error::BadCharacter);
        }
        if !name.bytes().any(|b| b.is_ascii_lowercase()) {
            return Err(Error::NoLetter);
        }
        if name.starts_with('-') || name.ends_with('-') {
            return Err(Error::HyphenAtEdge);
        }
        if name.contains("--") {
            return Err(Error::DoubleHyphen);
        }
        Ok(ServiceType(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_letters_digits_and_inner_hyphens() {
        assert_eq!(ServiceType::new("my-chat").unwrap().as_str(), "my-chat");
        assert_eq!(ServiceType::new("a-1").unwrap().as_str(), "a-1");
        // Exactly 15 bytes is still allowed.
        assert!(ServiceType::new("abcdefghijklmno").is_ok());
    }

    #[test]
    fn length_is_checked_before_the_characters() {
        assert_eq!(ServiceType::new(""), Err(Error::Empty));
        // 16 bytes, one over the limit.
        assert_eq!(ServiceType::new("abcdefghijklmnop"), Err(Error::TooLong));
    }

    #[test]
    fn uppercase_and_other_characters_are_refused() {
        assert_eq!(ServiceType::new("Chat"), Err(Error::BadCharacter));
        assert_eq!(ServiceType::new("_chat"), Err(Error::BadCharacter));
        assert_eq!(ServiceType::new("chat app"), Err(Error::BadCharacter));
    }

    #[test]
    fn needs_a_letter_and_no_hyphen_at_an_edge_or_doubled() {
        assert_eq!(ServiceType::new("1234"), Err(Error::NoLetter));
        assert_eq!(ServiceType::new("-chat"), Err(Error::HyphenAtEdge));
        assert_eq!(ServiceType::new("chat-"), Err(Error::HyphenAtEdge));
        assert_eq!(ServiceType::new("a--b"), Err(Error::DoubleHyphen));
    }
}
