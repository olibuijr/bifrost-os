//! A built-in compose method, the simple input method the shell runs without a provider process. A typed apostrophe
//! arms it; the next letter that has accented alternates opens the candidate list for that letter (`'a` offers
//! a, a-acute, a-grave, ...). Digits 1 to 9 pick a candidate. Any other key ends the composition and keeps the letter.
//! Icelandic letters (acute vowels, o-umlaut, ae, eth, thorn) are in the table. Pure model: it draws nothing.

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

#[cfg(test)]
mod tests {
    use super::*;

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
