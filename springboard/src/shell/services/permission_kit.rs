//! The `permission` service: the channel an app uses to send a PermissionKit question to the guardian's request
//! center (`bifrost_core::permission_request::Center`). The app calls `ask`; the question waits in the center until a
//! guardian answers it with `respond`, and the app reads the answers back with `responses`.
//!
//! Methods (params are a JSON object):
//! - `ask {topic, choices, default?, expires_at?, handles?}` returns `{id}`. `choices` is a list of
//!   `{id, title, answer}` with `answer` `approve` or `decline`. Only `significant-app-update` is accepted: a
//!   communication question needs communication limits and contact sync, which Settings does not offer yet.
//! - `responses {topic}` returns `{responses: [{question_id, choice_id, approve}]}` for this app's questions about the
//!   topic, and removes them from the queue.
//!
//! Each app's topics are stored under `app_id/topic`, so one app never reads another app's answers. The pending
//! sheet the guardian answers from is `shell/guardian_sheet.rs`: it lists `pending` and answers with `respond`, and the
//! shell's tick calls `expire` so an unanswered question goes away on time.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_core::permission_request::{
    Answer, Availability, Center, Choice, Question, Topic, SIGNIFICANT_APP_UPDATE_TOPIC,
};
use std::cell::RefCell;

/// The service name the shell registers.
pub const SERVICE: &str = "permission";

thread_local! {
    /// The one request center of the shell thread. Communication limits and contact sync are off until Settings has them.
    static CENTER: RefCell<Center> = RefCell::new(Center::new(Availability::new(true, false, false)));
}

/// The topic an app's question is filed under: the app id and the topic, so responses stay with their app.
struct ScopedTopic {
    id: String,
    handles: Vec<String>,
}

impl Topic for ScopedTopic {
    fn topic_id(&self) -> &str {
        &self.id
    }
    fn handles(&self) -> &[String] {
        &self.handles
    }
}

/// The key an app's answers are stored under.
pub fn scoped_topic(app_id: &str, topic: &str) -> String {
    format!("{app_id}/{topic}")
}

fn answer_of(name: &str) -> Option<Answer> {
    match name {
        "approve" => Some(Answer::Approve),
        "decline" => Some(Answer::Decline),
        _ => None,
    }
}

/// The answer's wire name.
pub fn answer_name(answer: Answer) -> &'static str {
    match answer {
        Answer::Approve => "approve",
        Answer::Decline => "decline",
    }
}

