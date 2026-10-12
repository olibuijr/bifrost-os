//! Named search indexes and their descriptions (`CSSearchableIndex(name:protectionClass:)` and
//! `CSSearchableIndexDescription`).
//!
//! An app can keep more than one index: the default index and named ones, each with its own data protection class.
//! A description is the index name plus that class. A name becomes part of a file name in the shell's store, so only
//! ASCII letters, digits, `-`, `_` and an inner `.` are accepted, up to [`MAX_NAME`] bytes, and a name may not start
//! with `.`.
//!
//! Pure model: the shell does not keep named indexes yet, so items still land in the app's default store and the
//! protection class is recorded here only. Gaps: no index delegate or batch state per index, and no protection applied
//! to the store file on disk.

/// Longest accepted index name, in bytes.
pub const MAX_NAME: usize = 64;

/// The data protection classes of an index (`File Protection Type`), with their raw names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectionClass {
    Complete,
    CompleteUnlessOpen,
    CompleteUntilFirstUserAuthentication,
    None,
}

impl ProtectionClass {
    /// The raw name, used in logs and in the store's metadata.
    pub fn raw(self) -> &'static str {
        match self {
            ProtectionClass::Complete => "complete",
            ProtectionClass::CompleteUnlessOpen => "completeUnlessOpen",
            ProtectionClass::CompleteUntilFirstUserAuthentication => "completeUntilFirstUserAuthentication",
            ProtectionClass::None => "none",
        }
    }
}

/// Why an index name is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    /// The name is empty.
    Empty,
    /// The name is longer than [`MAX_NAME`] bytes.
    TooLong,
    /// The name holds a character outside the accepted set, or starts with `.`. Carries the first such character.
    BadCharacter(char),
}

/// Checks an index name. The length is checked first, then the first character, then each character in order.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > MAX_NAME {
        return Err(NameError::TooLong);
    }
    if name.starts_with('.') {
        return Err(NameError::BadCharacter('.'));
    }
    match name.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))) {
        Some(c) => Err(NameError::BadCharacter(c)),
        None => Ok(()),
    }
}

/// `CSSearchableIndexDescription`: the name of a named index and its protection class, if one was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexDescription {
    pub name: String,
    pub protection_class: Option<ProtectionClass>,
}

impl IndexDescription {
    /// A description for the index `name`, with no protection class. Refuses an invalid name.
    pub fn new(name: &str) -> Result<Self, NameError> {
        validate_name(name)?;
        Ok(IndexDescription { name: name.into(), protection_class: None })
    }

    /// Sets the protection class (`protectionClass`).
    pub fn protection(mut self, class: ProtectionClass) -> Self {
        self.protection_class = Some(class);
        self
    }
}

/// The store file name for an app's index: `<app>.json` for the default index and `<app>@<name>.json` for a named one.
pub fn store_file_name(app_id: &str, index: Option<&IndexDescription>) -> String {
    match index {
        None => format!("{app_id}.json"),
        Some(d) => format!("{app_id}@{}.json", d.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_accept_letters_digits_dash_underscore_and_inner_dots() {
        assert_eq!(validate_name("notes-2_archive.v1"), Ok(()));
        assert_eq!(validate_name("a"), Ok(()));
    }

    #[test]
    fn names_reject_empty_long_and_bad_characters() {
        assert_eq!(validate_name(""), Err(NameError::Empty));
        assert_eq!(validate_name(&"a".repeat(MAX_NAME)), Ok(()));
        assert_eq!(validate_name(&"a".repeat(MAX_NAME + 1)), Err(NameError::TooLong));
        assert_eq!(validate_name("a/b"), Err(NameError::BadCharacter('/')));
        assert_eq!(validate_name(".hidden"), Err(NameError::BadCharacter('.')));
        assert_eq!(validate_name("caf\u{e9}"), Err(NameError::BadCharacter('\u{e9}')));
    }

    #[test]
    fn protection_class_raw_names_match_the_data_protection_names() {
        assert_eq!(ProtectionClass::Complete.raw(), "complete");
        assert_eq!(ProtectionClass::CompleteUnlessOpen.raw(), "completeUnlessOpen");
        assert_eq!(ProtectionClass::CompleteUntilFirstUserAuthentication.raw(), "completeUntilFirstUserAuthentication");
        assert_eq!(ProtectionClass::None.raw(), "none");
    }

    #[test]
    fn description_keeps_the_name_and_the_optional_class() {
        let plain = IndexDescription::new("archive").unwrap();
        assert_eq!(plain.name, "archive");
        assert_eq!(plain.protection_class, None);
        let guarded = IndexDescription::new("archive").unwrap().protection(ProtectionClass::CompleteUnlessOpen);
        assert_eq!(guarded.protection_class, Some(ProtectionClass::CompleteUnlessOpen));
        assert_eq!(IndexDescription::new(""), Err(NameError::Empty));
    }

    #[test]
    fn store_file_names_separate_the_default_and_named_indexes() {
        assert_eq!(store_file_name("bifrost.notes", None), "bifrost.notes.json");
        let archive = IndexDescription::new("archive").unwrap();
        assert_eq!(store_file_name("bifrost.notes", Some(&archive)), "bifrost.notes@archive.json");
    }
}
