//! A rule-based proofreader for the writing assistant (`writing_assist::Task::Proofread`). It finds the slips a careful
//! reader catches without understanding the text: a word typed twice, extra or missing spaces around punctuation, a
//! sentence that starts in lower case, "a" before a vowel sound, a lone "i". Every finding is a byte-range replacement
//! (`writing_assist::Suggestion`), so the session applies and drops them with the checks it already has.
//!
//! The proofreader runs on the device, reads only the range it is given and keeps nothing. It has no language model:
//! the spacing rules apply to any language, and the rules that depend on English (capitals after a full stop, "a" and
//! "an", "i") run only when [`Options::english`] is set. Limits: it does not know names, abbreviations beyond a short
//! list, or quoted code, so a finding is a suggestion the person accepts, never an automatic edit. Text such as
//! `a,b,c` is flagged for missing spaces after commas.

use crate::writing_assist::{Session, SessionError, Suggestion, Task};
use std::ops::Range;

/// What kind of slip a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    RepeatedWord,
    ExtraSpace,
    SpaceBeforePunctuation,
    MissingSpace,
    Capitalisation,
    Article,
    PronounI,
}

/// One finding: the suggestion and the sentence that explains it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub issue: Issue,
    pub suggestion: Suggestion,
    pub message: &'static str,
}

/// Which rules run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The text is English: also check capitals after a full stop, "a" and "an", and "i".
    pub english: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Word,
    Blank,
    Break,
    Punct,
    Other,
}

#[derive(Clone, Debug)]
struct Tok {
    kind: Kind,
    range: Range<usize>,
}

/// Words that may repeat on purpose.
const MAY_REPEAT: [&str; 4] = ["had", "that", "very", "bye"];
/// Words before a full stop that do not end a sentence.
const ABBREVIATIONS: [&str; 14] =
    ["etc", "vs", "eg", "ie", "approx", "fig", "inc", "ltd", "dr", "mr", "mrs", "ms", "prof", "no"];
/// Words that start with a vowel letter but a consonant sound.
const VOWEL_LETTER_CONSONANT_SOUND: [&str; 7] = ["uni", "use", "usu", "uti", "eu", "one", "once"];
/// Words that start with a consonant letter but a vowel sound.
const CONSONANT_LETTER_VOWEL_SOUND: [&str; 5] = ["hour", "honest", "honor", "honour", "heir"];

fn snap(text: &str, range: Range<usize>) -> Range<usize> {
    let down = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let start = down(range.start);
    start..down(range.end).max(start)
}

fn tokens(text: &str, scope: Range<usize>) -> Vec<Tok> {
    let cs: Vec<(usize, char)> = text[scope.clone()].char_indices().map(|(i, c)| (scope.start + i, c)).collect();
    let at = |k: usize| cs.get(k).map_or(scope.end, |&(i, _)| i);
    let mut out = Vec::new();
    let mut k = 0;
    while k < cs.len() {
        let (start, c) = cs[k];
        let (kind, next) = if c.is_alphanumeric() {
            let mut j = k + 1;
            while j < cs.len() {
                let d = cs[j].1;
                if d.is_alphanumeric() {
                    j += 1;
                } else if matches!(d, '\'' | '\u{2019}' | '-') && cs.get(j + 1).map_or(false, |&(_, e)| e.is_alphanumeric()) {
                    j += 2;
                } else {
                    break;
                }
            }
            (Kind::Word, j)
        } else if c == ' ' || c == '\t' {
            let mut j = k + 1;
            while j < cs.len() && (cs[j].1 == ' ' || cs[j].1 == '\t') {
                j += 1;
            }
            (Kind::Blank, j)
        } else if c == '\n' || c == '\r' {
            (Kind::Break, k + 1)
        } else if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') {
            (Kind::Punct, k + 1)
        } else {
            (Kind::Other, k + 1)
        };
        out.push(Tok { kind, range: start..at(next) });
        k = next;
    }
    out
}

fn first_char(s: &str) -> Option<char> {
    s.chars().next()
}

