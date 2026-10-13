//! A built-in compose method, the simple input method every text input runs without a provider process. A typed apostrophe
//! arms it; the next letter that has accented alternates opens the candidate list for that letter (`'a` offers
//! a, a-acute, a-grave, ...). Digits 1 to 9 pick a candidate. Any other key ends the composition and keeps the letter.
//! Icelandic letters (acute vowels, o-umlaut, ae, eth, thorn) are in the table.
//!
//! `Composer` is the state one text input keeps (the armed apostrophe and the open list). `type_text` and
//! `end_for_control` run it over a `ComposeTarget`, the field or view's own marked text and insertion, so the shell's
//! Search field and every SDK text field and text view share one flow (`text::TextEditor` is the usual target).
//! The model draws nothing: the caller paints the candidate list.

use crate::candidates::PanelType;
use crate::imsession::InputSession;

/// The key that arms the method (a plain apostrophe, which is never a letter in the table).
pub const PREFIX: char = '\'';

/// The most candidates one list shows; digits 1 to 9 pick among them.
pub const MAX_SHOWN: usize = 9;

/// Lowercase alternates for a letter, most used first. Empty for a letter with none.
fn alternates(c: char) -> &'static str {
    match c {
        'a' => "áàâäãåæ",
        'e' => "éèêë",
        'i' => "íìîï",
        'o' => "óòôöõø",
        'u' => "úùûü",
        'y' => "ýÿ",
        'c' => "ç",
        'n' => "ñ",
        'd' => "ð",
        't' => "þ",
        _ => "",
    }
}

/// The candidates for letter `c`: its alternates, upper-cased when `c` is upper case. Empty when there are none.
pub fn candidates_for(c: char) -> Vec<String> {
    let upper = c.is_uppercase();
    let lower = if upper { c.to_lowercase().next().unwrap_or(c) } else { c };
    alternates(lower)
        .chars()
        .map(|a| if upper { a.to_uppercase().collect::<String>() } else { a.to_string() })
        .collect()
}

/// The index picked by a digit key: "1" is 0 up to "9" is 8, when the list has that many candidates.
/// Zero, other keys and indexes past the end pick nothing.
pub fn pick(candidates: &[String], key: &str) -> Option<usize> {
    let mut chars = key.chars();
    let (Some(d), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let n = d.to_digit(10)? as usize;
    if n == 0 {
        return None;
    }
    let i = n - 1;
    (i < candidates.len().min(MAX_SHOWN)).then_some(i)
}

/// What a typed key means to the method.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Plain text to insert as typed.
    Insert(String),
    /// The apostrophe was typed: nothing is inserted yet.
    Armed,
    /// A letter with alternates: `original` is the letter, `candidates` the list (at most [`MAX_SHOWN`]).
    Open { original: String, candidates: Vec<String> },
}

/// The compose state: whether the apostrophe is waiting for its letter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compose {
    armed: bool,
}

impl Compose {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// Clears the armed state; true when it was armed (the caller then inserts the apostrophe it held back).
    pub fn take_prefix(&mut self) -> bool {
        std::mem::replace(&mut self.armed, false)
    }

    /// Feeds one typed text (a single key, or a paste) and says what it means.
    pub fn key(&mut self, text: &str) -> Step {
        let was_armed = self.take_prefix();
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        match single {
            Some(PREFIX) if !was_armed => {
                self.armed = true;
                Step::Armed
            }
            Some(c) if was_armed => {
                let candidates = candidates_for(c);
                if candidates.is_empty() {
                    Step::Insert(format!("{PREFIX}{c}"))
                } else {
                    let candidates = candidates.into_iter().take(MAX_SHOWN).collect();
                    Step::Open { original: c.to_string(), candidates }
                }
            }
            _ if was_armed => Step::Insert(format!("{PREFIX}{text}")),
            _ => Step::Insert(text.to_string()),
        }
    }
}

/// The composition state of one text input: the armed apostrophe and the open candidate list, if any.
#[derive(Default)]
pub struct Composer {
    method: Compose,
    session: Option<InputSession>,
}

impl Composer {
    /// True while a candidate list is open over the composed letter.
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// True while an apostrophe waits for its letter.
    pub fn is_armed(&self) -> bool {
        self.method.is_armed()
    }

