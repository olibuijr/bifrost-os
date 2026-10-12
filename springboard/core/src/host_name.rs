//! The computer name (the free text a person sees) and the local host name (the DNS label the machine answers to on
//! the local network), and the rule that derives one from the other.
//!
//! This is a pure model. It reads and writes no system file, sets no kernel host name and sends no advertisement.
//! A caller keeps a [`HostNames`], stores it in its own settings, and applies the local host name where it needs one.
//!
//! Rules of this model:
//! - A computer name is 1 to 255 bytes after trimming, with no control characters.
//! - A local host name is one DNS label: 1 to 63 ASCII letters, digits or hyphens, not starting or ending with a
//!   hyphen. It is stored in lower case, so `Desk` and `desk` are the same name.
//! - The derived name keeps ASCII letters and digits, turns every other character into a hyphen, collapses runs of
//!   hyphens, trims hyphens from both ends and cuts the result to 63 characters. When nothing is left, the default
//!   name is used.

use std::fmt;

/// The local host name used when a computer name has no ASCII letter or digit to build from.
pub const DEFAULT_LOCAL_HOST_NAME: &str = "bifrost";

/// The longest computer name, in bytes after trimming.
pub const MAX_COMPUTER_NAME_BYTES: usize = 255;

/// The longest local host name, in characters (one DNS label).
pub const MAX_LOCAL_HOST_NAME_LEN: usize = 63;

/// Why a name is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostNameError {
    /// The computer name is empty after trimming.
    EmptyComputerName,
    /// The computer name is longer than [`MAX_COMPUTER_NAME_BYTES`] bytes.
    ComputerNameTooLong(usize),
    /// The computer name holds a control character such as a line break.
    ControlCharacter,
    /// The local host name is empty.
    EmptyLocalHostName,
    /// The local host name is longer than [`MAX_LOCAL_HOST_NAME_LEN`] characters.
    LocalHostNameTooLong(usize),
    /// The local host name holds a character that is not an ASCII letter, digit or hyphen.
    InvalidCharacter(char),
    /// The local host name starts or ends with a hyphen.
    EdgeHyphen,
}

impl fmt::Display for HostNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostNameError::EmptyComputerName => write!(f, "the computer name is empty"),
            HostNameError::ComputerNameTooLong(n) => {
                write!(f, "the computer name is {n} bytes, the limit is {MAX_COMPUTER_NAME_BYTES}")
            }
            HostNameError::ControlCharacter => write!(f, "the computer name contains a control character"),
            HostNameError::EmptyLocalHostName => write!(f, "the local host name is empty"),
            HostNameError::LocalHostNameTooLong(n) => {
                write!(f, "the local host name is {n} characters, the limit is {MAX_LOCAL_HOST_NAME_LEN}")
            }
            HostNameError::InvalidCharacter(c) => {
                write!(f, "{c:?} is not allowed in a local host name (letters, digits and hyphens only)")
            }
            HostNameError::EdgeHyphen => write!(f, "a local host name cannot start or end with a hyphen"),
        }
    }
}

/// The free-text name of the computer, shown to people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputerName(String);