fn is_punct(text: &str, t: &Tok, c: char) -> bool {
    t.kind == Kind::Punct && text[t.range.clone()].starts_with(c)
}

fn finding(issue: Issue, range: Range<usize>, replacement: String, message: &'static str) -> Finding {
    Finding { issue, suggestion: Suggestion { range, replacement }, message }
}

/// Upper case of the first character of `word`, as a replacement for that character.
fn capital_of(word: &str) -> Option<(usize, String)> {
    let c = first_char(word)?;
    if !c.is_lowercase() {
        return None;
    }
    Some((c.len_utf8(), c.to_uppercase().collect()))
}

/// The article "a" or "an" the word that follows needs, in English.
fn article_for(next: &str) -> Option<&'static str> {
    let c = first_char(next)?;
    if !c.is_alphabetic() {
        return None;
    }
    // an acronym is read letter by letter, and the sound is not known from the letters
    if next.chars().count() > 1 && next.chars().all(|c| c.is_uppercase()) {
        return None;
    }
    let lower = next.to_lowercase();
    if CONSONANT_LETTER_VOWEL_SOUND.iter().any(|w| lower.starts_with(w)) {
        return Some("an");
    }
    if matches!(lower.chars().next(), Some('a' | 'e' | 'i' | 'o' | 'u')) {
        if VOWEL_LETTER_CONSONANT_SOUND.iter().any(|w| lower.starts_with(w)) {
            return Some("a");
        }
        return Some("an");
    }
    Some("a")
}

