//! Password generation rules (a generate-password request for a sign-up form): a random password that uses the character classes
//! an app asks for, with at least one character from each class.
//!
//! The generator takes its random draws from a caller-supplied function, so tests can fix every draw. [`generate`]
//! uses the kernel pool through [`entropy::below`]. The character sets are fixed: lowercase and uppercase ASCII
//! letters, digits, and a symbol set that avoids quotes, backslashes and spaces, which many sign-in forms reject.

use crate::entropy;
use std::fmt;

pub const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
pub const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
pub const DIGITS: &str = "0123456789";
pub const SYMBOLS: &str = "!#$%&*+-=?@^_";

/// Why a password could not be generated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No character class is enabled.
    NoCharacterClasses,
    /// The length is less than the number of enabled classes, so one character from each cannot fit.
    TooShort { required: usize },
    /// The random source failed.
    Entropy,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoCharacterClasses => f.write_str("no character class is enabled"),
            Error::TooShort { required } => write!(f, "the length must be at least {}", required),
            Error::Entropy => f.write_str("the random source failed"),
        }
    }
}

/// Which characters a password may use, and how long it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Rules {
    /// All four classes enabled, for a password of `length` characters.
    pub fn standard(length: usize) -> Rules {
        Rules { length, lowercase: true, uppercase: true, digits: true, symbols: true }
    }

    /// The enabled character sets, in the fixed order lowercase, uppercase, digits, symbols.
    fn classes(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.lowercase {
            out.push(LOWERCASE);
        }
        if self.uppercase {
            out.push(UPPERCASE);
        }
        if self.digits {
            out.push(DIGITS);
        }
        if self.symbols {
            out.push(SYMBOLS);
        }
        out
    }
}

/// A password from the kernel random pool.
pub fn generate(rules: &Rules) -> Result<String, Error> {
    generate_with(rules, |bound| entropy::below(bound).ok())
}

/// A password where `below(n)` supplies each draw as a whole number in `[0, n)`. `None` from `below` is an entropy
/// failure. The steps are: one character from each enabled class, the rest from the union, then a Fisher-Yates
/// shuffle so the class characters are not always first.
pub fn generate_with<F>(rules: &Rules, mut below: F) -> Result<String, Error>
where
    F: FnMut(u64) -> Option<u64>,
{
    let classes = rules.classes();
    if classes.is_empty() {
        return Err(Error::NoCharacterClasses);
    }
    if rules.length < classes.len() {
        return Err(Error::TooShort { required: classes.len() });
    }
    let union: Vec<char> = classes.concat().chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(rules.length);
    for class in &classes {
        let chars: Vec<char> = class.chars().collect();
        out.push(pick(&chars, &mut below)?);
    }
    while out.len() < rules.length {
        out.push(pick(&union, &mut below)?);
    }
    for i in (1..out.len()).rev() {
        let j = draw(&mut below, (i + 1) as u64)? as usize;
        out.swap(i, j);
    }
    Ok(out.into_iter().collect())
}

fn draw<F>(below: &mut F, bound: u64) -> Result<u64, Error>
where
    F: FnMut(u64) -> Option<u64>,
{
    below(bound).ok_or(Error::Entropy)
}

fn pick<F>(chars: &[char], below: &mut F) -> Result<char, Error>
where
    F: FnMut(u64) -> Option<u64>,
{
    let index = draw(below, chars.len() as u64)? as usize;
    Ok(chars[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_draws_give_the_hand_worked_password() {
        // Lowercase and digits only, length 4. First pass: 'a' (draw 0 of 26) and '3' (draw 3 of 10).
        // Fill: draw 35 of 36 is the last digit '9', draw 1 of 36 is 'b'. Shuffle from the end:
        // swap(3,0) gives b39a, swap(2,0) gives 93ba, swap(1,1) changes nothing. Result "93ba".
        let rules = Rules { length: 4, lowercase: true, uppercase: false, digits: true, symbols: false };
        let mut draws = vec![0u64, 3, 35, 1, 0, 0, 1].into_iter();
        assert_eq!(generate_with(&rules, |_| draws.next()), Ok("93ba".to_string()));
    }

    #[test]
    fn all_zero_draws_pick_the_first_of_each_class_and_reverse_the_shuffle() {
        // Each class takes its first character: a, A, 0, !. The all-zero shuffle swaps 3-0, 2-0, 1-0: "A0!a".
        assert_eq!(generate_with(&Rules::standard(4), |_| Some(0)), Ok("A0!a".to_string()));
    }

    #[test]
    fn no_enabled_class_is_refused() {
        let rules = Rules { length: 8, lowercase: false, uppercase: false, digits: false, symbols: false };
        assert_eq!(generate_with(&rules, |_| Some(0)), Err(Error::NoCharacterClasses));
    }

    #[test]
    fn a_length_below_the_class_count_is_refused() {
        assert_eq!(generate_with(&Rules::standard(3), |_| Some(0)), Err(Error::TooShort { required: 4 }));
    }

    #[test]
    fn a_failed_random_source_is_an_entropy_error() {
        let rules = Rules { length: 2, lowercase: true, uppercase: false, digits: false, symbols: false };
        assert_eq!(generate_with(&rules, |_| None), Err(Error::Entropy));
    }

    #[test]
    fn a_generated_password_has_the_length_and_every_class() {
        let pw = generate(&Rules::standard(20)).unwrap();
        assert_eq!(pw.chars().count(), 20);
        assert!(pw.chars().any(|c| LOWERCASE.contains(c)));
        assert!(pw.chars().any(|c| UPPERCASE.contains(c)));
        assert!(pw.chars().any(|c| DIGITS.contains(c)));
        assert!(pw.chars().any(|c| SYMBOLS.contains(c)));
        assert!(pw.chars().all(|c| LOWERCASE.contains(c) || UPPERCASE.contains(c) || DIGITS.contains(c) || SYMBOLS.contains(c)));
    }
}
