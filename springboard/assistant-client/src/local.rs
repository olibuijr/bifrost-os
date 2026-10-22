//! On-device intent matching: the assistant's way of understanding a typed or spoken request with no backend at all.
//! The text is matched against the catalogue of intents the person allowed. It needs every significant word of an
//! intent's title ("Create note" needs both `create` and `note`, in any order and either plural), prefers an intent whose
//! app is named, and takes the rest of the sentence (or a quoted phrase) as the value of the intent's first parameter.
//!
//! The result is the same [`Reply`] a backend gives: an intent call to confirm, or a question when a parameter is missing
//! or several apps fit equally. Nothing runs from here; the shell's confirmation gate still stands between a match and
//! the action. Text that matches nothing returns `None`, and the caller says so or asks a backend if one is chosen.
//!
//! This is keyword matching, not language understanding: it will not follow a request that rephrases the title.

use crate::intents::{disambiguation_question, missing_question, Call, Catalogue, Entry, Param, Reply};

/// Words that carry no meaning for matching and are never taken as a parameter value.
const STOP: [&str; 26] = [
    "a", "an", "the", "in", "on", "to", "for", "with", "of", "my", "please", "called", "named", "titled", "saying", "say", "that", "and", "it", "me", "i", "can",
    "you", "could", "would", "app",
];

/// Parameter types whose value is the first number in the rest of the sentence.
const NUMBER_TYPES: [&str; 5] = ["number", "int", "integer", "double", "float"];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(stem).collect()
}

/// Lower case, and a plural `s` dropped from words longer than three letters, so "notes" matches "note".
fn stem(word: &str) -> String {
    let lower = word.to_lowercase();
    if lower.len() > 3 && lower.ends_with('s') && !lower.ends_with("ss") {
        lower[..lower.len() - 1].to_string()
    } else {
        lower
    }
}

fn significant(text: &str) -> Vec<String> {
    words(text).into_iter().filter(|w| !STOP.contains(&w.as_str())).collect()
}

/// The first phrase in quotes (`"..."` or `'...'`). An opening quote must start the text or follow a space, so the
/// apostrophe of "don't" is not one.
pub fn quoted(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if (c == '"' || c == '\'') && (i == 0 || chars[i - 1].is_whitespace()) {
            let close = (i + 1..chars.len()).find(|&j| chars[j] == c && !matches!(chars.get(j + 1), Some(n) if n.is_alphanumeric()))?;
            let inside: String = chars[i + 1..close].iter().collect();
            let inside = inside.trim();
            return (!inside.is_empty()).then(|| inside.to_string());
        }
    }
    None
}

/// What the entry needs to be in the text, and how well the text names its app.
fn score(entry: &Entry, text_words: &[String]) -> Option<u32> {
    let required = significant(&entry.title);
    if required.is_empty() || !required.iter().all(|w| text_words.contains(w)) {
        return None;
    }
    let app_named = significant(&entry.app_name).iter().any(|w| text_words.contains(w));
    // a one-word title ("Play") would fit any sentence that has the word, so it needs its app named too
    if required.len() == 1 && !app_named {
        return None;
    }
    Some(required.len() as u32 * 2 + u32::from(app_named))
}

