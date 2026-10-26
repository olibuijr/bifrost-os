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

/// Splits a name into (given, family) where the card's recorded family name starts. The name is stored as one string, so
/// a family name of several words ("van Dyke") is only known from the `family` the editor recorded; when `family` is
/// empty, or the name does not end with it after a space, this is [`split`].
pub fn split_with<'a>(name: &'a str, family: &str) -> (&'a str, &'a str) {
    let (name, family) = (name.trim(), family.trim());
    if name == family {
        return (name, "");
    }
    if !family.is_empty() {
        if let Some(given) = name.strip_suffix(family) {
            if given.ends_with(char::is_whitespace) {
                return (given.trim(), name[given.len()..].trim());
            }
        }
    }
    split(name)
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
    section_with(display, "")
}

/// [`section`] for a card whose family name was recorded (see [`split_with`]).
pub fn section_with(display: &str, family: &str) -> char {
    let (given, family) = split_with(display, family);
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

/// The order a name is shown in (View menu): given name first, or family name first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameOrder {
    #[default]
    FirstLast,
    LastFirst,
}

/// The name as the list and the card show it: "Ada Lovelace" in first-name order, or "Lovelace, Ada" in last-name
/// order. A name with only one part is shown as it is.
pub fn shown(name: &str, order: NameOrder) -> String {
    shown_with(name, "", order)
}

/// [`shown`] for a card whose family name was recorded (see [`split_with`]).
pub fn shown_with(name: &str, family: &str, order: NameOrder) -> String {
    let (given, family) = split_with(name, family);
    match order {
        NameOrder::LastFirst if !given.is_empty() && !family.is_empty() => format!("{family}, {given}"),
        _ => name.trim().to_string(),
    }
}

/// The title a card is shown under: the company when the card is marked as a company, the nickname when the person
/// prefers nicknames and the card has one, and otherwise the name in the chosen order. `name` is the card's display name.
pub fn title(name: &str, family: &str, company: &str, nickname: &str, is_company: bool, prefer_nicknames: bool, order: NameOrder) -> String {
    let (company, nickname) = (company.trim(), nickname.trim());
    if is_company && !company.is_empty() {
        company.to_string()
    } else if prefer_nicknames && !nickname.is_empty() {
        nickname.to_string()
    } else {
        shown_with(name, family, order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_prefers_the_company_flag_then_the_nickname_then_the_name() {
        let order = NameOrder::LastFirst;
        assert_eq!(title("Ada Lovelace", "", "Analytical Ltd", "Countess", true, true, order), "Analytical Ltd");
        assert_eq!(title("Ada Lovelace", "", "Analytical Ltd", "Countess", false, true, order), "Countess");
        assert_eq!(title("Ada Lovelace", "", "Analytical Ltd", "Countess", false, false, order), "Lovelace, Ada");
        assert_eq!(title("Ada Lovelace", "", "", "", true, true, order), "Lovelace, Ada", "a company flag without a company changes nothing");
        assert_eq!(title("Ada Lovelace", "", "", "  ", false, true, NameOrder::FirstLast), "Ada Lovelace");
    }

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

    #[test]
    fn a_recorded_family_name_keeps_its_words_together() {
        assert_eq!(split_with("QA Testperson edited", "Testperson edited"), ("QA", "Testperson edited"));
        assert_eq!(section_with("QA Testperson edited", "Testperson edited"), 'T');
        assert_eq!(section("QA Testperson edited"), 'E', "without the record the last word is the family name");
        assert_eq!(split_with("Ada Lovelace", "Berg"), ("Ada", "Lovelace"), "a stale record falls back to the last space");
        assert_eq!(split_with("Lovelace", "Lovelace"), ("Lovelace", ""), "the family name needs a given name before it");
        assert_eq!(shown_with("Jan van Dyke", "van Dyke", NameOrder::LastFirst), "van Dyke, Jan");
    }

    #[test]
    fn the_name_order_decides_which_part_leads() {
        assert_eq!(shown("Ada Lovelace", NameOrder::FirstLast), "Ada Lovelace");
        assert_eq!(shown("Ada Lovelace", NameOrder::LastFirst), "Lovelace, Ada");
        assert_eq!(shown("  Ada  Lovelace ", NameOrder::LastFirst), "Lovelace, Ada");
        assert_eq!(shown("Cher", NameOrder::LastFirst), "Cher", "a one-word name has no family name to lead with");
        assert_eq!(shown("Acme Inc", NameOrder::LastFirst), "Inc, Acme");
        assert_eq!(shown("", NameOrder::LastFirst), "");
        assert_eq!(NameOrder::default(), NameOrder::FirstLast);
    }
}
