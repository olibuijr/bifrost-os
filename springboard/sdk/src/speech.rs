//! Text to speech over the shell's `speech` service (`springboard/src/shell/services/speech.rs`), the BifrOSt counterpart
//! of the reference speech synthesizer: an utterance with a voice and a rate, spoken by the app's own speech session.
//!
//! ```ignore
//! // start an utterance (the reply is the request id; read the utterance id in App::service_reply)
//! speech::speak(ctx, &speech::Utterance::new("Hello").rate(180));
//! speech::stop(ctx);
//! // App::service_reply: speech::id_from_reply(&result), speech::speaking_from_reply(&result),
//! // speech::voices_from_reply(&result)
//! ```
//! One utterance per app: a new `speak` replaces the current one, and the shell ends it when the app quits. The engine is
//! `espeak-ng` (else `espeak`); with no engine installed the reply is the error "no speech engine is installed".
//! Gaps: no word or sentence events, no pitch or volume, no per-utterance language (the voice carries it), and voices are
//! listed as the engine names them.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name, as the shell registers it.
pub const SERVICE: &str = "speech";
/// The slowest and fastest rates, in words per minute, the engine takes.
pub const MIN_RATE: u32 = 80;
pub const MAX_RATE: u32 = 450;

/// Text to speak, with the voice and rate to speak it in. `None` takes the engine's default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub voice: Option<String>,
    pub rate: Option<u32>,
}

impl Utterance {
    pub fn new(text: &str) -> Self {
        Utterance { text: text.to_string(), ..Utterance::default() }
    }

    /// The voice, by its name from [`voices`].
    pub fn voice(mut self, name: &str) -> Self {
        self.voice = Some(name.to_string());
        self
    }

    /// The rate in words per minute, from [`MIN_RATE`] to [`MAX_RATE`] (the shell refuses other rates).
    pub fn rate(mut self, words_per_minute: u32) -> Self {
        self.rate = Some(words_per_minute);
        self
    }

    /// The `speak` call's parameters. Absent options are left out, so the engine's defaults apply.
    fn params(&self) -> Json {
        let mut fields = vec![("text".to_string(), Json::Str(self.text.clone()))];
        if let Some(v) = &self.voice {
            fields.push(("voice".to_string(), Json::Str(v.clone())));
        }
        if let Some(r) = self.rate {
            fields.push(("rate".to_string(), Json::Num(r as f64)));
        }
        Json::Obj(fields)
    }
}

/// A voice the engine knows: its name (what [`Utterance::voice`] takes) and its language code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Voice {
    pub name: String,
    pub language: String,
}

fn call(ctx: &Ctx, method: &str, params: &Json) -> u32 {
    service::call(ctx, SERVICE, method, params)
}

/// Starts `utterance` for this app, replacing its current one. The reply is `{id}`, read with [`id_from_reply`].
pub fn speak(ctx: &Ctx, utterance: &Utterance) -> u32 {
    call(ctx, "speak", &utterance.params())
}

/// Ends this app's utterance. The reply is `{stopped}`.
pub fn stop(ctx: &Ctx) -> u32 {
    call(ctx, "stop", &Json::Null)
}

/// Pauses this app's utterance. The reply is `{paused}`.
pub fn pause(ctx: &Ctx) -> u32 {
    call(ctx, "pause", &Json::Null)
}

/// Resumes a paused utterance. The reply is `{paused}`, false once it runs again.
pub fn continue_speaking(ctx: &Ctx) -> u32 {
    call(ctx, "continue", &Json::Null)
}

/// Asks whether this app is speaking. The reply is `{speaking, paused}`, read with [`speaking_from_reply`].
pub fn is_speaking(ctx: &Ctx) -> u32 {
    call(ctx, "is_speaking", &Json::Null)
}

/// Asks for the voices. The reply is `{voices, loading}`: `loading` is true until the listing is read, so ask again.
/// Read it with [`voices_from_reply`].
pub fn voices(ctx: &Ctx) -> u32 {
    call(ctx, "voices", &Json::Null)
}

fn body(result: &Result<Json, String>) -> Result<&Json, String> {
    result.as_ref().map_err(|e| e.clone())
}

/// Reads the utterance id from a `speak` reply.
pub fn id_from_reply(result: &Result<Json, String>) -> Result<u32, String> {
    body(result)?
        .get("id")
        .and_then(Json::number)
        .filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n <= u32::MAX as f64)
        .map(|n| n as u32)
        .ok_or_else(|| "speech: the reply has no id".to_string())
}

/// Reads `(speaking, paused)` from an `is_speaking` reply.
pub fn speaking_from_reply(result: &Result<Json, String>) -> Result<(bool, bool), String> {
    let b = body(result)?;
    match (b.get("speaking"), b.get("paused")) {
        (Some(Json::Bool(speaking)), Some(Json::Bool(paused))) => Ok((*speaking, *paused)),
        _ => Err("speech: the reply has no speaking flag".to_string()),
    }
}

/// Reads the voices from a `voices` reply. Entries without a name are skipped.
pub fn voices_from_reply(result: &Result<Json, String>) -> Result<Vec<Voice>, String> {
    let list = body(result)?.get("voices").and_then(Json::as_array).ok_or("speech: the reply has no voices")?;
    Ok(list
        .iter()
        .filter_map(|v| {
            let name = v.str_field("name")?;
            let language = v.str_field("language").unwrap_or_default();
            Some(Voice { name, language })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Result<Json, String> {
        Ok(bifrost_core::json::parse(text).unwrap())
    }

    #[test]
    fn an_utterance_sends_only_the_options_it_has() {
        let plain = Utterance::new("Hello").params();
        assert_eq!(plain.str_field("text").as_deref(), Some("Hello"));
        assert!(plain.get("voice").is_none() && plain.get("rate").is_none());

        let tuned = Utterance::new("Hi").voice("is").rate(160).params();
        assert_eq!(tuned.str_field("voice").as_deref(), Some("is"));
        assert_eq!(tuned.get("rate").and_then(Json::number), Some(160.0));
    }

    #[test]
    fn the_replies_are_read_back() {
        assert_eq!(id_from_reply(&reply(r#"{"id":4}"#)), Ok(4));
        assert!(id_from_reply(&reply(r#"{"id":0}"#)).is_err());
        assert_eq!(speaking_from_reply(&reply(r#"{"speaking":true,"paused":false}"#)), Ok((true, false)));
        assert!(speaking_from_reply(&reply(r#"{"speaking":1}"#)).is_err());
        assert_eq!(id_from_reply(&Err("no engine".into())), Err("no engine".to_string()));
    }

    #[test]
    fn the_voice_reply_gives_names_and_languages() {
        let r = reply(r#"{"voices":[{"name":"en-us","language":"en-us"},{"language":"is"},{"name":"is","language":"is"}],"loading":false}"#);
        assert_eq!(
            voices_from_reply(&r),
            Ok(vec![Voice { name: "en-us".into(), language: "en-us".into() }, Voice { name: "is".into(), language: "is".into() }])
        );
        assert!(voices_from_reply(&reply(r#"{"loading":true}"#)).is_err());
    }

    #[test]
    fn rates_are_the_engines_range() {
        assert!(MIN_RATE < MAX_RATE);
        assert_eq!(MIN_RATE, bifrost_core::speech_engine::MIN_RATE);
        assert_eq!(MAX_RATE, bifrost_core::speech_engine::MAX_RATE);
    }
}