/// Findings in `text` within `scope`, in text order, none overlapping another. `scope` is snapped to character
/// boundaries.
pub fn proofread(text: &str, scope: Range<usize>, options: Options) -> Vec<Finding> {
    let scope = snap(text, scope);
    let toks = tokens(text, scope);
    let mut found: Vec<Finding> = Vec::new();
    let s = |t: &Tok| &text[t.range.clone()];

    for i in 0..toks.len() {
        let t = &toks[i];
        match t.kind {
            Kind::Word => {
                let word = s(t);
                // a word typed twice
                if let (Some(b), Some(w2)) = (toks.get(i + 1), toks.get(i + 2)) {
                    if b.kind == Kind::Blank
                        && w2.kind == Kind::Word
                        && word.chars().all(|c| c.is_alphabetic())
                        && word.eq_ignore_ascii_case(s(w2))
                        && !MAY_REPEAT.contains(&word.to_ascii_lowercase().as_str())
                    {
                        found.push(finding(Issue::RepeatedWord, b.range.start..w2.range.end, String::new(), "This word is repeated."));
                    }
                }
                if options.english {
                    let after_blank = toks.get(i + 1).filter(|b| b.kind == Kind::Blank && b.range.len() == 1);
                    if let (Some(_), Some(w2)) = (after_blank, toks.get(i + 2).filter(|n| n.kind == Kind::Word)) {
                        if matches!(word, "a" | "A" | "an" | "An") {
                            if let Some(want) = article_for(s(w2)) {
                                if !word.eq_ignore_ascii_case(want) {
                                    let upper = word.starts_with('A');
                                    let mut r = want.to_string();
                                    if upper {
                                        r = r.replacen('a', "A", 1);
                                    }
                                    found.push(finding(Issue::Article, t.range.clone(), r, "Use \"a\" before a consonant sound and \"an\" before a vowel sound."));
                                }
                            }
                        }
                    }
                    let lone_i = word == "i" || word.starts_with("i'") || word.starts_with("i\u{2019}");
                    let next_ok = toks.get(i + 1).map_or(true, |n| !(is_punct(text, n, '.') || s(n) == ")"));
                    if lone_i && next_ok {
                        found.push(finding(Issue::PronounI, t.range.start..t.range.start + 1, "I".into(), "The word \"I\" is written with a capital letter."));
                    }
                }
            }
            Kind::Blank => {
                let prev = i.checked_sub(1).map(|p| &toks[p]);
                let next = toks.get(i + 1);
                let inside_line = prev.map_or(false, |p| p.kind != Kind::Break) && next.map_or(false, |n| n.kind != Kind::Break);
                if inside_line && t.range.len() > 1 {
                    found.push(finding(Issue::ExtraSpace, t.range.clone(), " ".into(), "There is more than one space here."));
                }
                // a space before a comma, semicolon, full stop, exclamation or question mark
                if let (Some(p), Some(n)) = (prev, next) {
                    let after_ellipsis = toks.get(i + 2).map_or(false, |a| is_punct(text, a, '.'));
                    let closes = n.kind == Kind::Punct
                        && matches!(s(n), "," | ";" | "!" | "?" | ".")
                        && !(is_punct(text, n, '.') && after_ellipsis);
                    if p.kind == Kind::Word && closes {
                        found.push(finding(Issue::SpaceBeforePunctuation, t.range.clone(), String::new(), "Remove the space before the punctuation mark."));
                    }
                }
            }
            Kind::Punct => {
                let mark = s(t);
                // a comma or semicolon straight into a word
                if matches!(mark, "," | ";") {
                    let before = i.checked_sub(1).map(|p| &toks[p]);
                    let after = toks.get(i + 1);
                    if let (Some(b), Some(a)) = (before, after) {
                        let b_ends_letter = s(b).chars().last().map_or(false, |c| c.is_alphabetic());
                        let a_starts_letter = first_char(s(a)).map_or(false, |c| c.is_alphabetic());
                        if b.kind == Kind::Word && a.kind == Kind::Word && b_ends_letter && a_starts_letter {
                            found.push(finding(Issue::MissingSpace, t.range.end..t.range.end, " ".into(), "Add a space after the punctuation mark."));
                        }
                    }
                }
                // a sentence that starts in lower case
                if options.english && matches!(mark, "." | "!" | "?") {
                    let prev = i.checked_sub(1).map(|p| &toks[p]);
                    let before_prev = i.checked_sub(2).map(|p| &toks[p]);
                    let gap = toks.get(i + 1);
                    let next = toks.get(i + 2);
                    if let (Some(p), Some(g), Some(n)) = (prev, gap, next) {
                        let word = s(p);
                        let sentence_end = p.kind == Kind::Word
                            && word.chars().count() >= 3
                            && word.chars().any(|c| c.is_alphabetic())
                            && !ABBREVIATIONS.contains(&word.to_ascii_lowercase().as_str())
                            && !before_prev.map_or(false, |b| is_punct(text, b, '.'));
                        if sentence_end && g.kind == Kind::Blank && n.kind == Kind::Word {
                            if let Some((len, upper)) = capital_of(s(n)) {
                                found.push(finding(Issue::Capitalisation, n.range.start..n.range.start + len, upper, "A sentence starts with a capital letter."));
                            }
                        }
                    }
                }
            }
            Kind::Break | Kind::Other => {}
        }
    }

    // text order, and no two findings over the same bytes
    found.sort_by_key(|f| (f.suggestion.range.start, f.suggestion.range.end));
    let mut kept: Vec<Finding> = Vec::new();
    let mut last_end = 0;
    for f in found {
        if kept.is_empty() || f.suggestion.range.start >= last_end {
            last_end = f.suggestion.range.end;
            kept.push(f);
        }
    }
    kept
}