    /// The candidates on offer (empty when no list is open).
    pub fn candidates(&self) -> &[String] {
        match &self.session {
            Some(s) => s.candidates(),
            None => &[],
        }
    }

    /// Closes the open list. The text to commit is candidate `pick` when one was chosen, else the composed letter.
    fn finish(&mut self, pick: Option<usize>) -> Option<String> {
        let mut session = self.session.take()?;
        if let Some(i) = pick {
            session.select_candidate(i);
        }
        session.commit()
    }

    /// Opens the candidate list for `original` (already shown as marked text by the caller).
    fn open(&mut self, original: &str, candidates: Vec<String>) {
        let mut session = InputSession::new(0, PanelType::SingleColumnScrolling);
        session.update(original, original);
        session.set_candidates(candidates);
        self.session = Some(session);
    }
}

/// The text input a composition types into: its marked text and its plain text at the caret.
pub trait ComposeTarget {
    /// Replaces the marked text (the composed letter, or a picked candidate) with `text`, the caret after it.
    fn set_marked(&mut self, text: &str);
    /// Ends the marked text and keeps it as plain text.
    fn commit_marked(&mut self);
    /// Types `text` as plain text at the caret.
    fn insert(&mut self, text: &str);
}

/// Types `text` (one key, or a paste) through the compose method. While a list is open a digit picks from it; any other
/// text first ends the list and keeps its letter. Then the text means what the method says: plain text, a held
/// apostrophe, or a new list over its letter.
pub fn type_text<T: ComposeTarget>(c: &mut Composer, target: &mut T, text: &str) {
    if c.is_open() {
        let picked = pick(c.candidates(), text);
        if let Some(committed) = c.finish(picked) {
            target.set_marked(&committed);
            target.commit_marked();
        }
        if picked.is_some() {
            return;
        }
    }
    match c.method.key(text) {
        Step::Insert(s) => target.insert(&s),
        Step::Armed => {}
        Step::Open { original, candidates } => {
            target.set_marked(&original);
            c.open(&original, candidates);
        }
    }
}