impl ComputerName {
    /// Trims the text and checks it against the rules of this module.
    pub fn new(name: &str) -> Result<ComputerName, HostNameError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HostNameError::EmptyComputerName);
        }
        if name.len() > MAX_COMPUTER_NAME_BYTES {
            return Err(HostNameError::ComputerNameTooLong(name.len()));
        }
        if name.chars().any(char::is_control) {
            return Err(HostNameError::ControlCharacter);
        }
        Ok(ComputerName(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A DNS label for the local network: lower-case ASCII letters, digits and hyphens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalHostName(String);

impl LocalHostName {
    /// Lower-cases the text, then checks it against the rules of this module.
    pub fn new(name: &str) -> Result<LocalHostName, HostNameError> {
        let name = name.to_ascii_lowercase();
        if name.is_empty() {
            return Err(HostNameError::EmptyLocalHostName);
        }
        let len = name.chars().count();
        if len > MAX_LOCAL_HOST_NAME_LEN {
            return Err(HostNameError::LocalHostNameTooLong(len));
        }
        if let Some(bad) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-')) {
            return Err(HostNameError::InvalidCharacter(bad));
        }
        if name.starts_with('-') || name.ends_with('-') {
            return Err(HostNameError::EdgeHyphen);
        }
        Ok(LocalHostName(name))
    }

    /// Derives the local host name from a computer name, as described in the module notes.
    pub fn from_computer_name(name: &ComputerName) -> LocalHostName {
        let mut out = String::new();
        for c in name.as_str().chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if !out.ends_with('-') {
                out.push('-');
            }
        }
        let trimmed = out.trim_matches('-');
        let cut: String = trimmed.chars().take(MAX_LOCAL_HOST_NAME_LEN).collect();
        let cut = cut.trim_end_matches('-');
        if cut.is_empty() {
            LocalHostName(DEFAULT_LOCAL_HOST_NAME.to_string())
        } else {
            LocalHostName(cut.to_string())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LocalHostName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The two names of the machine, kept in step by [`HostNames::set_computer_name`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostNames {
    computer: ComputerName,
    local: LocalHostName,
}

impl HostNames {
    /// Names the machine from a computer name; the local host name is derived from it.
    pub fn from_computer_name(name: &str) -> Result<HostNames, HostNameError> {
        let computer = ComputerName::new(name)?;
        let local = LocalHostName::from_computer_name(&computer);
        Ok(HostNames { computer, local })
    }

    pub fn computer_name(&self) -> &ComputerName {
        &self.computer
    }

    pub fn local_host_name(&self) -> &LocalHostName {
        &self.local
    }

    /// Changes the computer name and derives the local host name again.
    pub fn set_computer_name(&mut self, name: &str) -> Result<(), HostNameError> {
        let computer = ComputerName::new(name)?;
        self.local = LocalHostName::from_computer_name(&computer);
        self.computer = computer;
        Ok(())
    }

    /// Changes only the local host name; the computer name stays as it is.
    pub fn set_local_host_name(&mut self, name: &str) -> Result<(), HostNameError> {
        self.local = LocalHostName::new(name)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_name_is_trimmed_and_checked() {
        assert_eq!(ComputerName::new("  Olafs Desk  ").unwrap().as_str(), "Olafs Desk");
        assert_eq!(ComputerName::new("   "), Err(HostNameError::EmptyComputerName));
        assert_eq!(ComputerName::new("desk\nroom"), Err(HostNameError::ControlCharacter));
        let long = "a".repeat(256);
        assert_eq!(ComputerName::new(&long), Err(HostNameError::ComputerNameTooLong(256)));
        let max = "a".repeat(255);
        assert!(ComputerName::new(&max).is_ok());
    }

    #[test]
    fn local_host_name_is_derived_from_the_computer_name() {
        let name = ComputerName::new("Olafs Desk").unwrap();
        assert_eq!(LocalHostName::from_computer_name(&name).as_str(), "olafs-desk");
        // The apostrophe becomes a hyphen, the double space collapses into one hyphen and the exclamation mark ends
        // with a hyphen that is trimmed: "Olafs's  Desk!" -> "olafs-s-desk-" -> "olafs-s-desk".
        let punct = ComputerName::new("Olafs's  Desk!").unwrap();
        assert_eq!(LocalHostName::from_computer_name(&punct).as_str(), "olafs-s-desk");
        // Non-ASCII letters are not ASCII letters, so they become hyphens and the rest of the name survives.
        let accent = ComputerName::new("Ólafur").unwrap();
        assert_eq!(LocalHostName::from_computer_name(&accent).as_str(), "lafur");
        let none = ComputerName::new("!!!").unwrap();
        assert_eq!(LocalHostName::from_computer_name(&none).as_str(), DEFAULT_LOCAL_HOST_NAME);
    }

    #[test]
    fn derived_name_is_cut_to_one_label_without_a_trailing_hyphen() {
        // 62 letters, then a hyphen at position 63, then a letter: the cut at 63 leaves a trailing hyphen, which is
        // removed, so the result is the 62 letters.
        let text = format!("{}-b", "a".repeat(62));
        let name = ComputerName::new(&text).unwrap();
        let local = LocalHostName::from_computer_name(&name);
        assert_eq!(local.as_str(), "a".repeat(62));
        let long = ComputerName::new(&"c".repeat(70)).unwrap();
        assert_eq!(LocalHostName::from_computer_name(&long).as_str().len(), 63);
    }

    #[test]
    fn local_host_name_checks_the_dns_label_rules() {
        assert_eq!(LocalHostName::new("Desk").unwrap().as_str(), "desk");
        assert_eq!(LocalHostName::new("a_b"), Err(HostNameError::InvalidCharacter('_')));
        assert_eq!(LocalHostName::new("-desk"), Err(HostNameError::EdgeHyphen));
        assert_eq!(LocalHostName::new("desk-"), Err(HostNameError::EdgeHyphen));
        assert_eq!(LocalHostName::new(""), Err(HostNameError::EmptyLocalHostName));
        assert!(LocalHostName::new(&"d".repeat(63)).is_ok());
        assert_eq!(LocalHostName::new(&"d".repeat(64)), Err(HostNameError::LocalHostNameTooLong(64)));
    }

    #[test]
    fn setting_the_computer_name_moves_the_local_name_with_it() {
        let mut names = HostNames::from_computer_name("Olafs Desk").unwrap();
        assert_eq!(names.local_host_name().as_str(), "olafs-desk");
        names.set_computer_name("Living Room").unwrap();
        assert_eq!(names.local_host_name().as_str(), "living-room");
        assert_eq!(names.computer_name().as_str(), "Living Room");
    }

    #[test]
    fn a_refused_name_leaves_both_names_unchanged() {
        let mut names = HostNames::from_computer_name("Olafs Desk").unwrap();
        names.set_local_host_name("office").unwrap();
        assert!(names.set_local_host_name("bad name").is_err());
        assert!(names.set_computer_name("   ").is_err());
        assert_eq!(names.local_host_name().as_str(), "office");
        assert_eq!(names.computer_name().as_str(), "Olafs Desk");
    }
}
