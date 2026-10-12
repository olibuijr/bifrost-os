//! Lexical tagging of text: splits a string into tokens (words, numbers, punctuation, whitespace) with byte ranges and
//! gives each word a lexical class (noun, verb, adjective, ...). Mirrors the lexical-class tagging of the natural
//! language framework (`NLTagger` with the lexical class scheme), as a rule-based model. Pure functions, no I/O.
//!
//! Rules, in order:
//! 1. A run of whitespace is `Whitespace`. Each punctuation character is its own `Punctuation` token.
//! 2. A word run of ASCII digits is `Number`.
//! 3. A word in the closed-class lexicon (articles, pronouns, prepositions, conjunctions, auxiliary and common verbs,
//!    a few adverbs and interjections) takes that class, matched case-insensitively.
//! 4. Otherwise suffix rules guess the class, for words longer than four characters: `-ly` adverb; `-ing` and `-ed`
//!    verb; `-ous`, `-ful`, `-able`, `-ible`, `-ive`, `-less`, `-ical` adjective; `-tion`, `-sion`, `-ness`, `-ment`,
//!    `-ity`, `-ism` noun.
//! 5. Anything else is `OtherWord`. No guess is made beyond the rules.
//!
//! Limits: there is no trained model and no lemma data, so the suffix rules are wrong for many words (family is an
//! adverb by the `-ly` rule). Words with inner digits or hyphens are one `OtherWord` token, and decimals split at the
//! full stop. A word can have only one class, so ambiguous words such as "her" take the first lexicon entry.

use std::ops::Range;

/// `LexicalClass`: the part of speech or the token kind given by the tagger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexicalClass {
    Noun,
    Verb,
    Adjective,
    Adverb,
    Pronoun,
    Determiner,
    Preposition,
    Conjunction,
    Interjection,
    Number,
    Punctuation,
    Whitespace,
    OtherWord,
}

/// `LexicalToken`: one token of the input, with its byte range in the input and its class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexicalToken {
    pub range: Range<usize>,
    pub class: LexicalClass,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Space,
    Word,
    Punct,
}

fn kind_of(c: char) -> Kind {
    if c.is_whitespace() {
        Kind::Space
    } else if c.is_alphanumeric() || c == '\'' {
        Kind::Word
    } else {
        Kind::Punct
    }
}

/// The class of a single word from the lexicon or the suffix rules. The word is matched in lowercase.
pub fn classify_word(word: &str) -> LexicalClass {
    let lower = word.to_lowercase();
    match lower.as_str() {
        "the" | "a" | "an" | "this" | "that" | "these" | "those" | "my" | "your" | "his" | "her" | "its" | "our"
        | "their" | "some" | "any" | "every" | "each" | "no" => LexicalClass::Determiner,
        "i" | "you" | "he" | "she" | "it" | "we" | "they" | "me" | "him" | "us" | "them" | "who" | "whom"
        | "which" => LexicalClass::Pronoun,
        "in" | "on" | "at" | "by" | "for" | "with" | "from" | "to" | "of" | "about" | "into" | "over" | "under"
        | "between" | "through" | "during" | "without" => LexicalClass::Preposition,
        "and" | "or" | "but" | "nor" | "so" | "yet" | "because" | "if" | "when" | "while" | "although"
        | "unless" => LexicalClass::Conjunction,
        "oh" | "wow" | "hello" | "hi" | "hey" | "ouch" | "oops" | "alas" => LexicalClass::Interjection,
        "is" | "am" | "are" | "was" | "were" | "be" | "been" | "being" | "have" | "has" | "had" | "do" | "does"
        | "did" | "can" | "could" | "will" | "would" | "shall" | "should" | "may" | "might" | "must" => {
            LexicalClass::Verb
        }
        "not" | "very" | "too" | "also" | "just" | "only" | "never" | "always" | "often" | "here" | "there"
        | "now" | "then" => LexicalClass::Adverb,
        _ => suffix_class(&lower),
    }
}

fn suffix_class(lower: &str) -> LexicalClass {
    if lower.chars().count() <= 4 {
        return LexicalClass::OtherWord;
    }
    if lower.ends_with("ly") {
        LexicalClass::Adverb
    } else if lower.ends_with("ing") || lower.ends_with("ed") {
        LexicalClass::Verb
    } else if ["ous", "ful", "able", "ible", "ive", "less", "ical"].iter().any(|s| lower.ends_with(*s)) {
        LexicalClass::Adjective
    } else if ["tion", "sion", "ness", "ment", "ity", "ism"].iter().any(|s| lower.ends_with(*s)) {
        LexicalClass::Noun
    } else {
        LexicalClass::OtherWord
    }
}

