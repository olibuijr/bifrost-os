//! Permission requests for apps (the app side of the parent or guardian approval flow).
//!
//! The model lives in `bifrost_core::permission_request`: a question is built from a topic and choices, sent to a
//! center with the current time, and answered by the guardian. This module re-exports that surface so an app
//! imports one path.
//!
//! ```ignore
//! use bifrost_app::permission_kit::{Center, Availability, Question, SignificantAppUpdate, Answer};
//! let mut center = Center::new(Availability::new(true, true, true));
//! let id = center.ask(Question::yes_no(&SignificantAppUpdate), now_secs)?;
//! // later, in the app loop:
//! for r in center.responses_for(SignificantAppUpdate.topic_id()) { /* r.choice().answer() == Answer::Approve */ }
//! ```

pub use bifrost_core::permission_request::{
    Answer, Availability, Center, Choice, Communication, Error, Question, Response, SignificantAppUpdate, Topic,
    COMMUNICATION_TOPIC, SIGNIFICANT_APP_UPDATE_TOPIC,
};

/// The button an app puts in its UI to ask a guardian (the model in `bifrost_ui::permission_question`). It sends its
/// question once, reads the answer back from an `AskCenter`, and reports a [`ButtonState`] for the shell to draw.
pub use bifrost_ui::permission_question::{ButtonState, PermissionButton};

/// The question model the shell's permission sheet is built from (`springboard/src/shell/permission_sheet.rs`): a
/// question about a topic, the queue that holds it until the person answers, and the answer. An app that builds its own
/// question for a guardian uses the same types.
///
/// The shell's sheet for the privacy permissions is not an app-side API: an app asks through
/// `privacy_permissions::request` and `privacy_permissions::location_request`, and the person answers Allow or Don't
/// Allow on the sheet.
pub use bifrost_ui::permission_question::{AskCenter, AskError, PermissionQuestion, PermissionResponse, QuestionTopic};

use bifrost_core::json::Json;

/// The service the shell's request channel registers (`springboard/src/shell/services/permission_kit.rs`).
pub const SERVICE: &str = "permission";

/// The `ask` params for a question: its topic, handles, choices (`id`, `title`, `answer`), the default index and the
/// expiry, when it has one. Decoding is the shell's: it refuses what the center would refuse.
pub fn ask_params(question: &Question) -> Json {
    let choices: Vec<Json> = question
        .choices()
        .iter()
        .map(|c| {
            let answer = match c.answer() {
                Answer::Approve => "approve",
                Answer::Decline => "decline",
            };
            Json::Obj(vec![
                ("id".into(), Json::Str(c.id().into())),
                ("title".into(), Json::Str(c.title().into())),
                ("answer".into(), Json::Str(answer.into())),
            ])
        })
        .collect();
    let default = question
        .default_choice()
        .and_then(|d| question.choices().iter().position(|c| c.id() == d.id()))
        .unwrap_or(0);
    let mut params = vec![
        ("topic".into(), Json::Str(question.topic_id().into())),
        ("choices".into(), Json::Arr(choices)),
        ("default".into(), Json::Num(default as f64)),
    ];
    if !question.handles().is_empty() {
        params.push(("handles".into(), Json::Arr(question.handles().iter().map(|h| Json::Str(h.clone())).collect())));
    }
    if let Some(at) = question.expires_at() {
        params.push(("expires_at".into(), Json::Num(at as f64)));
    }
    Json::Obj(params)
}

/// Sends a question to the guardian's request center. The reply is `{id}`, the number the center gave it.
pub fn ask(ctx: &crate::runtime::Ctx, question: &Question) -> u32 {
    crate::service::call(ctx, SERVICE, "ask", &ask_params(question))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::permission_request::SignificantAppUpdate;

    #[test]
    fn a_yes_no_question_encodes_its_topic_choices_and_decline_default() {
        let params = ask_params(&Question::yes_no(&SignificantAppUpdate));
        assert_eq!(params.get("topic").and_then(Json::as_str), Some(bifrost_core::permission_request::SIGNIFICANT_APP_UPDATE_TOPIC));
        assert_eq!(params.get("default").and_then(Json::as_u64), Some(1));
        let choices = params.get("choices").and_then(Json::as_array).unwrap();
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].get("answer").and_then(Json::as_str), Some("approve"));
        assert!(params.get("expires_at").is_none(), "no expiry unless one is set");
    }

    #[test]
    fn an_expiry_is_encoded_when_the_question_has_one() {
        let question = Question::yes_no(&SignificantAppUpdate).with_expiration(500);
        assert_eq!(ask_params(&question).get("expires_at").and_then(Json::as_u64), Some(500));
    }
}
