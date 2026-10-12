//! A model session for each request: the conversation's saved messages under the instructions of the request's skill,
//! fitted to the context budget. It is built on the shell's `model_session` (profiles, instructions) and
//! `context_window` (budget) models, which are pure: nothing here makes a request or touches the disk.

use crate::api::MAX_TOKENS;
use crate::model::Message;
use bifrost_app::controls::model_session::{DynamicInstructions, Profile, Session};
use bifrost_app::controls::transcript::{Entry, Segment};

/// Tokens the history and the instructions may take together. Older messages past this are not sent.
pub const BUDGET_TOKENS: usize = 150_000;
/// Tokens kept free for the reply.
pub const RESERVE_TOKENS: usize = MAX_TOKENS as usize;

/// The profile of a request: the skill's instructions under the skill's name. A general request (no skill) has no
/// instructions, so it goes out with no system prompt, as before.
pub fn profile(skill: Option<&str>, system: Option<&str>) -> Profile {
    let mut instructions = DynamicInstructions::new();
    if let Some(text) = system {
        instructions = instructions.fixed(text);
    }
    Profile::new(skill.unwrap_or("General"), instructions)
}

/// The session of one conversation under `profile`: its saved messages, with empty ones (a reply not yet started) left
/// out, as prompts and responses in order.
pub fn session(messages: &[Message], profile: Profile) -> Session {
    let history = messages
        .iter()
        .filter(|m| !m.text.trim().is_empty())
        .map(|m| {
            let segments = vec![Segment::Text(m.text.clone())];
            if m.user {
                Entry::Prompt(segments)
            } else {
                Entry::Response(segments)
            }
        })
        .collect();
    Session::with_history(profile, history)
}

/// What one request sends: the instructions (`None` when there are none), the newest history that fits as
/// `(is_user, text)` pairs in order, and how many older entries were left out to fit.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub system: Option<String>,
    pub messages: Vec<(bool, String)>,
    pub dropped: usize,
}

/// The request for `session` within the default budget.
pub fn request(session: &Session) -> Request {
    request_within(session, BUDGET_TOKENS, RESERVE_TOKENS)
}

/// The request for `session` within `budget` tokens, leaving `reserve` tokens for the reply.
pub fn request_within(session: &Session, budget: usize, reserve: usize) -> Request {
    let fitted = session.prepare(budget, reserve);
    let system = Some(session.instructions()).filter(|s| !s.is_empty());
    let messages = fitted
        .transcript
        .history()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Prompt(segments) => Some((true, text_of(segments))),
            Entry::Response(segments) => Some((false, text_of(segments))),
            _ => None,
        })
        .collect();
    Request { system, messages, dropped: fitted.dropped }
}

/// The text segments of an entry, joined. Other segments (attachments, structured content) are not sent by this app.
fn text_of(segments: &[Segment]) -> String {
    segments
        .iter()
        .filter_map(|s| match s {
            Segment::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(user: bool, text: &str) -> Message {
        Message { user, text: text.to_string() }
    }

    #[test]
    fn a_skill_request_carries_its_instructions_and_the_history_in_order() {
        let messages = vec![msg(true, "hi"), msg(false, "yo")];
        let s = session(&messages, profile(Some("Code"), Some("Be brief.")));
        let r = request(&s);
        assert_eq!(r.system.as_deref(), Some("Be brief."));
        assert_eq!(r.messages, vec![(true, "hi".to_string()), (false, "yo".to_string())]);
        assert_eq!(r.dropped, 0);
    }

    #[test]
    fn a_general_request_has_no_system_prompt() {
        let messages = vec![msg(true, "hi")];
        let r = request(&session(&messages, profile(None, None)));
        assert_eq!(r.system, None);
        assert_eq!(r.messages, vec![(true, "hi".to_string())]);
    }

    #[test]
    fn empty_messages_are_left_out_of_the_session() {
        // The newest assistant message is the empty placeholder of a reply not yet started.
        let messages = vec![msg(true, "a"), msg(false, ""), msg(true, "   ")];
        let r = request(&session(&messages, profile(None, None)));
        assert_eq!(r.messages, vec![(true, "a".to_string())]);
    }

    #[test]
    fn the_oldest_messages_go_first_when_the_budget_is_small() {
        // 400 characters are 100 estimated tokens each. Budget 250 with no reserve leaves room for two entries
        // counted from the newest end, but the kept history must start at a prompt: entry 1 (a response) is skipped,
        // so only the newest prompt is kept and two entries are dropped.
        let messages = vec![msg(true, &"a".repeat(400)), msg(false, &"b".repeat(400)), msg(true, &"c".repeat(400))];
        let r = request_within(&session(&messages, profile(None, None)), 250, 0);
        assert_eq!(r.messages, vec![(true, "c".repeat(400))]);
        assert_eq!(r.dropped, 2);
    }
}
