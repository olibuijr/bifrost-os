//! Inflection and grammar agreement: the plural category of a number in a language, the plural of a noun, and the
//! automatic agreement markup `^[2 apple](inflect: true)` that makes a sentence with a count read correctly ("2 apples",
//! "1 apple").
//!
//! * [`plural_category`] gives the CLDR-style category (zero, one, two, few, many, other) of a whole number, for all of
//!   the [`Language`]s the recognizer knows. A caller that has its own per-category strings (a `.stringsdict` style table)
//!   needs only this.
//! * [`inflect_noun`] builds the noun form for a count. Rules exist for English, Spanish and French (regular forms plus a
//!   table of irregular and invariable words); see [`Rules`]. Other languages have no noun rules here: [`rules_for`]
//!   falls back to [`Rules::Invariant`] and the noun is returned as written, never guessed.
//! * [`agree`] rewrites every `^[...](inflect: true)` span of a string for its number.
//!
//! Limits, stated plainly: only whole numbers and plain decimals are counts (a decimal always takes the plural form, as in
//! "1.5 apples"); there is no case, gender or article agreement; and the noun is taken to be the last word of the span.

use crate::language::Language;

/// A plural category (`CLDR` names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

/// The plural category of the whole number `n` in `language`.
pub fn plural_category(language: Language, n: u64) -> PluralCategory {
    use PluralCategory::*;
    let (m10, m100) = (n % 10, n % 100);
    match language {
        Language::Chinese | Language::Japanese | Language::Korean | Language::Thai => Other,
        Language::French | Language::Portuguese => {
            if n <= 1 {
                One
            } else {
                Other
            }
        }
        Language::Icelandic => {
            if m10 == 1 && m100 != 11 {
                One
            } else {
                Other
            }
        }
        Language::Russian => {
            if m10 == 1 && m100 != 11 {
                One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Few
            } else {
                Many
            }
        }
        Language::Polish => {
            if n == 1 {
                One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Few
            } else {
                Many
            }
        }
        Language::Arabic => match n {
            0 => Zero,
            1 => One,
            2 => Two,
            _ if (3..=10).contains(&m100) => Few,
            _ if (11..=99).contains(&m100) => Many,
            _ => Other,
        },
        Language::Hebrew => match n {
            1 => One,
            2 => Two,
            _ => Other,
        },
        // English, German, Dutch, Danish, Norwegian, Swedish, Italian, Spanish, Greek
        _ => {
            if n == 1 {
                One
            } else {
                Other
            }
        }
    }
}

/// How a language forms the plural of a noun here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rules {
    English,
    Spanish,
    French,
    /// No noun rules: the noun is never changed.
    Invariant,
}

/// The noun rules of a language; a language without any falls back to [`Rules::Invariant`].
pub fn rules_for(language: Language) -> Rules {
    match language {
        Language::English => Rules::English,
        Language::Spanish => Rules::Spanish,
        Language::French => Rules::French,
        _ => Rules::Invariant,
    }
}

const EN_IRREGULAR: &[(&str, &str)] = &[
    ("child", "children"),
    ("person", "people"),
    ("man", "men"),
    ("woman", "women"),
    ("foot", "feet"),
    ("tooth", "teeth"),
    ("goose", "geese"),
    ("mouse", "mice"),
    ("louse", "lice"),
    ("ox", "oxen"),
    ("datum", "data"),
    ("criterion", "criteria"),
    ("phenomenon", "phenomena"),
    ("analysis", "analyses"),
    ("crisis", "crises"),
    ("axis", "axes"),
    ("cactus", "cacti"),
    ("focus", "foci"),
];
const EN_INVARIANT: &[&str] = &["sheep", "fish", "deer", "series", "species", "aircraft", "moose", "offspring", "means"];
const EN_VES: &[&str] = &["leaf", "wolf", "knife", "life", "wife", "shelf", "half", "thief", "loaf", "calf", "self", "elf", "scarf"];
const EN_OES: &[&str] = &["hero", "potato", "tomato", "echo", "veto", "torpedo", "embargo"];
const FR_AL_S: &[&str] = &["bal", "carnaval", "festival", "récital", "chacal", "régal", "cérémonial"];

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

/// Gives `plural` the capital letter of `original` when it starts with one.
fn match_case(original: &str, plural: &str) -> String {
    if original.chars().next().is_some_and(char::is_uppercase) {
        let mut cs = plural.chars();
        match cs.next() {
            Some(f) => f.to_uppercase().chain(cs).collect(),
            None => String::new(),
        }
    } else {
        plural.to_string()
    }
}

/// Removes the last `n` characters of `s` and appends `tail`.
fn replace_end(s: &str, n: usize, tail: &str) -> String {
    let keep = s.chars().count().saturating_sub(n);
    s.chars().take(keep).chain(tail.chars()).collect()
}