/// Ends the composition before a key that is not text (a control key, a command, a pointer press): an open list closes
/// keeping its letter, and an apostrophe still held is typed as its own key first. True when that apostrophe was typed.
pub fn end_for_control<T: ComposeTarget>(c: &mut Composer, target: &mut T) -> bool {
    if let Some(committed) = c.finish(None) {
        target.set_marked(&committed);
        target.commit_marked();
    }
    let held = c.method.take_prefix();
    if held {
        target.insert(&PREFIX.to_string());
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target that records what the composition did to it.
    #[derive(Default)]
    struct Rec {
        marked: Option<String>,
        committed: Vec<String>,
        inserted: String,
    }

    impl ComposeTarget for Rec {
        fn set_marked(&mut self, text: &str) {
            self.marked = Some(text.to_string());
        }
        fn commit_marked(&mut self) {
            if let Some(m) = self.marked.take() {
                self.committed.push(m);
            }
        }
        fn insert(&mut self, text: &str) {
            self.inserted.push_str(text);
        }
    }

    #[test]
    fn a_digit_picks_from_the_open_list_and_commits_the_candidate() {
        let mut c = Composer::default();
        let mut r = Rec::default();
        type_text(&mut c, &mut r, "'");
        type_text(&mut c, &mut r, "e");
        assert!(c.is_open());
        assert_eq!(r.marked.as_deref(), Some("e"));
        assert_eq!(c.candidates(), &["é".to_string(), "è".to_string(), "ê".to_string(), "ë".to_string()]);
        type_text(&mut c, &mut r, "2");
        assert!(!c.is_open());
        assert_eq!(r.committed, vec!["è".to_string()]);
        assert!(r.inserted.is_empty());
    }

    #[test]
    fn a_letter_typed_over_an_open_list_keeps_the_letter_and_is_typed() {
        let mut c = Composer::default();
        let mut r = Rec::default();
        type_text(&mut c, &mut r, "'");
        type_text(&mut c, &mut r, "e");
        type_text(&mut c, &mut r, "x");
        assert!(!c.is_open());
        assert_eq!(r.committed, vec!["e".to_string()]);
        assert_eq!(r.inserted, "x");
    }

    #[test]
    fn a_control_key_ends_the_list_and_keeps_the_letter() {
        let mut c = Composer::default();
        let mut r = Rec::default();
        type_text(&mut c, &mut r, "'");
        type_text(&mut c, &mut r, "o");
        assert!(!end_for_control(&mut c, &mut r), "the apostrophe was already used");
        assert!(!c.is_open());
        assert_eq!(r.committed, vec!["o".to_string()]);
        assert!(r.inserted.is_empty());
    }

    #[test]
    fn a_held_apostrophe_is_typed_before_a_control_key() {
        let mut c = Composer::default();
        let mut r = Rec::default();
        type_text(&mut c, &mut r, "'");
        assert!(c.is_armed());
        assert!(end_for_control(&mut c, &mut r));
        assert!(!c.is_armed());
        assert_eq!(r.inserted, "'");
    }

    #[test]
    fn plain_text_is_typed_and_a_letter_without_alternates_keeps_its_apostrophe() {
        let mut c = Composer::default();
        let mut r = Rec::default();
        type_text(&mut c, &mut r, "h");
        type_text(&mut c, &mut r, "'");
        type_text(&mut c, &mut r, "z");
        assert_eq!(r.inserted, "h'z");
        assert!(!c.is_open());
        assert!(r.committed.is_empty());
    }

    #[test]
    fn a_letter_without_alternates_after_an_open_list_is_plain_text() {
        let mut c = Composer::default();
        let mut r = Rec::default();
        type_text(&mut c, &mut r, "'");
        type_text(&mut c, &mut r, "a");
        // "a" opened a list; typing "z" closes it (keeping "a") and types "z"
        type_text(&mut c, &mut r, "z");
        assert_eq!(r.committed, vec!["a".to_string()]);
        assert_eq!(r.inserted, "z");
    }

    #[test]
    fn a_letter_offers_its_alternates_in_order_and_upper_case_follows_the_letter() {
        let a = candidates_for('a');
        assert_eq!(a.len(), 7);
        assert_eq!(a[0], "á");
        assert_eq!(a[6], "æ");
        let big = candidates_for('A');
        assert_eq!(big[0], "Á");
        assert_eq!(big.len(), 7);
        assert!(candidates_for('x').is_empty());
        assert!(candidates_for('q').is_empty());
        assert_eq!(candidates_for('d'), vec!["ð".to_string()]);
    }

    #[test]
    fn digits_pick_from_one_and_stop_at_the_list_end() {
        let list: Vec<String> = candidates_for('e');
        assert_eq!(list.len(), 4);
        assert_eq!(pick(&list, "1"), Some(0));
        assert_eq!(pick(&list, "4"), Some(3));
        assert_eq!(pick(&list, "5"), None);
        assert_eq!(pick(&list, "0"), None);
        assert_eq!(pick(&list, "x"), None);
        assert_eq!(pick(&list, "12"), None);
        assert_eq!(pick(&[], "1"), None);
    }

    #[test]
    fn an_apostrophe_then_a_letter_opens_the_list() {
        let mut c = Compose::new();
        assert_eq!(c.key("'"), Step::Armed);
        assert!(c.is_armed());
        assert_eq!(
            c.key("o"),
            Step::Open {
                original: "o".into(),
                candidates: vec!["ó".into(), "ò".into(), "ô".into(), "ö".into(), "õ".into(), "ø".into()]
            }
        );
        assert!(!c.is_armed());
    }

    #[test]
    fn plain_keys_pass_through_and_an_armed_apostrophe_is_kept_for_other_keys() {
        let mut c = Compose::new();
        assert_eq!(c.key("h"), Step::Insert("h".into()));
        assert_eq!(c.key("'"), Step::Armed);
        // a letter without alternates: the apostrophe and the letter are inserted together
        assert_eq!(c.key("z"), Step::Insert("'z".into()));
        assert_eq!(c.key("'"), Step::Armed);
        assert_eq!(c.key("'"), Step::Insert("''".into()));
        assert_eq!(c.key("'"), Step::Armed);
        assert_eq!(c.key(" "), Step::Insert("' ".into()));
        assert_eq!(c.key(""), Step::Insert("".into()));
    }

    #[test]
    fn take_prefix_reports_and_clears_the_armed_state() {
        let mut c = Compose::new();
        assert!(!c.take_prefix());
        c.key("'");
        assert!(c.take_prefix());
        assert!(!c.is_armed());
    }
}
