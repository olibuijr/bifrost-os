//! Permission questions an app sends to a guardian, and the answers that come back.
//!
//! A [`PermissionQuestion`] asks a guardian for permission about a topic: a communication permission with one or more
//! contacts ([`CommunicationTopic`]), or permission to keep using the app after a significant update
//! ([`SignificantAppUpdateTopic`]). Each question offers a short list of choice ids and a default choice. The
//! [`AskCenter`] queue holds the questions that were sent and are still waiting, takes the chosen answer for one of
//! them, drops the ones that expired, and hands the answers back per topic, the way a response stream is read.
//! Errors use [`AskError`]. The model is pure: the caller passes the time in seconds and the question id (a UUID
//! in the caller's world), and the model reads no clock and sends nothing.

use std::collections::BTreeMap;

/// Most contacts one communication question names.
pub const MAX_HANDLES: usize = 32;
/// Most choices one question offers.
pub const MAX_CHOICES: usize = 8;
/// Most questions waiting for an answer at one time.
pub const MAX_PENDING: usize = 16;
/// Longest contact handle, in bytes.
pub const MAX_HANDLE_LEN: usize = 256;

/// What a question asks for. Each topic has a stable name, which routes the answers back to the right reader.
pub trait QuestionTopic {
    fn topic_name(&self) -> &'static str;
}

/// A communication permission with one or more contacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommunicationTopic;

impl QuestionTopic for CommunicationTopic {
    fn topic_name(&self) -> &'static str {
        "communication"
    }
}

/// Permission to keep using the app after a significant update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignificantAppUpdateTopic;

impl QuestionTopic for SignificantAppUpdateTopic {
    fn topic_name(&self) -> &'static str {
        "significant-app-update"
    }
}

/// Why a question could not be sent, or why an answer was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskError {
    Unknown,
    /// The communication limits are not turned on, so no question can be sent.
    CommunicationLimitsNotEnabled,
    /// Contact sync is not set up, so the handles cannot be resolved.
    ContactSyncNotSetup,
    /// The question is malformed, expired, a duplicate, or the answer is not one of its choices.
    InvalidQuestion,
    /// The queue is full or another system failure; the text says which.
    SystemError(String),
    NotAvailable,
}

impl AskError {
    /// An English description of the error.
    pub fn description(&self) -> String {
        match self {
            AskError::Unknown => "Unknown error.".to_string(),
            AskError::CommunicationLimitsNotEnabled => "Communication limits are not turned on.".to_string(),
            AskError::ContactSyncNotSetup => "Contact sync is not set up.".to_string(),
            AskError::InvalidQuestion => "The permission question is not valid.".to_string(),
            AskError::SystemError(why) => format!("System error: {}", why),
            AskError::NotAvailable => "Permission questions are not available.".to_string(),
        }
    }
}

/// A contact a communication question is about. Non-blank, at most [`MAX_HANDLE_LEN`] bytes, no control characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handle(String);