fn english_plural(word: &str) -> String {
    let lower = word.to_lowercase();
    if let Some((_, p)) = EN_IRREGULAR.iter().find(|(s, _)| *s == lower) {
        return match_case(word, p);
    }
    if EN_INVARIANT.contains(&lower.as_str()) {
        return word.to_string();
    }
    let ends = |s: &str| lower.ends_with(s);
    let chars: Vec<char> = lower.chars().collect();
    let n = chars.len();
    if ends("s") || ends("x") || ends("z") || ends("ch") || ends("sh") {
        return format!("{word}es");
    }
    if ends("y") && n > 1 && !is_vowel(chars[n - 2]) {
        return replace_end(word, 1, "ies");
    }
    if EN_VES.contains(&lower.as_str()) {
        return replace_end(word, if ends("fe") { 2 } else { 1 }, "ves");
    }
    if EN_OES.contains(&lower.as_str()) {
        return format!("{word}es");
    }
    format!("{word}s")
}

fn spanish_plural(word: &str) -> String {
    let lower = word.to_lowercase();
    if lower.ends_with("ión") {
        return replace_end(word, 3, "iones");
    }
    if lower.ends_with('z') {
        return replace_end(word, 1, "ces");
    }
    match lower.chars().last() {
        None => String::new(),
        Some('s') | Some('x') => word.to_string(),
        Some(c) if is_vowel(c) || matches!(c, 'á' | 'é' | 'ó') => format!("{word}s"),
        Some(_) => format!("{word}es"),
    }
}

fn french_plural(word: &str) -> String {
    let lower = word.to_lowercase();
    if lower.ends_with('s') || lower.ends_with('x') || lower.ends_with('z') {
        return word.to_string();
    }
    if lower.ends_with("eau") || lower.ends_with("au") || lower.ends_with("eu") {
        return format!("{word}x");
    }
    if lower.ends_with("al") && !FR_AL_S.contains(&lower.as_str()) {
        return replace_end(word, 2, "aux");
    }
    format!("{word}s")
}

/// The plural of `word` under `rules`. `Invariant` returns the word unchanged.
pub fn pluralize(rules: Rules, word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }
    match rules {
        Rules::English => english_plural(word),
        Rules::Spanish => spanish_plural(word),
        Rules::French => french_plural(word),
        Rules::Invariant => word.to_string(),
    }
}

/// The form of the singular noun `singular` that goes with the count `n` in `language`: the singular when the language
/// puts `n` in the `One` category, the plural otherwise, and the word unchanged where no rules exist. A phrase takes the
/// plural on its last word ("memory card" becomes "memory cards").
pub fn inflect_noun(language: Language, singular: &str, n: u64) -> String {
    let rules = rules_for(language);
    if rules == Rules::Invariant || plural_category(language, n) == PluralCategory::One {
        return singular.to_string();
    }
    pluralize_phrase(rules, singular)
}

fn pluralize_phrase(rules: Rules, phrase: &str) -> String {
    // keep spacing: only the last word changes, and punctuation after it stays
    let trimmed_end = phrase.trim_end_matches(|c: char| !c.is_alphanumeric());
    let tail = &phrase[trimmed_end.len()..];
    let (head, word) = match trimmed_end.rfind(' ') {
        Some(i) => (&trimmed_end[..=i], &trimmed_end[i + 1..]),
        None => ("", trimmed_end),
    };
    format!("{head}{}{tail}", pluralize(rules, word))
}

/// Whether a token is a count, and which form it takes: `Some(n)` for a whole number, `None` for a plain decimal (which
/// always takes the plural), and `Err(())` when the token is not a number.
fn count_of(token: &str) -> Result<Option<u64>, ()> {
    let t = token.trim_matches(|c: char| !c.is_ascii_digit());
    if t.is_empty() || !token.chars().any(|c| c.is_ascii_digit()) {
        return Err(());
    }
    if !t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
        return Err(());
    }
    if t.contains('.') {
        return Ok(None);
    }
    t.replace(',', "").parse::<u64>().map(Some).map_err(|_| ())
}

fn inflect_span(language: Language, inner: &str) -> String {
    let tokens: Vec<&str> = inner.split(' ').collect();
    let Some(pos) = tokens.iter().position(|t| count_of(t).is_ok()) else { return inner.to_string() };
    let count = count_of(tokens[pos]).unwrap();
    // the noun is the last word after the number
    let Some(last) = (pos + 1..tokens.len()).rev().find(|&i| !tokens[i].is_empty()) else { return inner.to_string() };
    let rules = rules_for(language);
    if rules == Rules::Invariant || count.is_some_and(|n| plural_category(language, n) == PluralCategory::One) {
        return inner.to_string();
    }
    let mut out: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
    out[last] = pluralize_phrase(rules, tokens[last]);
    out.join(" ")
}

