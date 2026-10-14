//! The Info dictionary of an input method bundle, read into the [`Registration`] record that [`Server`] checks.
//!
//! The keys read are `IMKControllerClass` (a string, required), `IMKDelegateClass` (a string, optional) and
//! `IMKModeDictionary` (a dictionary from mode id to a settings dictionary, in document order, so the first mode is
//! the default). A mode's label is the `label` string of its settings when present, otherwise the mode id. Other keys
//! are ignored. The label key is a BifrOSt choice, not an Apple key.
//!
//! Pure model: the property list is parsed by `bifrost_core::plist`; nothing reads a bundle from disk here.
//!
//! [`Server`]: crate::input_method::Server

use bifrost_core::plist::Plist;

use crate::input_method::{Registration, RegistrationError};

/// The Info dictionary key of the controller class the server creates for each session.
pub const CONTROLLER_KEY: &str = "IMKControllerClass";
/// The Info dictionary key of the optional delegate class.
pub const DELEGATE_KEY: &str = "IMKDelegateClass";
/// The Info dictionary key of the mode dictionary.
pub const MODE_DICTIONARY_KEY: &str = "IMKModeDictionary";
/// The key, inside a mode's settings, that holds the label shown for the mode.
pub const MODE_LABEL_KEY: &str = "label";

/// Why an Info dictionary does not describe a usable input method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoError {
    /// The root value is not a dictionary.
    NotADictionary,
    /// `IMKControllerClass` is missing or not a string.
    MissingController,
    /// `IMKDelegateClass` is present but not a string.
    DelegateNotString,
    /// `IMKModeDictionary` is present but not a dictionary.
    ModesNotDictionary,
    /// The settings of the named mode are not a dictionary.
    ModeNotDictionary(String),
    /// The record parsed but failed [`Registration::validate`].
    Invalid(RegistrationError),
}

