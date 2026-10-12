//! Typed client for the shell's `text` service: spelling checks, learned and ignored words, and the text replacements
//! of Settings > Keyboard > Text Replacements. The shell runs the same checker as `bifrost_app::spellcheck`, so an
//! app asks the system for the answer and keeps no word list of its own. Each call returns the request id; the reply
//! comes back as `App::service_reply` (or `service::take_reply`). Ranges are byte ranges in the text.
//!
//! ```ignore
//! use bifrost_app::text_services;
//! text_services::check_spelling(ctx, "hello wrold", 0);   // reply: [6, 11], or null
//! text_services::correction(ctx, "hello wrold", 6..11);  // reply: "world", or null
//! text_services::expand(ctx, "omw");                      // reply: the phrase, or null
//! ```
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use std::ops::Range;

fn call(ctx: &Ctx, method: &str, fields: Vec<(&str, Json)>) -> u32 {
    let params = Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
    service::call(ctx, "text", method, &params)
}

fn text(s: &str) -> Json {
    Json::Str(s.to_string())
}

fn range(r: Range<usize>) -> Json {
    Json::Arr(vec![Json::Num(r.start as f64), Json::Num(r.end as f64)])
}

/// Asks for the first misspelled word at or after byte `start`. The reply is its range, or null.
pub fn check_spelling(ctx: &Ctx, text_in: &str, start: usize) -> u32 {
    call(ctx, "check_spelling", vec![("text", text(text_in)), ("start", Json::Num(start as f64))])
}

/// Asks for every misspelled word in `text_in`. The reply is a list of ranges.
pub fn check(ctx: &Ctx, text_in: &str) -> u32 {
    call(ctx, "check", vec![("text", text(text_in))])
}

/// Asks for the best correction of the word at `word` in `text_in`. The reply is the word, or null.
pub fn correction(ctx: &Ctx, text_in: &str, word: Range<usize>) -> u32 {
    call(ctx, "correction", vec![("text", text(text_in)), ("range", range(word))])
}

/// Asks for the corrections of the word at `word` in `text_in`. The reply is a list of words.
pub fn guesses(ctx: &Ctx, text_in: &str, word: Range<usize>) -> u32 {
    call(ctx, "guesses", vec![("text", text(text_in)), ("range", range(word))])
}

/// Teaches the system a word, so no app flags it again. Words with a space in them are refused.
pub fn learn_word(ctx: &Ctx, word: &str) -> u32 {
    call(ctx, "learn_word", vec![("word", text(word))])
}

/// Forgets a learned word.
pub fn unlearn_word(ctx: &Ctx, word: &str) -> u32 {
    call(ctx, "unlearn_word", vec![("word", text(word))])
}

/// Asks whether the word was learned. The reply is true or false.
pub fn has_learned_word(ctx: &Ctx, word: &str) -> u32 {
    call(ctx, "has_learned_word", vec![("word", text(word))])
}

/// Ignores a word for the rest of this app's session.
pub fn ignore_word(ctx: &Ctx, word: &str) -> u32 {
    call(ctx, "ignore_word", vec![("word", text(word))])
}

/// Asks for the text replacements. The reply is a list of `{"shortcut", "phrase"}` objects, sorted by shortcut.
pub fn replacements(ctx: &Ctx) -> u32 {
    call(ctx, "replacements", vec![])
}

/// Adds a text replacement, or changes the phrase of an existing shortcut. The reply is true, or an error.
pub fn set_replacement(ctx: &Ctx, shortcut: &str, phrase: &str) -> u32 {
    call(ctx, "set_replacement", vec![("shortcut", text(shortcut)), ("phrase", text(phrase))])
}

/// Removes a text replacement. The reply is true when one was removed.
pub fn remove_replacement(ctx: &Ctx, shortcut: &str) -> u32 {
    call(ctx, "remove_replacement", vec![("shortcut", text(shortcut))])
}

/// Asks what a whole word expands to. The reply is the phrase, or null.
pub fn expand(ctx: &Ctx, word: &str) -> u32 {
    call(ctx, "expand", vec![("word", text(word))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ToShell;

    fn test_ctx() -> Ctx {
        use crate::kit::{Font, Ui};
        use crate::proto::Configure;
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    /// Every service call sent so far, as (method, parsed params).
    fn sent(ctx: &Ctx) -> Vec<(String, Json)> {
        ctx.take_sent()
            .into_iter()
            .map(|m| match m {
                ToShell::ServiceCall { service, method, body, .. } => {
                    assert_eq!(service, "text");
                    (method, bifrost_core::json::parse(&body).unwrap())
                }
                other => panic!("not a service call: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn check_spelling_sends_the_text_and_the_start() {
        let ctx = test_ctx();
        check_spelling(&ctx, "hello wrold", 3);
        let calls = sent(&ctx);
        assert_eq!(calls[0].0, "check_spelling");
        assert_eq!(calls[0].1.get("text"), Some(&Json::Str("hello wrold".into())));
        assert_eq!(calls[0].1.get("start"), Some(&Json::Num(3.0)));
    }

    #[test]
    fn correction_and_replacements_send_their_ranges_and_shortcuts() {
        let ctx = test_ctx();
        correction(&ctx, "hello wrold", 6..11);
        set_replacement(&ctx, "omw", "On my way");
        replacements(&ctx);
        let calls = sent(&ctx);
        assert_eq!(calls[0].1.get("range"), Some(&Json::Arr(vec![Json::Num(6.0), Json::Num(11.0)])));
        assert_eq!(calls[1].0, "set_replacement");
        assert_eq!(calls[1].1.get("shortcut"), Some(&Json::Str("omw".into())));
        assert_eq!(calls[1].1.get("phrase"), Some(&Json::Str("On my way".into())));
        assert_eq!(calls[2], ("replacements".to_string(), Json::Obj(vec![])));
    }
}
