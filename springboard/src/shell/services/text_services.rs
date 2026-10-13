//! The `text` service: spelling checks for every app, the words a person taught the system or told it to ignore, and
//! the text replacements of Settings > Keyboard > Text Replacements. The checker is `bifrost_core::spellcheck` over
//! the keyboard's English word list (the same list the on-screen keyboard predicts from). The learned words and the
//! replacements are saved in the settings file (`text.replacements`, and the learned words under the checker's key).
//!
//! Methods (params are JSON objects; a range is `[start, end]` in bytes of the text):
//! `check_spelling {text, start}` (range or null), `check {text}` (list of ranges), `correction {text, range}`
//! (word or null), `guesses {text, range}` (list of words), `learn_word {word}`, `unlearn_word {word}`,
//! `has_learned_word {word}`, `ignore_word {word}`, `replacements` (list of `{shortcut, phrase}`),
//! `set_replacement {shortcut, phrase}`, `remove_replacement {shortcut}`, `expand {word}` (phrase or null).
//!
//! The state is one process-wide value, like the `cursor` service: the Keyboard pages read the replacement list
//! through [`replacement_rows`] and remove an entry through [`remove_replacement`].
use super::super::prefs::Prefs;
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_core::spellcheck::{CheckingTypes, KeyValues, SpellChecker, WordList};
use bifrost_core::text_replacements::{Refused, Replacements};
use std::ops::Range;
use std::sync::Mutex;

/// The settings key of the replacement list (one line, see `bifrost_core::text_replacements`).
pub const REPLACEMENTS_PREF: &str = "text.replacements";

/// The English word list the checker starts from (the on-screen keyboard's list: a word and its count per line).
const EN_WORDS: &str = include_str!("../../../apps/keyboard/src/words/en.txt");

/// The checker and the replacement list.
struct State {
    checker: SpellChecker,
    replacements: Replacements,
}

impl State {
    fn fresh() -> State {
        let words = EN_WORDS.lines().filter_map(|l| l.split_whitespace().next());
        State { checker: SpellChecker::new(Box::new(WordList::new("en", words))), replacements: Replacements::new() }
    }
}

/// The one state of the desktop. `None` until the first use, because the word list is built on first use.
static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Runs `f` on the state. A poisoned lock still gives the state back: a spelling check is never worth a panic.
fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::fresh))
}

/// The settings file holds one line per value: a learned word is stored with its line breaks as spaces, and a word
/// never holds a space (the checker's words are runs of letters, digits and apostrophes).
impl KeyValues for Prefs {
    fn get_string(&self, key: &str) -> Option<String> {
        self.get(key).map(|v| v.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("\n"))
    }
    fn put_string(&mut self, key: &str, value: &str) {
        let _ = self.set(key, &value.replace('\n', " "));
    }
}

/// Loads the learned words and the replacements from the settings file. The shell calls it once at start.
pub fn load(prefs: &Prefs) {
    with(|s| {
        s.checker.load(prefs);
        s.replacements = Replacements::from_line(prefs.get(REPLACEMENTS_PREF).unwrap_or(""));
    });
}

/// Every replacement as (shortcut, phrase), sorted by shortcut.
pub fn replacement_rows() -> Vec<(String, String)> {
    with(|s| s.replacements.entries().iter().map(|e| (e.shortcut.clone(), e.phrase.clone())).collect())
}

/// The number of replacements, for the row's value on the Keyboard page.
pub fn replacement_count() -> usize {
    with(|s| s.replacements.len())
}

/// The Keyboard row's value: "None", or the number of replacements.
pub fn replacements_summary() -> String {
    match replacement_count() {
        0 => "None".to_string(),
        n => n.to_string(),
    }
}

/// The phrase the whole word `word` expands to, for a hardware keyboard's typing (None when it is not a replacement).
pub fn expand_word(word: &str) -> Option<String> {
    with(|s| s.replacements.expand(word).map(str::to_string))
}

/// Removes the replacement for `shortcut` and saves the list. False when there was none.
pub fn remove_replacement(prefs: &mut Prefs, shortcut: &str) -> bool {
    with(|s| {
        let gone = s.replacements.remove(shortcut);
        if gone {
            let _ = prefs.set(REPLACEMENTS_PREF, &s.replacements.to_line());
        }
        gone
    })
}

fn refusal(r: Refused) -> &'static str {
    match r {
        Refused::EmptyShortcut => "the shortcut must not be empty",
        Refused::ShortcutHasSpace => "a shortcut is one word",
        Refused::EmptyPhrase => "the phrase must not be empty",
        Refused::ControlCharacter => "the text must not contain control characters",
    }
}

