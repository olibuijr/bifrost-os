//! Name order and sort keys for the contact list. A name is split at its last space into the given name and the family
//! name. The list sorts by family name, then given name (the `CNContactSortOrder.familyName` order). Letters
//! fold to a collation order: accents sort with their base letter, and the Icelandic letters ð, þ, æ and ö keep their
//! own places in the alphabet (ð after d, þ after z). Pure std; no locale tables beyond these.

use std::cmp::Ordering;

/// A collation key: one weight per character. Non-letters sort before letters.
pub type Key = Vec<(u8, u32)>;

/// Splits a name at its last space into (given, family). A one-word name is all given name.
pub fn split(name: &str) -> (&str, &str) {
    let name = name.trim();
    match name.rsplit_once(char::is_whitespace) {
        Some((given, family)) => (given.trim(), family.trim()),
        None => (name, ""),
    }
}

/// The ASCII letter a Latin letter sorts as, with its accent removed. `None` for anything else.
pub fn base(c: char) -> Option<char> {
    let c = c.to_lowercase().next()?;
    let letter = match c {
        'a'..='z' => c,
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'ç' => 'c',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ð' => 'd',
        _ => return None,
    };
    Some(letter)
}

/// A name folded for comparison: lower case, accents removed, single spaces. Icelandic letters without a Latin base
/// keep their lower-case form.
pub fn fold(text: &str) -> String {
    let folded: String = text.chars().map(|c| base(c).unwrap_or_else(|| c.to_lowercase().next().unwrap_or(c))).collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn weight(c: char) -> (u8, u32) {
    // Four slots per letter leave room for the Icelandic letters that sort after a base letter.
    let slot = |letter: char| 4 * (letter as u32 - 'a' as u32 + 1);
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'ý' => (1, slot('y') + 1),
        'ð' => (1, slot('d') + 1),
        'þ' => (1, slot('z') + 1),
        'æ' => (1, slot('z') + 2),
        'ö' => (1, slot('z') + 3),
        _ => match base(lower) {
            Some(letter) => (1, slot(letter)),
            None if lower.is_alphabetic() => (2, lower as u32),
            None => (0, lower as u32),
        },
    }
}

/// The collation key of a text.
pub fn key(text: &str) -> Key {
    text.chars().map(weight).collect()
}

/// The sort key of a displayed name: family key, given key, then the whole name as a tie-break.
pub fn sort_key(display: &str) -> (Key, Key, Key) {
    let (given, family) = split(display);
    let family = if family.is_empty() { given } else { family };
    (key(family), key(given), key(display.trim()))
}

/// Orders two displayed names for the list.
pub fn compare(a: &str, b: &str) -> Ordering {
    sort_key(a).cmp(&sort_key(b))
}

/// The section letter of a name: its family name's first letter, upper case. Other names share the `#` section.
pub fn section(display: &str) -> char {
    let (given, family) = split(display);
    let word = if family.is_empty() { given } else { family };
    let Some(first) = word.chars().next() else { return '#' };
    if let Some(letter) = base(first) {
        return letter.to_ascii_uppercase();
    }
    match first.to_lowercase().next() {
        Some('þ') => 'Þ',
        Some('æ') => 'Æ',
        Some('ö') => 'Ö',
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or('#'),
        _ => '#',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.sort_by(|a, b| compare(a, b));
        v
    }

    #[test]
    fn a_name_splits_at_its_last_space() {
        assert_eq!(split("Ólafur Búi Ólafsson"), ("Ólafur Búi", "Ólafsson"));
        assert_eq!(split("  Anna  "), ("Anna", ""));
        assert_eq!(split("Anna Lindqvist"), ("Anna", "Lindqvist"));
    }

    #[test]
    fn the_list_sorts_by_family_name_then_given_name() {
        assert_eq!(sorted(&["Anders Ek", "Anna As", "Bo Berg"]), ["Anna As", "Bo Berg", "Anders Ek"]);
        assert_eq!(sorted(&["Bo Berg", "Anna Berg"]), ["Anna Berg", "Bo Berg"]);
    }

    #[test]
    fn accents_sort_with_their_base_letter() {
        assert_eq!(sorted(&["Olsen Ole", "Ólafsson Ólafur"]), ["Ólafsson Ólafur", "Olsen Ole"]);
        assert_eq!(compare("Émile Ernst", "Eric Ernst"), Ordering::Less, "the given name breaks the family tie");
    }

    #[test]
    fn icelandic_letters_keep_their_places() {
        // ð sorts after d and before e; þ sorts after z.
        assert_eq!(sorted(&["Anna Eiríksson", "Anna Ðórsson"]), ["Anna Ðórsson", "Anna Eiríksson"]);
        assert_eq!(sorted(&["Anna Þórsson", "Anna Zoega"]), ["Anna Zoega", "Anna Þórsson"]);
        assert_eq!(sorted(&["Anna Ölvir", "Anna Æsir"]), ["Anna Æsir", "Anna Ölvir"]);
    }

    #[test]
    fn sections_use_the_family_name_initial() {
        assert_eq!(section("Anna Lindqvist"), 'L');
        assert_eq!(section("Ólafur Ólafsson"), 'O');
        assert_eq!(section("Anna Þórsson"), 'Þ');
        assert_eq!(section("Acme Inc"), 'I', "the last word is the family name");
        assert_eq!(section("Acme"), 'A', "a one-word name sorts by that word");
        assert_eq!(section("Taxi 1800"), '#', "a family name that starts with a digit goes under #");
        assert_eq!(section(""), '#');
    }
}