/// Starts a proofreading task on `session` over `scope` and offers it the findings. Returns how many there are.
pub fn begin_proofread(session: &mut Session, text: &str, scope: Range<usize>, options: Options) -> Result<usize, SessionError> {
    let scope = snap(text, scope);
    session.begin(Task::Proofread, scope.clone())?;
    let findings = proofread(text, scope, options);
    let n = findings.len();
    session.offer(findings.into_iter().map(|f| f.suggestion).collect())?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing_assist::{apply, Behavior};

    const EN: Options = Options { english: true };
    const ANY: Options = Options { english: false };

    fn fixed(text: &str, options: Options) -> String {
        let found = proofread(text, 0..text.len(), options);
        let s: Vec<Suggestion> = found.into_iter().map(|f| f.suggestion).collect();
        apply(text, &s).unwrap()
    }

    #[test]
    fn a_repeated_word_loses_the_second_copy() {
        assert_eq!(fixed("This is is a test.", ANY), "This is a test.");
        assert_eq!(fixed("The The end", ANY), "The end");
    }

    #[test]
    fn words_that_may_repeat_and_numbers_are_left_alone() {
        assert_eq!(fixed("It had had effect.", ANY), "It had had effect.");
        assert_eq!(fixed("Call 11 11 now", ANY), "Call 11 11 now");
    }

    #[test]
    fn extra_spaces_inside_a_line_are_one_space_but_indentation_is_kept() {
        assert_eq!(fixed("one  two", ANY), "one two");
        assert_eq!(fixed("a\n    indented", ANY), "a\n    indented");
        assert_eq!(fixed("end   \nnext", ANY), "end   \nnext");
    }

    #[test]
    fn a_space_before_a_comma_or_full_stop_is_removed_but_an_ellipsis_is_kept() {
        assert_eq!(fixed("Well , yes .", ANY), "Well, yes.");
        assert_eq!(fixed("wait ...", ANY), "wait ...");
    }

    #[test]
    fn a_missing_space_after_a_comma_is_added_only_between_letters() {
        assert_eq!(fixed("red,green", ANY), "red, green");
        assert_eq!(fixed("pi is 3,14", ANY), "pi is 3,14");
    }

    #[test]
    fn the_english_rules_run_only_when_asked() {
        assert_eq!(fixed("it ends. then more", ANY), "it ends. then more");
        assert_eq!(fixed("it ends. then more", EN), "it ends. Then more");
    }

    #[test]
    fn abbreviations_and_initials_do_not_end_a_sentence() {
        assert_eq!(fixed("See fig. two and etc. more", EN), "See fig. two and etc. more");
        assert_eq!(fixed("by J.R.R. tolkien", EN), "by J.R.R. tolkien");
        assert_eq!(fixed("item 3. buy milk", EN), "item 3. buy milk");
    }

    #[test]
    fn a_and_an_follow_the_sound() {
        assert_eq!(fixed("an cat and a apple", EN), "a cat and an apple");
        assert_eq!(fixed("A hour later, An unit", EN), "An hour later, A unit");
        assert_eq!(fixed("an FBI agent", EN), "an FBI agent");
        assert_eq!(fixed("a 5 item list", EN), "a 5 item list");
    }

    #[test]
    fn a_lone_i_is_capitalised_with_its_contraction() {
        assert_eq!(fixed("i think i'm late", EN), "I think I'm late");
        assert_eq!(fixed("step i) then ii", EN), "step i) then ii");
    }

    #[test]
    fn findings_come_in_text_order_and_never_overlap() {
        let text = "the the  cat , sat";
        let found = proofread(text, 0..text.len(), EN);
        for pair in found.windows(2) {
            assert!(pair[0].suggestion.range.end <= pair[1].suggestion.range.start);
        }
        assert!(found.iter().any(|f| f.issue == Issue::RepeatedWord));
    }

    #[test]
    fn only_the_scope_is_read_and_a_scope_inside_a_character_is_snapped() {
        let text = "ok ok. fine fine";
        let found = proofread(text, 7..text.len(), ANY);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].suggestion.range, 11..16);
        let accents = "é é";
        assert!(proofread(accents, 1..accents.len(), ANY).len() <= 1);
    }

    #[test]
    fn the_session_gets_the_findings_and_applies_one() {
        let text = "Say it it again";
        let mut session = Session::new(Behavior::Complete);
        let n = begin_proofread(&mut session, text, 0..text.len(), ANY).unwrap();
        assert_eq!(n, 1);
        assert_eq!(session.task(), Some(Task::Proofread));
        assert_eq!(session.accept(text, 0).unwrap(), "Say it again");
        assert!(session.pending().is_empty());
    }

    #[test]
    fn a_disabled_view_gets_no_proofreading() {
        let mut session = Session::new(Behavior::None);
        assert_eq!(begin_proofread(&mut session, "x x", 0..3, ANY), Err(SessionError::Disabled));
    }
}