fn text_of(params: &Json) -> Result<String, String> {
    params.get("text").and_then(Json::as_str).map(str::to_string).ok_or_else(|| "the text must be a string".to_string())
}

fn word_of(params: &Json) -> Result<String, String> {
    let w = params.get("word").and_then(Json::as_str).ok_or("the word must be a string")?;
    if w.is_empty() || w.chars().any(char::is_whitespace) {
        return Err("a word is one run of letters".to_string());
    }
    Ok(w.to_string())
}

fn field_str(params: &Json, key: &str) -> Result<String, String> {
    params.get(key).and_then(Json::as_str).map(str::to_string).ok_or_else(|| format!("{key} must be a string"))
}

fn range_json(r: Range<usize>) -> Json {
    Json::Arr(vec![Json::Num(r.start as f64), Json::Num(r.end as f64)])
}

/// The `range` field of `text`: `[start, end]` on char boundaries inside the text.
fn range_of(text: &str, params: &Json) -> Result<Range<usize>, String> {
    let bad = || "the range must be [start, end] inside the text".to_string();
    let a = params.get("range").and_then(Json::array).ok_or_else(bad)?;
    let (s, e) = match a {
        [s, e] => (s.number().ok_or_else(bad)?, e.number().ok_or_else(bad)?),
        _ => return Err(bad()),
    };
    if s < 0.0 || e < s || (e as usize) > text.len() {
        return Err(bad());
    }
    let (s, e) = (s as usize, e as usize);
    if !text.is_char_boundary(s) || !text.is_char_boundary(e) {
        return Err(bad());
    }
    Ok(s..e)
}

/// Applies one `text` method to `s`, saving through `prefs`. Returns the JSON result, or an error message.
fn apply(s: &mut State, prefs: &mut Prefs, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "check_spelling" => {
            let text = text_of(params)?;
            let start = params.get("start").and_then(Json::number).unwrap_or(0.0).max(0.0) as usize;
            Ok(s.checker.check_spelling(&text, start).map_or(Json::Null, range_json))
        }
        "check" => {
            let text = text_of(params)?;
            let found = s.checker.check(&text, CheckingTypes::SPELLING);
            Ok(Json::Arr(found.into_iter().map(|r| range_json(r.range)).collect()))
        }
        "correction" => {
            let text = text_of(params)?;
            let r = range_of(&text, params)?;
            Ok(s.checker.correction(&text, r).map_or(Json::Null, Json::Str))
        }
        "guesses" => {
            let text = text_of(params)?;
            let r = range_of(&text, params)?;
            Ok(Json::Arr(s.checker.guesses(&text, r).into_iter().map(Json::Str).collect()))
        }
        "learn_word" => {
            let w = word_of(params)?;
            s.checker.learn_word(&w, prefs);
            Ok(Json::Null)
        }
        "unlearn_word" => {
            let w = word_of(params)?;
            s.checker.unlearn_word(&w, prefs);
            Ok(Json::Null)
        }
        "has_learned_word" => Ok(Json::Bool(s.checker.has_learned_word(&word_of(params)?))),
        "ignore_word" => {
            s.checker.ignore_word(&word_of(params)?);
            Ok(Json::Null)
        }
        "replacements" => Ok(Json::Arr(
            s.replacements
                .entries()
                .iter()
                .map(|e| Json::Obj(vec![("shortcut".into(), Json::Str(e.shortcut.clone())), ("phrase".into(), Json::Str(e.phrase.clone()))]))
                .collect(),
        )),
        "set_replacement" => {
            let (shortcut, phrase) = (field_str(params, "shortcut")?, field_str(params, "phrase")?);
            s.replacements.set(&shortcut, &phrase).map_err(|r| refusal(r).to_string())?;
            let _ = prefs.set(REPLACEMENTS_PREF, &s.replacements.to_line());
            Ok(Json::Bool(true))
        }
        "remove_replacement" => {
            let shortcut = field_str(params, "shortcut")?;
            let gone = s.replacements.remove(&shortcut);
            if gone {
                let _ = prefs.set(REPLACEMENTS_PREF, &s.replacements.to_line());
            }
            Ok(Json::Bool(gone))
        }
        "expand" => {
            let word = field_str(params, "word")?;
            Ok(s.replacements.expand(&word).map_or(Json::Null, |p| Json::Str(p.to_string())))
        }
        _ => Err(format!("text has no method {method}")),
    }
}

pub struct TextServices;