/// Tokenizes `text` and classes every token. Ranges are byte ranges on char boundaries and cover the whole input.
pub fn tag(text: &str) -> Vec<LexicalToken> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = kind_of(c);
        let mut end = start + c.len_utf8();
        // Punctuation is one character per token; whitespace and words are runs of one kind.
        if kind != Kind::Punct {
            while let Some(&(i, d)) = chars.peek() {
                if kind_of(d) != kind {
                    break;
                }
                end = i + d.len_utf8();
                chars.next();
            }
        }
        let slice = &text[start..end];
        let class = match kind {
            Kind::Space => LexicalClass::Whitespace,
            Kind::Punct => LexicalClass::Punctuation,
            Kind::Word => {
                if slice.chars().all(|d| d.is_ascii_digit()) {
                    LexicalClass::Number
                } else {
                    classify_word(slice)
                }
            }
        };
        out.push(LexicalToken { range: start..end, class });
    }
    out
}

/// The word tokens of `text` (no whitespace, no punctuation), in order, with their classes.
pub fn words(text: &str) -> Vec<(&str, LexicalClass)> {
    tag(text)
        .into_iter()
        .filter(|t| !matches!(t.class, LexicalClass::Punctuation | LexicalClass::Whitespace))
        .map(|t| (&text[t.range.clone()], t.class))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str) -> Vec<LexicalClass> {
        tag(text).into_iter().map(|t| t.class).collect()
    }

    #[test]
    fn a_sentence_gets_one_class_per_token() {
        // "The" determiner, "happiness" noun (-ness), "is" verb, "quickly" adverb (-ly), "2024" number, "." punctuation.
        let got = classes("The happiness is quickly 2024.");
        assert_eq!(
            got,
            vec![
                LexicalClass::Determiner,
                LexicalClass::Whitespace,
                LexicalClass::Noun,
                LexicalClass::Whitespace,
                LexicalClass::Verb,
                LexicalClass::Whitespace,
                LexicalClass::Adverb,
                LexicalClass::Whitespace,
                LexicalClass::Number,
                LexicalClass::Punctuation,
            ]
        );
    }

    #[test]
    fn token_ranges_cover_the_input_in_bytes() {
        let tokens = tag("in  it");
        assert_eq!(
            tokens,
            vec![
                LexicalToken { range: 0..2, class: LexicalClass::Preposition },
                LexicalToken { range: 2..4, class: LexicalClass::Whitespace },
                LexicalToken { range: 4..6, class: LexicalClass::Pronoun },
            ]
        );
        // The first letter is two bytes long, so the word is 7 bytes.
        assert_eq!(tag("Ísland"), vec![LexicalToken { range: 0..7, class: LexicalClass::OtherWord }]);
    }

    #[test]
    fn suffix_rules_apply_only_to_longer_words() {
        assert_eq!(classify_word("walking"), LexicalClass::Verb);
        assert_eq!(classify_word("wonderful"), LexicalClass::Adjective);
        assert_eq!(classify_word("nation"), LexicalClass::Noun);
        assert_eq!(classify_word("quickly"), LexicalClass::Adverb);
        // "only" is a lexicon adverb, so it does not depend on the suffix rule.
        assert_eq!(classify_word("only"), LexicalClass::Adverb);
        // Four letters or fewer skip the suffix rules.
        assert_eq!(classify_word("cat"), LexicalClass::OtherWord);
        assert_eq!(classify_word("ring"), LexicalClass::OtherWord);
    }

    #[test]
    fn each_punctuation_character_is_its_own_token() {
        let tokens = tag("?!");
        assert_eq!(
            tokens,
            vec![
                LexicalToken { range: 0..1, class: LexicalClass::Punctuation },
                LexicalToken { range: 1..2, class: LexicalClass::Punctuation },
            ]
        );
    }

    #[test]
    fn words_skips_spaces_and_punctuation() {
        assert_eq!(
            words("Oh, wow"),
            vec![("Oh", LexicalClass::Interjection), ("wow", LexicalClass::Interjection)]
        );
    }
}
