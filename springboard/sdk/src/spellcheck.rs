//! `NativeSpellChecker` parity for apps. The model lives in `bifrost_core::spellcheck`, which the shell uses too;
//! this module re-exports it and keeps the learned words in `UserDefaults`.

use crate::defaults::UserDefaults;
pub use bifrost_core::spellcheck::*;

impl KeyValues for UserDefaults {
    fn get_string(&self, key: &str) -> Option<String> {
        UserDefaults::string(self, key)
    }
    fn put_string(&mut self, key: &str, value: &str) {
        UserDefaults::set_string(self, key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_words_reach_a_new_checker_through_user_defaults() {
        let mut d = UserDefaults::memory();
        let mut c = SpellChecker::new(Box::new(WordList::new("en", ["hello"])));
        c.learn_word("wrold", &mut d);
        let mut c2 = SpellChecker::new(Box::new(WordList::new("en", ["hello"])));
        assert!(!c2.has_learned_word("wrold"));
        c2.load(&d);
        assert!(c2.has_learned_word("WROLD"));
    }
}