impl SystemService for TextServices {
    fn name(&self) -> &'static str {
        "text"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        with(|s| apply(s, &mut shell.prefs, method, params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: &[(&str, Json)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    fn s(v: &str) -> Json {
        Json::Str(v.into())
    }

    #[test]
    fn spelling_checks_flag_an_unknown_word_and_offer_a_correction() {
        let mut st = State::fresh();
        let mut p = Prefs::memory();
        let text = Json::Str("hello wrold".into());
        let params = obj(&[("text", text.clone()), ("start", Json::Num(0.0))]);
        assert_eq!(apply(&mut st, &mut p, "check_spelling", &params), Ok(Json::Arr(vec![Json::Num(6.0), Json::Num(11.0)])));
        let fix = obj(&[("text", text), ("range", Json::Arr(vec![Json::Num(6.0), Json::Num(11.0)]))]);
        // several words are one edit away from "wrold" (word, world, wold); the first in order is offered
        assert!(matches!(apply(&mut st, &mut p, "correction", &fix), Ok(Json::Str(_))));
        assert!(matches!(apply(&mut st, &mut p, "guesses", &fix), Ok(Json::Arr(v)) if v.contains(&s("world"))));
    }

    #[test]
    fn a_learned_word_is_not_flagged_and_survives_a_reload_from_the_settings_file() {
        let mut st = State::fresh();
        let mut p = Prefs::memory();
        apply(&mut st, &mut p, "learn_word", &obj(&[("word", s("Bifrostish"))])).unwrap();
        let check = obj(&[("text", s("Bifrostish")), ("start", Json::Num(0.0))]);
        assert_eq!(apply(&mut st, &mut p, "check_spelling", &check), Ok(Json::Null));
        let mut fresh = State::fresh();
        fresh.checker.load(&p);
        assert!(fresh.checker.has_learned_word("bifrostish"));
        // the learned words are stored under the checker's own key, one per line in the store
        assert_eq!(p.get_string("NativeSpellChecker.learnedWords"), Some("bifrostish".to_string()));
    }

    #[test]
    fn a_learned_word_with_a_space_is_refused() {
        let mut st = State::fresh();
        let mut p = Prefs::memory();
        assert!(apply(&mut st, &mut p, "learn_word", &obj(&[("word", s("two words"))])).is_err());
        assert!(apply(&mut st, &mut p, "learn_word", &obj(&[("word", s(""))])).is_err());
    }

    #[test]
    fn replacements_are_set_listed_expanded_and_removed() {
        let mut st = State::fresh();
        let mut p = Prefs::memory();
        apply(&mut st, &mut p, "set_replacement", &obj(&[("shortcut", s("omw")), ("phrase", s("On my way"))])).unwrap();
        apply(&mut st, &mut p, "set_replacement", &obj(&[("shortcut", s("brb")), ("phrase", s("be right back"))])).unwrap();
        assert_eq!(apply(&mut st, &mut p, "expand", &obj(&[("word", s("omw"))])), Ok(s("On my way")));
        assert_eq!(apply(&mut st, &mut p, "expand", &obj(&[("word", s("xyz"))])), Ok(Json::Null));
        let listed = apply(&mut st, &mut p, "replacements", &Json::Null).unwrap();
        assert_eq!(listed.array().map(<[Json]>::len), Some(2));
        assert_eq!(listed.array().unwrap()[0].get("shortcut"), Some(&s("brb")), "sorted by shortcut");
        assert_eq!(apply(&mut st, &mut p, "remove_replacement", &obj(&[("shortcut", s("brb"))])), Ok(Json::Bool(true)));
        assert_eq!(apply(&mut st, &mut p, "remove_replacement", &obj(&[("shortcut", s("brb"))])), Ok(Json::Bool(false)));
        assert_eq!(Replacements::from_line(p.get(REPLACEMENTS_PREF).unwrap()).len(), 1);
    }

    #[test]
    fn a_refused_replacement_names_the_problem() {
        let mut st = State::fresh();
        let mut p = Prefs::memory();
        let r = apply(&mut st, &mut p, "set_replacement", &obj(&[("shortcut", s("a b")), ("phrase", s("x"))]));
        assert_eq!(r, Err("a shortcut is one word".to_string()));
        assert!(st.replacements.is_empty());
    }

    #[test]
    fn unknown_methods_and_bad_ranges_are_errors_not_panics() {
        let mut st = State::fresh();
        let mut p = Prefs::memory();
        assert!(apply(&mut st, &mut p, "nope", &Json::Null).is_err());
        let bad = obj(&[("text", s("héllo")), ("range", Json::Arr(vec![Json::Num(0.0), Json::Num(2.0)]))]);
        assert!(apply(&mut st, &mut p, "correction", &bad).is_err(), "2 cuts the é in half");
    }
}