/// Rewrites each `^[text](inflect: true)` span of `text` for its number and drops the markup; spans with other
/// attributes lose the markup and keep their text; text outside spans is unchanged. An unfinished span stays as written.
pub fn agree(language: Language, text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("^[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(close) = after.find("](") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..close];
        let attrs_from = close + 2;
        let Some(end) = after[attrs_from..].find(')') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let attrs: String = after[attrs_from..attrs_from + end].chars().filter(|c| !c.is_whitespace()).collect();
        if attrs.split(',').any(|a| a == "inflect:true") {
            out.push_str(&inflect_span(language, inner));
        } else {
            out.push_str(inner);
        }
        rest = &after[attrs_from + end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_nouns_take_regular_and_irregular_plurals() {
        for (one, many) in [
            ("apple", "apples"),
            ("box", "boxes"),
            ("church", "churches"),
            ("city", "cities"),
            ("day", "days"),
            ("knife", "knives"),
            ("leaf", "leaves"),
            ("hero", "heroes"),
            ("photo", "photos"),
            ("child", "children"),
            ("Person", "People"),
            ("sheep", "sheep"),
            ("analysis", "analyses"),
        ] {
            assert_eq!(pluralize(Rules::English, one), many, "{one}");
        }
    }

    #[test]
    fn spanish_and_french_nouns_follow_their_regular_rules() {
        for (one, many) in [("casa", "casas"), ("papel", "papeles"), ("lápiz", "lápices"), ("canción", "canciones"), ("lunes", "lunes")] {
            assert_eq!(pluralize(Rules::Spanish, one), many, "{one}");
        }
        for (one, many) in [("livre", "livres"), ("gâteau", "gâteaux"), ("journal", "journaux"), ("festival", "festivals"), ("prix", "prix")] {
            assert_eq!(pluralize(Rules::French, one), many, "{one}");
        }
    }

    #[test]
    fn plural_categories_follow_each_language() {
        use PluralCategory::*;
        assert_eq!(plural_category(Language::English, 1), One);
        assert_eq!(plural_category(Language::English, 0), Other);
        assert_eq!(plural_category(Language::French, 0), One);
        assert_eq!(plural_category(Language::Icelandic, 21), One);
        assert_eq!(plural_category(Language::Icelandic, 11), Other);
        assert_eq!(plural_category(Language::Russian, 22), Few);
        assert_eq!(plural_category(Language::Russian, 11), Many);
        assert_eq!(plural_category(Language::Polish, 5), Many);
        assert_eq!(plural_category(Language::Arabic, 0), Zero);
        assert_eq!(plural_category(Language::Arabic, 2), Two);
        assert_eq!(plural_category(Language::Arabic, 7), Few);
        assert_eq!(plural_category(Language::Arabic, 25), Many);
        assert_eq!(plural_category(Language::Japanese, 1), Other);
    }

    #[test]
    fn a_noun_follows_its_count() {
        assert_eq!(inflect_noun(Language::English, "apple", 1), "apple");
        assert_eq!(inflect_noun(Language::English, "apple", 0), "apples");
        assert_eq!(inflect_noun(Language::English, "memory card", 3), "memory cards");
        assert_eq!(inflect_noun(Language::French, "livre", 0), "livre");
        assert_eq!(inflect_noun(Language::French, "livre", 2), "livres");
    }

    #[test]
    fn a_language_without_rules_never_changes_the_noun() {
        assert_eq!(rules_for(Language::German), Rules::Invariant);
        assert_eq!(inflect_noun(Language::German, "Apfel", 5), "Apfel");
        assert_eq!(inflect_noun(Language::Icelandic, "epli", 5), "epli");
    }

    #[test]
    fn markup_spans_agree_with_their_number() {
        let en = Language::English;
        assert_eq!(agree(en, "^[1 apple](inflect: true)"), "1 apple");
        assert_eq!(agree(en, "You have ^[2 apple](inflect: true) in the bag."), "You have 2 apples in the bag.");
        assert_eq!(agree(en, "^[0 memory card](inflect: true)."), "0 memory cards.");
        assert_eq!(agree(en, "^[1,200 file](inflect: true)"), "1,200 files");
        assert_eq!(agree(en, "^[1.5 hour](inflect: true)"), "1.5 hours");
        assert_eq!(agree(en, "^[3 child](inflect: true) and ^[1 child](inflect: true)"), "3 children and 1 child");
    }

    #[test]
    fn markup_without_inflection_or_a_number_is_left_as_text() {
        let en = Language::English;
        assert_eq!(agree(en, "^[2 apple](bold: true)"), "2 apple");
        assert_eq!(agree(en, "^[some apple](inflect: true)"), "some apple");
        assert_eq!(agree(en, "plain text"), "plain text");
        assert_eq!(agree(en, "broken ^[2 apple"), "broken ^[2 apple");
        assert_eq!(agree(en, "broken ^[2 apple](inflect: true"), "broken ^[2 apple](inflect: true");
    }

    #[test]
    fn markup_in_a_language_without_rules_keeps_the_noun() {
        assert_eq!(agree(Language::German, "^[2 Apfel](inflect: true)"), "2 Apfel");
        assert_eq!(agree(Language::Spanish, "^[2 casa](inflect: true)"), "2 casas");
    }
}