impl Handle {
    pub fn new(text: &str) -> Result<Handle, AskError> {
        if text.trim().is_empty() || text.len() > MAX_HANDLE_LEN || text.chars().any(|c| c.is_control()) {
            return Err(AskError::InvalidQuestion);
        }
        Ok(Handle(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn default_choices() -> Vec<String> {
    vec!["approve".to_string(), "decline".to_string()]
}

/// A question about a topic. It offers choices; the default is the one the guardian sees selected first.
/// The default for a new question is "decline", so a child never gets a yes by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionQuestion<T: QuestionTopic> {
    id: u64,
    topic: T,
    handles: Vec<Handle>,
    choices: Vec<String>,
    default_index: usize,
    expires_at: Option<u64>,
}

impl PermissionQuestion<CommunicationTopic> {
    /// A communication question about one to [`MAX_HANDLES`] contacts.
    pub fn communication(id: u64, handles: &[Handle], expires_at: Option<u64>) -> Result<Self, AskError> {
        if handles.is_empty() || handles.len() > MAX_HANDLES {
            return Err(AskError::InvalidQuestion);
        }
        Ok(PermissionQuestion {
            id,
            topic: CommunicationTopic,
            handles: handles.to_vec(),
            choices: default_choices(),
            default_index: 1,
            expires_at,
        })
    }

    pub fn handles(&self) -> &[Handle] {
        &self.handles
    }
}

impl PermissionQuestion<SignificantAppUpdateTopic> {
    /// A question about continuing to use the app after a significant update.
    pub fn significant_app_update(id: u64, expires_at: Option<u64>) -> Self {
        PermissionQuestion {
            id,
            topic: SignificantAppUpdateTopic,
            handles: Vec::new(),
            choices: default_choices(),
            default_index: 1,
            expires_at,
        }
    }
}

impl<T: QuestionTopic> PermissionQuestion<T> {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn topic(&self) -> &T {
        &self.topic
    }

    pub fn choices(&self) -> &[String] {
        &self.choices
    }

    pub fn default_choice(&self) -> &str {
        &self.choices[self.default_index]
    }

    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    /// Replaces the choices. Needs 1 to [`MAX_CHOICES`] ids, all non-blank and unique, and a default index inside them.
    pub fn with_choices(mut self, ids: &[&str], default_index: usize) -> Result<Self, AskError> {
        if ids.is_empty() || ids.len() > MAX_CHOICES || default_index >= ids.len() {
            return Err(AskError::InvalidQuestion);
        }
        for (i, id) in ids.iter().enumerate() {
            if id.trim().is_empty() || ids[..i].contains(id) {
                return Err(AskError::InvalidQuestion);
            }
        }
        self.choices = ids.iter().map(|s| s.to_string()).collect();
        self.default_index = default_index;
        Ok(self)
    }
}

/// The answer a guardian gave to one question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionResponse {
    pub question_id: u64,
    pub topic: &'static str,
    pub choice: String,
    pub answered_at: u64,
}

#[derive(Clone, Debug)]
struct Pending {
    topic: &'static str,
    choices: Vec<String>,
    expires_at: Option<u64>,
}

/// The queue of sent questions. Questions wait here until an answer comes, or until they expire.
#[derive(Clone, Debug, Default)]
pub struct AskCenter {
    pending: BTreeMap<u64, Pending>,
    answered: Vec<PermissionResponse>,
}

impl AskCenter {
    pub fn new() -> AskCenter {
        AskCenter::default()
    }

    /// Sends a question at `now`. Fails when it has expired already, its id is waiting, or the queue is full.
    pub fn ask<T: QuestionTopic>(&mut self, question: &PermissionQuestion<T>, now: u64) -> Result<(), AskError> {
        if let Some(t) = question.expires_at {
            if now >= t {
                return Err(AskError::InvalidQuestion);
            }
        }
        if self.pending.contains_key(&question.id) {
            return Err(AskError::InvalidQuestion);
        }
        if self.pending.len() >= MAX_PENDING {
            return Err(AskError::SystemError("too many questions are waiting".to_string()));
        }
        self.pending.insert(
            question.id,
            Pending {
                topic: question.topic.topic_name(),
                choices: question.choices.clone(),
                expires_at: question.expires_at,
            },
        );
        Ok(())
    }

    /// Records the guardian's choice for a waiting question. An expired question is dropped and refused. A choice
    /// that the question does not offer is refused and the question keeps waiting.
    pub fn answer(&mut self, question_id: u64, choice: &str, now: u64) -> Result<PermissionResponse, AskError> {
        let (topic, expired, valid) = match self.pending.get(&question_id) {
            None => return Err(AskError::InvalidQuestion),
            Some(p) => (
                p.topic,
                p.expires_at.map_or(false, |t| now >= t),
                p.choices.iter().any(|c| c.as_str() == choice),
            ),
        };
        if expired {
            let _ = self.pending.remove(&question_id);
            return Err(AskError::InvalidQuestion);
        }
        if !valid {
            return Err(AskError::InvalidQuestion);
        }
        let _ = self.pending.remove(&question_id);
        let response = PermissionResponse {
            question_id,
            topic,
            choice: choice.to_string(),
            answered_at: now,
        };
        self.answered.push(response.clone());
        Ok(response)
    }

    /// Drops the waiting questions that expired at or before `now`. Returns how many were dropped.
    pub fn expire(&mut self, now: u64) -> usize {
        let before = self.pending.len();
        self.pending.retain(|_, p| p.expires_at.map_or(true, |t| now < t));
        before - self.pending.len()
    }

    /// Takes the answers for one topic, in the order they came. Answers for other topics stay in the queue.
    pub fn take_responses(&mut self, topic: &str) -> Vec<PermissionResponse> {
        let all = std::mem::take(&mut self.answered);
        let (taken, kept): (Vec<PermissionResponse>, Vec<PermissionResponse>) =
            all.into_iter().partition(|r| r.topic == topic);
        self.answered = kept;
        taken
    }

    pub fn is_pending(&self, question_id: u64) -> bool {
        self.pending.contains_key(&question_id)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn answered_count(&self) -> usize {
        self.answered.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Handle {
        Handle::new(s).unwrap()
    }

    #[test]
    fn communication_question_needs_one_to_32_contacts_and_defaults_to_decline() {
        assert_eq!(
            PermissionQuestion::communication(1, &[], None).unwrap_err(),
            AskError::InvalidQuestion
        );
        let too_many = vec![h("kid@example.test"); 33];
        assert_eq!(
            PermissionQuestion::communication(1, &too_many, None).unwrap_err(),
            AskError::InvalidQuestion
        );
        let max = vec![h("kid@example.test"); 32];
        let q = PermissionQuestion::communication(1, &max, None).unwrap();
        assert_eq!(q.handles().len(), 32);
        assert_eq!(q.choices(), &["approve".to_string(), "decline".to_string()]);
        assert_eq!(q.default_choice(), "decline");
        assert_eq!(q.topic().topic_name(), "communication");
    }

    #[test]
    fn handle_rejects_blank_control_and_overlong_text() {
        assert_eq!(Handle::new("   ").unwrap_err(), AskError::InvalidQuestion);
        assert_eq!(Handle::new("a\nb").unwrap_err(), AskError::InvalidQuestion);
        assert_eq!(Handle::new(&"x".repeat(257)).unwrap_err(), AskError::InvalidQuestion);
        assert_eq!(Handle::new(&"x".repeat(256)).unwrap().as_str().len(), 256);
    }

    #[test]
    fn asked_question_is_answered_once_and_leaves_the_queue() {
        let mut center = AskCenter::new();
        let q = PermissionQuestion::communication(7, &[h("kid@example.test")], Some(500)).unwrap();
        center.ask(&q, 100).unwrap();
        assert!(center.is_pending(7));
        assert_eq!(center.pending_count(), 1);
        let r = center.answer(7, "approve", 120).unwrap();
        assert_eq!(
            r,
            PermissionResponse {
                question_id: 7,
                topic: "communication",
                choice: "approve".to_string(),
                answered_at: 120,
            }
        );
        assert!(!center.is_pending(7));
        assert_eq!(center.pending_count(), 0);
        assert_eq!(center.answered_count(), 1);
    }

    #[test]
    fn expired_question_is_refused_on_ask_and_on_answer() {
        let mut center = AskCenter::new();
        let q = PermissionQuestion::communication(3, &[h("kid@example.test")], Some(200)).unwrap();
        assert_eq!(center.ask(&q, 200).unwrap_err(), AskError::InvalidQuestion);
        center.ask(&q, 150).unwrap();
        assert_eq!(center.answer(3, "approve", 200).unwrap_err(), AskError::InvalidQuestion);
        assert!(!center.is_pending(3));
    }

    #[test]
    fn duplicate_ask_and_unknown_choice_are_refused_and_the_question_keeps_waiting() {
        let mut center = AskCenter::new();
        let q = PermissionQuestion::significant_app_update(1, None);
        center.ask(&q, 0).unwrap();
        assert_eq!(center.ask(&q, 0).unwrap_err(), AskError::InvalidQuestion);
        assert_eq!(center.answer(1, "maybe", 0).unwrap_err(), AskError::InvalidQuestion);
        assert!(center.is_pending(1));
        assert_eq!(center.answer(99, "approve", 0).unwrap_err(), AskError::InvalidQuestion);
    }

    #[test]
    fn take_responses_returns_one_topic_in_order_and_keeps_the_rest() {
        let mut center = AskCenter::new();
        let comm = PermissionQuestion::communication(1, &[h("kid@example.test")], None).unwrap();
        let update = PermissionQuestion::significant_app_update(2, None);
        center.ask(&comm, 10).unwrap();
        center.ask(&update, 10).unwrap();
        center.answer(1, "approve", 11).unwrap();
        center.answer(2, "decline", 12).unwrap();

        let comm_answers = center.take_responses("communication");
        assert_eq!(comm_answers.len(), 1);
        assert_eq!(comm_answers[0].choice, "approve");
        assert!(center.take_responses("communication").is_empty());

        let update_answers = center.take_responses("significant-app-update");
        assert_eq!(update_answers.len(), 1);
        assert_eq!(update_answers[0].choice, "decline");
        assert_eq!(update_answers[0].answered_at, 12);
        assert_eq!(center.answered_count(), 0);
    }

    #[test]
    fn queue_holds_at_most_sixteen_questions() {
        let mut center = AskCenter::new();
        for i in 0..MAX_PENDING as u64 {
            let q = PermissionQuestion::significant_app_update(i, None);
            center.ask(&q, 0).unwrap();
        }
        let extra = PermissionQuestion::significant_app_update(MAX_PENDING as u64, None);
        let err = center.ask(&extra, 0).unwrap_err();
        assert!(matches!(err, AskError::SystemError(_)));
        assert_eq!(center.pending_count(), MAX_PENDING);
    }

    #[test]
    fn expire_drops_only_questions_whose_deadline_has_passed() {
        let mut center = AskCenter::new();
        let a = PermissionQuestion::communication(1, &[h("a@example.test")], Some(50)).unwrap();
        let b = PermissionQuestion::communication(2, &[h("b@example.test")], Some(80)).unwrap();
        let c = PermissionQuestion::significant_app_update(3, None);
        center.ask(&a, 0).unwrap();
        center.ask(&b, 0).unwrap();
        center.ask(&c, 0).unwrap();
        assert_eq!(center.expire(60), 1);
        assert!(!center.is_pending(1));
        assert_eq!(center.expire(90), 1);
        assert!(center.is_pending(3));
        assert_eq!(center.pending_count(), 1);
    }

    #[test]
    fn custom_choices_need_unique_non_blank_ids_and_a_valid_default() {
        let base = PermissionQuestion::significant_app_update(1, None);
        let ok = base.clone().with_choices(&["yes", "no", "later"], 2).unwrap();
        assert_eq!(ok.default_choice(), "later");
        assert_eq!(ok.choices().len(), 3);
        assert_eq!(
            base.clone().with_choices(&["yes", "yes"], 0).unwrap_err(),
            AskError::InvalidQuestion
        );
        assert_eq!(base.clone().with_choices(&["yes"], 1).unwrap_err(), AskError::InvalidQuestion);
        assert_eq!(base.with_choices(&[" "], 0).unwrap_err(), AskError::InvalidQuestion);
    }
}