fn value_for(param: &Param, rest: &str) -> Option<String> {
    if NUMBER_TYPES.contains(&param.ty.as_str()) {
        return rest
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find(|t| !t.is_empty() && t.parse::<f64>().is_ok())
            .map(str::to_string);
    }
    let value = rest.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Matches `text` against `catalogue`. `None` when no intent fits.
pub fn match_text(text: &str, catalogue: &Catalogue) -> Option<Reply> {
    let text_words = words(text);
    let mut best: Vec<(&Entry, u32)> = Vec::new();
    for entry in &catalogue.entries {
        let Some(s) = score(entry, &text_words) else { continue };
        match best.first().map(|(_, top)| *top) {
            Some(top) if s < top => {}
            Some(top) if s == top => best.push((entry, s)),
            _ => best = vec![(entry, s)],
        }
    }
    let entry = match best.as_slice() {
        [] => return None,
        [(only, _)] => *only,
        several => return Some(Reply::Ask(disambiguation_question(&several.iter().map(|(e, _)| *e).collect::<Vec<_>>()))),
    };

    let matched: Vec<String> = significant(&entry.title).into_iter().chain(significant(&entry.app_name)).collect();
    let rest = quoted(text).unwrap_or_else(|| {
        text.split_whitespace()
            .filter(|token| {
                let w = words(token);
                !w.is_empty() && w.iter().all(|w| !matched.contains(w) && !STOP.contains(&w.as_str()))
            })
            .collect::<Vec<_>>()
            .join(" ")
    });

    let mut params: Vec<(String, String)> = Vec::new();
    let first = entry.params.iter().find(|p| !p.optional).or_else(|| entry.params.first());
    if let Some(param) = first {
        match value_for(param, &rest) {
            Some(value) => params.push((param.name.clone(), value)),
            None if !param.optional => return Some(Reply::Ask(missing_question(entry, param))),
            None => {}
        }
    }
    if let Some(missing) = entry.params.iter().find(|p| !p.optional && !params.iter().any(|(n, _)| n == &p.name)) {
        return Some(Reply::Ask(missing_question(entry, missing)));
    }
    Some(Reply::Intent(Call {
        app_id: entry.app_id.clone(),
        app_name: entry.app_name.clone(),
        intent_id: entry.intent_id.clone(),
        title: entry.title.clone(),
        params,
        summary: String::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intents::parse_param;

    fn entry(app_id: &str, app_name: &str, intent_id: &str, title: &str, params: &[&str]) -> Entry {
        Entry {
            app_id: app_id.into(),
            app_name: app_name.into(),
            intent_id: intent_id.into(),
            title: title.into(),
            description: String::new(),
            params: params.iter().map(|p| parse_param(p).unwrap()).collect(),
        }
    }

    fn catalogue() -> Catalogue {
        Catalogue {
            entries: vec![
                entry("notes", "Notes", "create-note", "Create note", &["text:string"]),
                entry("timer", "Timer", "start-timer", "Start timer", &["minutes:number"]),
                entry("music", "Music", "play", "Play", &[]),
                entry("reminders", "Reminders", "create-reminder", "Create reminder", &["text:string"]),
            ],
        }
    }

    fn call(reply: Option<Reply>) -> Call {
        match reply {
            Some(Reply::Intent(call)) => call,
            other => panic!("expected an intent, got {other:?}"),
        }
    }

    #[test]
    fn the_title_words_and_the_rest_of_the_sentence_make_a_call() {
        let c = call(match_text("create note Groceries for Friday", &catalogue()));
        assert_eq!((c.app_id.as_str(), c.intent_id.as_str()), ("notes", "create-note"));
        assert_eq!(c.params, vec![("text".to_string(), "Groceries Friday".to_string())]);
    }

    #[test]
    fn word_order_articles_and_plurals_do_not_matter() {
        let c = call(match_text("Please make... a note? create it: Buy milk", &catalogue()));
        assert_eq!(c.intent_id, "create-note");
        let c = call(match_text("start the timers 10", &catalogue()));
        assert_eq!(c.intent_id, "start-timer");
    }

    #[test]
    fn a_quoted_phrase_is_the_value_whole() {
        let c = call(match_text("Create a note called \"Buy milk and eggs\" please", &catalogue()));
        assert_eq!(c.params, vec![("text".to_string(), "Buy milk and eggs".to_string())]);
        assert_eq!(quoted("say 'hello there' now"), Some("hello there".to_string()));
        assert_eq!(quoted("don't quote me"), None);
        assert_eq!(quoted("\"\""), None);
    }

    #[test]
    fn a_number_parameter_takes_the_first_number() {
        let c = call(match_text("start a timer for 10 minutes", &catalogue()));
        assert_eq!(c.params, vec![("minutes".to_string(), "10".to_string())]);
        let c = call(match_text("start timer 2.5 min", &catalogue()));
        assert_eq!(c.params[0].1, "2.5");
    }

    #[test]
    fn a_missing_required_value_becomes_a_question() {
        assert_eq!(
            match_text("create note", &catalogue()),
            Some(Reply::Ask("What text should I use to create note in Notes?".to_string()))
        );
        assert!(matches!(match_text("start timer soon", &catalogue()), Some(Reply::Ask(_))), "no number in the sentence");
    }

    #[test]
    fn a_one_word_title_needs_its_app_named() {
        assert_eq!(match_text("play", &catalogue()), None);
        assert_eq!(match_text("can you play something nice", &catalogue()), None);
        let c = call(match_text("play music", &catalogue()));
        assert_eq!((c.app_id.as_str(), c.params.len()), ("music", 0));
    }

    #[test]
    fn incomplete_or_unrelated_text_matches_nothing() {
        assert_eq!(match_text("create", &catalogue()), None);
        assert_eq!(match_text("what is the weather like", &catalogue()), None);
        assert_eq!(match_text("", &catalogue()), None);
        assert_eq!(match_text("create note hi", &Catalogue::default()), None);
    }

    #[test]
    fn two_apps_with_the_same_intent_are_a_question_until_one_is_named() {
        let mut both = catalogue();
        both.entries.push(entry("lists", "Lists", "add-item", "Add item", &["text:string"]));
        both.entries.push(entry("todo", "Todo", "add-item", "Add item", &["text:string"]));
        match match_text("add item milk", &both) {
            Some(Reply::Ask(q)) => assert!(q.contains("Lists") && q.contains("Todo"), "{q}"),
            other => panic!("expected a question, got {other:?}"),
        }
        let c = call(match_text("add item milk in Todo", &both));
        assert_eq!(c.app_id, "todo");
        assert_eq!(c.params, vec![("text".to_string(), "milk".to_string())], "the app name is not part of the value");
    }

    #[test]
    fn the_more_specific_title_wins() {
        let mut c = catalogue();
        c.entries.push(entry("notes", "Notes", "create-note-list", "Create note list", &["text:string"]));
        assert_eq!(call(match_text("create note list errands", &c)).intent_id, "create-note-list");
        assert_eq!(call(match_text("create note errands", &c)).intent_id, "create-note");
    }

    #[test]
    fn the_call_reads_as_a_confirmation_card() {
        let c = call(match_text("create reminder call mum", &catalogue()));
        assert_eq!(crate::intents::card_text(&c), "Create reminder 'call mum' in Reminders?");
    }
}