fn lookup<'a>(entries: &'a [(String, Plist)], key: &str) -> Option<&'a Plist> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Reads an input method's Info dictionary into a validated [`Registration`].
pub fn registration_from_info(info: &Plist) -> Result<Registration, InfoError> {
    let entries = match info {
        Plist::Dict(entries) => entries,
        _ => return Err(InfoError::NotADictionary),
    };
    let controller = match lookup(entries, CONTROLLER_KEY) {
        Some(Plist::Str(name)) => name.as_str(),
        _ => return Err(InfoError::MissingController),
    };
    let mut registration = Registration::new(controller);
    if let Some(value) = lookup(entries, DELEGATE_KEY) {
        match value {
            Plist::Str(name) => registration = registration.delegate(name),
            _ => return Err(InfoError::DelegateNotString),
        }
    }
    if let Some(value) = lookup(entries, MODE_DICTIONARY_KEY) {
        let modes = match value {
            Plist::Dict(modes) => modes,
            _ => return Err(InfoError::ModesNotDictionary),
        };
        for (id, settings) in modes {
            let fields = match settings {
                Plist::Dict(fields) => fields,
                _ => return Err(InfoError::ModeNotDictionary(id.clone())),
            };
            let label = match lookup(fields, MODE_LABEL_KEY) {
                Some(Plist::Str(label)) => label.as_str(),
                _ => id.as_str(),
            };
            registration = registration.mode(id, label);
        }
    }
    registration.validate().map_err(InfoError::Invalid)?;
    Ok(registration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_method::Server;

    fn text(value: &str) -> Plist {
        Plist::Str(value.to_string())
    }

    fn dict(entries: Vec<(&str, Plist)>) -> Plist {
        Plist::Dict(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn modes(entries: Vec<(&str, Plist)>) -> Plist {
        dict(entries)
    }

    fn full_info() -> Plist {
        dict(vec![
            (CONTROLLER_KEY, text("PinyinController")),
            (DELEGATE_KEY, text("PinyinDelegate")),
            (
                MODE_DICTIONARY_KEY,
                modes(vec![
                    ("zh-pinyin", dict(vec![(MODE_LABEL_KEY, text("Pinyin"))])),
                    ("ja-romaji", dict(vec![])),
                ]),
            ),
            ("CFBundleIdentifier", text("org.example.ime")),
        ])
    }

    #[test]
    fn reads_controller_delegate_and_modes_in_document_order() {
        let registration = registration_from_info(&full_info()).unwrap();
        assert_eq!(registration.controller_class(), "PinyinController");
        assert_eq!(registration.delegate_class(), Some("PinyinDelegate"));
        let ids: Vec<&str> = registration.modes().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["zh-pinyin", "ja-romaji"]);
        assert_eq!(registration.default_mode().map(|m| m.id.as_str()), Some("zh-pinyin"));
        assert_eq!(registration.modes()[0].label, "Pinyin");
        assert_eq!(registration.modes()[1].label, "ja-romaji", "a mode without a label is shown by its id");
    }

    #[test]
    fn a_label_that_is_not_a_string_falls_back_to_the_id() {
        let info = dict(vec![
            (CONTROLLER_KEY, text("C")),
            (MODE_DICTIONARY_KEY, modes(vec![("a", dict(vec![(MODE_LABEL_KEY, Plist::Int(3))]))])),
        ]);
        let registration = registration_from_info(&info).unwrap();
        assert_eq!(registration.modes()[0].label, "a");
    }

    #[test]
    fn blank_delegate_means_no_delegate() {
        let info = dict(vec![
            (CONTROLLER_KEY, text("C")),
            (DELEGATE_KEY, text("   ")),
            (MODE_DICTIONARY_KEY, modes(vec![("a", dict(vec![]))])),
        ]);
        assert_eq!(registration_from_info(&info).unwrap().delegate_class(), None);
    }

    #[test]
    fn root_must_be_a_dictionary() {
        assert_eq!(registration_from_info(&text("C")).err(), Some(InfoError::NotADictionary));
        assert_eq!(registration_from_info(&Plist::Array(vec![])).err(), Some(InfoError::NotADictionary));
    }

    #[test]
    fn controller_is_required_and_must_be_a_string() {
        assert_eq!(registration_from_info(&dict(vec![])).err(), Some(InfoError::MissingController));
        let not_text = dict(vec![(CONTROLLER_KEY, Plist::Int(1))]);
        assert_eq!(registration_from_info(&not_text).err(), Some(InfoError::MissingController));
        let blank = dict(vec![(CONTROLLER_KEY, text("  "))]);
        assert_eq!(registration_from_info(&blank).err(), Some(InfoError::Invalid(RegistrationError::NoController)));
    }

    #[test]
    fn delegate_must_be_a_string_when_present() {
        let info = dict(vec![(CONTROLLER_KEY, text("C")), (DELEGATE_KEY, Plist::Bool(true))]);
        assert_eq!(registration_from_info(&info).err(), Some(InfoError::DelegateNotString));
    }

    #[test]
    fn modes_must_be_dictionaries_and_be_present() {
        let not_dict = dict(vec![(CONTROLLER_KEY, text("C")), (MODE_DICTIONARY_KEY, text("a"))]);
        assert_eq!(registration_from_info(&not_dict).err(), Some(InfoError::ModesNotDictionary));
        let bad_settings = dict(vec![
            (CONTROLLER_KEY, text("C")),
            (MODE_DICTIONARY_KEY, modes(vec![("zh", text("Pinyin"))])),
        ]);
        assert_eq!(registration_from_info(&bad_settings).err(), Some(InfoError::ModeNotDictionary("zh".to_string())));
        let no_modes = dict(vec![(CONTROLLER_KEY, text("C"))]);
        assert_eq!(registration_from_info(&no_modes).err(), Some(InfoError::Invalid(RegistrationError::NoModes)));
    }

    #[test]
    fn duplicate_mode_ids_are_refused_by_validation() {
        let info = dict(vec![
            (CONTROLLER_KEY, text("C")),
            (MODE_DICTIONARY_KEY, modes(vec![("a", dict(vec![])), ("a", dict(vec![]))])),
        ]);
        assert_eq!(
            registration_from_info(&info).err(),
            Some(InfoError::Invalid(RegistrationError::DuplicateMode("a".to_string())))
        );
    }

    #[test]
    fn parsed_record_starts_a_server() {
        let registration = registration_from_info(&full_info()).unwrap();
        let mut server = Server::new(registration).unwrap();
        assert!(server.connect(1).is_ok());
        assert_eq!(server.clients(), vec![1]);
    }
}