/// Builds the question from `ask` params and sends it to the center. Returns the question number.
pub fn ask(app_id: &str, params: &Json, now: u64) -> Result<u64, String> {
    let topic = params.str_field("topic").ok_or("ask needs a topic")?;
    if topic != SIGNIFICANT_APP_UPDATE_TOPIC {
        return Err(format!("topic {topic} is not supported yet"));
    }
    let mut choices = Vec::new();
    for item in params.get("choices").and_then(Json::as_array).ok_or("ask needs choices")? {
        let id = item.str_field("id").ok_or("a choice needs an id")?;
        let title = item.str_field("title").ok_or("a choice needs a title")?;
        let answer = item.str_field("answer").and_then(|a| answer_of(&a)).ok_or("a choice answer is approve or decline")?;
        choices.push(Choice::new(&id, &title, answer));
    }
    let handles = params
        .get("handles")
        .and_then(Json::as_array)
        .map(|items| items.iter().filter_map(|h| h.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let scoped = ScopedTopic { id: scoped_topic(app_id, &topic), handles };
    let mut question = Question::new(&scoped, choices);
    if let Some(default) = params.get("default").and_then(Json::as_u64) {
        question = question.with_default(default as usize);
    }
    if let Some(at) = params.get("expires_at").and_then(Json::as_u64) {
        question = question.with_expiration(at);
    }
    CENTER.with(|c| c.borrow_mut().ask(question, now)).map_err(|e| e.description().to_string())
}

/// The guardian's answer to a pending question. An unknown or expired question, or a choice the question does not
/// offer, is an error and leaves the question as it was.
pub fn respond(question_id: u64, choice_id: &str, now: u64) -> Result<(), String> {
    CENTER.with(|c| c.borrow_mut().respond(question_id, choice_id, now)).map_err(|e| e.description().to_string())
}

/// The answers to this app's questions about `topic`, taken out of the queue: `(question number, choice id, approve)`.
pub fn responses(app_id: &str, topic: &str) -> Vec<(u64, String, bool)> {
    let key = scoped_topic(app_id, topic);
    CENTER.with(|c| {
        c.borrow_mut()
            .responses_for(&key)
            .into_iter()
            .map(|r| (r.question().id(), r.choice().id().to_string(), r.choice().answer() == Answer::Approve))
            .collect()
    })
}

/// Drops the questions no one answered by `now`. Returns how many were dropped.
pub fn expire(now: u64) -> usize {
    CENTER.with(|c| c.borrow_mut().expire(now))
}

/// A question waiting for a guardian: its number, its scoped topic (`app/topic`) and its choices as `(id, title)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub id: u64,
    pub topic: String,
    pub choices: Vec<(String, String)>,
}

/// The questions waiting for a guardian, oldest first.
pub fn pending() -> Vec<Pending> {
    CENTER.with(|c| {
        c.borrow()
            .pending()
            .iter()
            .map(|q| Pending {
                id: q.id(),
                topic: q.topic_id().to_string(),
                choices: q.choices().iter().map(|c| (c.id().to_string(), c.title().to_string())).collect(),
            })
            .collect()
    })
}

/// Whether question `id` still waits for a guardian (not answered, not expired).
pub fn is_pending(id: u64) -> bool {
    CENTER.with(|c| c.borrow().pending().iter().any(|q| q.id() == id))
}

/// The `permission` service as the shell's service table sees it.
pub struct PermissionKit;

impl SystemService for PermissionKit {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        let now = t.max(0.0) as u64;
        match method {
            "ask" => {
                let id = ask(app_id, params, now)?;
                Ok(Json::Obj(vec![("id".into(), Json::Num(id as f64))]))
            }
            "responses" => {
                let topic = params.str_field("topic").ok_or("responses needs a topic")?;
                let items = responses(app_id, &topic)
                    .into_iter()
                    .map(|(question, choice, approve)| {
                        Json::Obj(vec![
                            ("question_id".into(), Json::Num(question as f64)),
                            ("choice_id".into(), Json::Str(choice)),
                            ("approve".into(), Json::Bool(approve)),
                        ])
                    })
                    .collect();
                Ok(Json::Obj(vec![("responses".into(), Json::Arr(items))]))
            }
            _ => Err(format!("unknown permission method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_params(topic: &str) -> Json {
        Json::Obj(vec![
            ("topic".into(), Json::Str(topic.into())),
            (
                "choices".into(),
                Json::Arr(vec![
                    Json::Obj(vec![
                        ("id".into(), Json::Str("approve".into())),
                        ("title".into(), Json::Str("Allow".into())),
                        ("answer".into(), Json::Str("approve".into())),
                    ]),
                    Json::Obj(vec![
                        ("id".into(), Json::Str("decline".into())),
                        ("title".into(), Json::Str("Not now".into())),
                        ("answer".into(), Json::Str("decline".into())),
                    ]),
                ]),
            ),
        ])
    }

    #[test]
    fn a_question_is_sent_and_waits_for_the_guardian() {
        let id = ask("is.example.app", &ask_params(SIGNIFICANT_APP_UPDATE_TOPIC), 100).unwrap();
        assert!(id >= 1);
        assert!(responses("is.example.app", SIGNIFICANT_APP_UPDATE_TOPIC).is_empty());
    }

    #[test]
    fn the_guardian_answer_reaches_only_the_app_that_asked() {
        let id = ask("is.example.owner", &ask_params(SIGNIFICANT_APP_UPDATE_TOPIC), 100).unwrap();
        respond(id, "approve", 101).unwrap();
        assert!(responses("is.example.other", SIGNIFICANT_APP_UPDATE_TOPIC).is_empty());
        assert_eq!(responses("is.example.owner", SIGNIFICANT_APP_UPDATE_TOPIC), vec![(id, "approve".to_string(), true)]);
        assert!(responses("is.example.owner", SIGNIFICANT_APP_UPDATE_TOPIC).is_empty(), "drained once");
    }

    #[test]
    fn a_pending_question_lists_its_choices_until_it_is_answered() {
        let id = ask("is.example.pending", &ask_params(SIGNIFICANT_APP_UPDATE_TOPIC), 100).unwrap();
        let waiting = pending().into_iter().find(|p| p.id == id).expect("queued for the guardian");
        assert_eq!(waiting.topic, scoped_topic("is.example.pending", SIGNIFICANT_APP_UPDATE_TOPIC));
        assert_eq!(waiting.choices, vec![("approve".to_string(), "Allow".to_string()), ("decline".to_string(), "Not now".to_string())]);
        assert!(is_pending(id));
        respond(id, "decline", 101).unwrap();
        assert!(!is_pending(id), "an answered question leaves the queue");
    }

    #[test]
    fn an_expired_question_leaves_the_queue_and_is_refused_an_answer() {
        let params = Json::Obj(vec![
            ("topic".into(), Json::Str(SIGNIFICANT_APP_UPDATE_TOPIC.into())),
            ("expires_at".into(), Json::Num(200.0)),
            ("choices".into(), ask_params(SIGNIFICANT_APP_UPDATE_TOPIC).get("choices").unwrap().clone()),
        ]);
        let id = ask("is.example.expiry", &params, 100).unwrap();
        assert!(is_pending(id));
        assert_eq!(expire(199), 0, "not yet due");
        assert_eq!(expire(200), 1);
        assert!(!is_pending(id));
        assert!(respond(id, "approve", 201).is_err(), "a late answer is refused");
    }

    #[test]
    fn a_decline_is_read_as_not_approved() {
        let id = ask("is.example.decline", &ask_params(SIGNIFICANT_APP_UPDATE_TOPIC), 100).unwrap();
        respond(id, "decline", 101).unwrap();
        assert_eq!(responses("is.example.decline", SIGNIFICANT_APP_UPDATE_TOPIC), vec![(id, "decline".to_string(), false)]);
    }

    #[test]
    fn communication_questions_are_refused_until_their_limits_exist() {
        let err = ask("is.example.comm", &ask_params("communication"), 100).unwrap_err();
        assert!(err.contains("not supported"));
    }

    #[test]
    fn a_question_with_an_unknown_choice_or_no_choices_is_refused() {
        assert!(respond(999_999, "approve", 100).is_err());
        let no_choices = Json::Obj(vec![
            ("topic".into(), Json::Str(SIGNIFICANT_APP_UPDATE_TOPIC.into())),
            ("choices".into(), Json::Arr(vec![])),
        ]);
        assert!(ask("is.example.empty", &no_choices, 100).is_err());
        let bad_answer = Json::Obj(vec![
            ("topic".into(), Json::Str(SIGNIFICANT_APP_UPDATE_TOPIC.into())),
            (
                "choices".into(),
                Json::Arr(vec![Json::Obj(vec![
                    ("id".into(), Json::Str("x".into())),
                    ("title".into(), Json::Str("X".into())),
                    ("answer".into(), Json::Str("maybe".into())),
                ])]),
            ),
        ]);
        assert!(ask("is.example.bad", &bad_answer, 100).is_err());
    }
}
