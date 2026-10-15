//! The switches of the Spelling and Grammar, Substitutions and Data Detectors menus. They are kept in the app's defaults,
//! so they stay across launches. A switch the defaults have never stored takes its built-in value.

use bifrost_app::controls::detect;
use bifrost_app::defaults::UserDefaults;

const CHECK_WHILE_TYPING: &str = "stickies.spelling.check-while-typing";
const CORRECT_AUTOMATICALLY: &str = "stickies.spelling.correct-automatically";
const SMART_QUOTES: &str = "stickies.substitutions.smart-quotes";
const SMART_DASHES: &str = "stickies.substitutions.smart-dashes";
const DETECT_DATES: &str = "stickies.detect.dates";
const DETECT_ADDRESSES: &str = "stickies.detect.addresses";
const DETECT_PHONES: &str = "stickies.detect.phone-numbers";
const FIND_IGNORE_CASE: &str = "stickies.find.ignore-case";
const FIND_WHOLE_WORD: &str = "stickies.find.whole-word";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// Misspelled words are marked as they are typed. On by default, as in the reference app.
    pub check_while_typing: bool,
    /// A misspelled word is replaced by the checker's correction when a space or punctuation follows it.
    pub correct_automatically: bool,
    pub smart_quotes: bool,
    pub smart_dashes: bool,
    pub detect_dates: bool,
    pub detect_addresses: bool,
    pub detect_phone_numbers: bool,
    /// Find and Replace match letters of either case. On by default, as in the reference app.
    pub find_ignore_case: bool,
    /// Find and Replace match only whole words. Off by default.
    pub find_whole_word: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            check_while_typing: true,
            correct_automatically: false,
            smart_quotes: false,
            smart_dashes: false,
            detect_dates: false,
            detect_addresses: false,
            detect_phone_numbers: false,
            find_ignore_case: true,
            find_whole_word: false,
        }
    }
}

impl Prefs {
    /// Reads the switches from the app's defaults.
    pub fn load(store: &UserDefaults) -> Prefs {
        let flag = |key: &str, built_in: bool| if store.contains(key) { store.bool(key) } else { built_in };
        let built_in = Prefs::default();
        Prefs {
            check_while_typing: flag(CHECK_WHILE_TYPING, built_in.check_while_typing),
            correct_automatically: flag(CORRECT_AUTOMATICALLY, built_in.correct_automatically),
            smart_quotes: flag(SMART_QUOTES, built_in.smart_quotes),
            smart_dashes: flag(SMART_DASHES, built_in.smart_dashes),
            detect_dates: flag(DETECT_DATES, built_in.detect_dates),
            detect_addresses: flag(DETECT_ADDRESSES, built_in.detect_addresses),
            detect_phone_numbers: flag(DETECT_PHONES, built_in.detect_phone_numbers),
            find_ignore_case: flag(FIND_IGNORE_CASE, built_in.find_ignore_case),
            find_whole_word: flag(FIND_WHOLE_WORD, built_in.find_whole_word),
        }
    }

    /// Writes every switch to the app's defaults, which keeps them in the app's data directory.
    pub fn save(&self, store: &mut UserDefaults) {
        store.set_bool(CHECK_WHILE_TYPING, self.check_while_typing);
        store.set_bool(CORRECT_AUTOMATICALLY, self.correct_automatically);
        store.set_bool(SMART_QUOTES, self.smart_quotes);
        store.set_bool(SMART_DASHES, self.smart_dashes);
        store.set_bool(DETECT_DATES, self.detect_dates);
        store.set_bool(DETECT_ADDRESSES, self.detect_addresses);
        store.set_bool(DETECT_PHONES, self.detect_phone_numbers);
        store.set_bool(FIND_IGNORE_CASE, self.find_ignore_case);
        store.set_bool(FIND_WHOLE_WORD, self.find_whole_word);
    }

    /// The kinds of item the data detectors mark, as the switches choose.
    pub fn detector_kinds(&self) -> Vec<detect::Kind> {
        let mut kinds = Vec::new();
        if self.detect_dates {
            kinds.push(detect::Kind::Date);
        }
        if self.detect_addresses {
            kinds.push(detect::Kind::Address);
        }
        if self.detect_phone_numbers {
            kinds.push(detect::Kind::Phone);
        }
        kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_gives_the_built_in_switches() {
        let store = UserDefaults::memory();
        assert_eq!(Prefs::load(&store), Prefs::default());
        assert!(Prefs::default().check_while_typing, "spelling is checked from the start");
        assert!(Prefs::default().detector_kinds().is_empty());
        assert!(Prefs::default().find_ignore_case, "find ignores case from the start");
        assert!(!Prefs::default().find_whole_word, "find matches inside words from the start");
    }

    #[test]
    fn switches_round_trip_through_the_defaults() {
        let mut store = UserDefaults::memory();
        let on = Prefs { check_while_typing: false, smart_quotes: true, detect_dates: true, detect_phone_numbers: true, find_ignore_case: false, find_whole_word: true, ..Prefs::default() };
        on.save(&mut store);
        assert_eq!(Prefs::load(&store), on);
        assert_eq!(on.detector_kinds(), vec![detect::Kind::Date, detect::Kind::Phone]);
    }
}
