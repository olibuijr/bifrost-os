//! Permission requests from a child's app to a parent or guardian, and the answers that come back.
//!
//! Pure: no bus, no thread, no clock. The caller passes the current time in whole seconds, and it drains the
//! responses in its own loop. The app side builds a `Question` from a `Topic`, optional choices, an optional
//! default and an optional expiry, then sends it to a `Center`. The guardian side answers with `Center::respond`.
//!
//! Model, in the order the rules apply:
//! - The center checks what is switched on first: the whole service, the communication limits and the contact
//!   sync. A communication question needs the last two, a significant app update question needs only the service.
//! - The question must be well formed: at least one choice, unique choice ids, a default that names a choice, a
//!   non-empty topic, a communication topic with at least one handle, and an expiry after the time it is sent.
//! - A question gets a number at send time. Numbers start at 1 and are never reused.
//! - A question that has passed its expiry cannot be answered. A question left unanswered is dropped by `expire`.
//! - An answer names one of the question's choice ids. A wrong id keeps the question pending. A right one moves
//!   the question and its choice into the response queue.
//! - Responses are drained per topic, so an app reads only the answers to the topics it asked about.
//!
//! Default choice: a question built with `Question::yes_no` defaults to decline, so a question nobody answers
//! never grants an exception.

/// The topic id of a significant app update question.
pub const SIGNIFICANT_APP_UPDATE_TOPIC: &str = "significant-app-update";

/// The topic id of a communication question.
pub const COMMUNICATION_TOPIC: &str = "communication";

/// Error values for permission requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The request service is switched off.
    NotAvailable,
    /// A communication question was sent while communication limits are off.
    CommunicationLimitsNotEnabled,
    /// A communication question was sent while contact sync is not set up.
    ContactSyncNotSetup,
    /// The question is malformed: no choices, duplicate or unknown default, empty topic, no handles, or an expiry
    /// that is not after the send time.
    InvalidQuestion,
    /// No pending question has this number. It was answered, expired or never sent.
    UnknownQuestion,
    /// The answer names a choice the question does not offer. The question stays pending.
    UnknownChoice,
    /// The question passed its expiry before it was answered.
    Expired,
    /// An unexpected failure. Nothing in the model produces it.
    Unknown,
}

impl Error {
    /// A short English description of the error, for logs and the request sheet.
    pub fn description(&self) -> &'static str {
        match self {
            Error::NotAvailable => "Permission requests are not available on this system.",
            Error::CommunicationLimitsNotEnabled => "Communication limits are not turned on.",
            Error::ContactSyncNotSetup => "Contact sync is not set up.",
            Error::InvalidQuestion => "The permission question is not valid.",
            Error::UnknownQuestion => "No pending question has this number.",
            Error::UnknownChoice => "The question does not offer this choice.",
            Error::Expired => "The permission question has expired.",
            Error::Unknown => "An unknown error occurred.",
        }
    }
}

/// Which switches must be on before the center accepts a question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Availability {
    /// The request service as a whole.
    pub enabled: bool,
    /// Communication limits, needed by communication questions.
    pub communication_limits: bool,
    /// Contact sync, needed by communication questions.
    pub contact_sync: bool,
}

impl Availability {
    /// Builds the switch set from three flags.
    pub fn new(enabled: bool, communication_limits: bool, contact_sync: bool) -> Availability {
        Availability {
            enabled,
            communication_limits,
            contact_sync,
        }
    }
}

/// The answer a choice stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Grants the request.
    Approve,
    /// Refuses the request.
    Decline,
}

/// One answer a question offers. The id is what a response names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    id: String,
    title: String,
    answer: Answer,
}

impl Choice {
    /// Builds a choice with its id, its title shown to the guardian, and the answer it stands for.
    pub fn new(id: &str, title: &str, answer: Answer) -> Choice {
        Choice {
            id: id.to_string(),
            title: title.to_string(),
            answer,
        }
    }

    /// The system-preferred approve choice.
    pub fn approve() -> Choice {
        Choice::new("approve", "Approve", Answer::Approve)
    }

    /// The system-preferred decline choice.
    pub fn decline() -> Choice {
        Choice::new("decline", "Decline", Answer::Decline)
    }

    /// The choice id that a response names.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The title shown to the guardian.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The answer this choice stands for.
    pub fn answer(&self) -> Answer {
        self.answer
    }
}

/// What a question is about. The topic id groups responses when an app drains them.
pub trait Topic {
    /// The unique topic id.
    fn topic_id(&self) -> &str;

    /// The people a communication question is about. Other topics have none.
    fn handles(&self) -> &[String] {
        &[]
    }
}

/// A significant app update: the guardian decides whether the child keeps using the app after an update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SignificantAppUpdate;

impl Topic for SignificantAppUpdate {
    fn topic_id(&self) -> &str {
        SIGNIFICANT_APP_UPDATE_TOPIC
    }
}

/// A communication exception: the guardian decides whether the child may contact the listed people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Communication {
    handles: Vec<String>,
}

impl Communication {
    /// Builds a communication topic for one or more people.
    pub fn new(handles: Vec<String>) -> Communication {
        Communication { handles }
    }
}

impl Topic for Communication {
    fn topic_id(&self) -> &str {
        COMMUNICATION_TOPIC
    }

    fn handles(&self) -> &[String] {
        &self.handles
    }
}

/// A permission question an app sends. The number is set when the center accepts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    id: u64,
    topic_id: String,
    handles: Vec<String>,
    choices: Vec<Choice>,
    default_index: usize,
    expires_at: Option<u64>,
}

impl Question {
    /// Builds a question from a topic and its choices. The default is the first decline choice, or the first
    /// choice when none declines.
    pub fn new(topic: &dyn Topic, choices: Vec<Choice>) -> Question {
        let default_index = choices
            .iter()
            .position(|c| c.answer == Answer::Decline)
            .unwrap_or(0);
        Question {
            id: 0,
            topic_id: topic.topic_id().to_string(),
            handles: topic.handles().to_vec(),
            choices,
            default_index,
            expires_at: None,
        }
    }

    /// Builds a question with the two standard choices, approve and decline, and decline as the default.
    pub fn yes_no(topic: &dyn Topic) -> Question {
        Question::new(topic, vec![Choice::approve(), Choice::decline()])
    }

    /// Sets the index of the default choice. An index past the last choice makes the question invalid.
    pub fn with_default(mut self, index: usize) -> Question {
        self.default_index = index;
        self
    }

    /// Sets the time, in whole seconds, after which the question cannot be answered.
    pub fn with_expiration(mut self, at: u64) -> Question {
        self.expires_at = Some(at);
        self
    }

    /// The number the center gave the question. Zero until it is sent.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The topic id of the question.
    pub fn topic_id(&self) -> &str {
        &self.topic_id
    }

    /// The people a communication question is about.
    pub fn handles(&self) -> &[String] {
        &self.handles
    }

    /// The choices the guardian can pick from, in order.
    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    /// The default choice, if the default index names one.
    pub fn default_choice(&self) -> Option<&Choice> {
        self.choices.get(self.default_index)
    }

    /// The expiry time in whole seconds, if the question has one.
    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }
}

/// An answer to a question: the question and the choice the guardian picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    question: Question,
    choice: Choice,
}

impl Response {
    /// The original question.
    pub fn question(&self) -> &Question {
        &self.question
    }

    /// The choice the guardian picked.
    pub fn choice(&self) -> &Choice {
        &self.choice
    }
}

/// Holds the pending questions and the responses waiting to be drained.
#[derive(Clone, Debug)]
pub struct Center {
    availability: Availability,
    next_id: u64,
    pending: Vec<Question>,
    responses: Vec<Response>,
}

fn is_expired(question: &Question, now: u64) -> bool {
    matches!(question.expires_at, Some(t) if t <= now)
}

impl Center {
    /// A center with no pending questions, using the given switches.
    pub fn new(availability: Availability) -> Center {
        Center {
            availability,
            next_id: 1,
            pending: Vec::new(),
            responses: Vec::new(),
        }
    }

    /// Replaces the switch set. Questions already pending stay pending.
    pub fn set_availability(&mut self, availability: Availability) {
        self.availability = availability;
    }

    /// The questions that are waiting for an answer, oldest first.
    pub fn pending(&self) -> &[Question] {
        &self.pending
    }

    /// Accepts a question at time `now` and returns its number.
    pub fn ask(&mut self, mut question: Question, now: u64) -> Result<u64, Error> {
        if !self.availability.enabled {
            return Err(Error::NotAvailable);
        }
        if question.topic_id == COMMUNICATION_TOPIC {
            if !self.availability.communication_limits {
                return Err(Error::CommunicationLimitsNotEnabled);
            }
            if !self.availability.contact_sync {
                return Err(Error::ContactSyncNotSetup);
            }
        }
        if !is_well_formed(&question, now) {
            return Err(Error::InvalidQuestion);
        }
        let id = self.next_id;
        self.next_id += 1;
        question.id = id;
        self.pending.push(question);
        Ok(id)
    }

    /// Records the guardian's answer. An expired question is dropped and reports `Expired`. An unknown choice
    /// reports `UnknownChoice` and leaves the question pending.
    pub fn respond(&mut self, question_id: u64, choice_id: &str, now: u64) -> Result<(), Error> {
        let idx = self
            .pending
            .iter()
            .position(|q| q.id == question_id)
            .ok_or(Error::UnknownQuestion)?;
        if is_expired(&self.pending[idx], now) {
            self.pending.remove(idx);
            return Err(Error::Expired);
        }
        let choice = self.pending[idx]
            .choices
            .iter()
            .find(|c| c.id == choice_id)
            .cloned()
            .ok_or(Error::UnknownChoice)?;
        let question = self.pending.remove(idx);
        self.responses.push(Response { question, choice });
        Ok(())
    }

    /// Drops every pending question whose expiry is at or before `now`. Returns how many were dropped.
    pub fn expire(&mut self, now: u64) -> usize {
        let before = self.pending.len();
        self.pending.retain(|q| !is_expired(q, now));
        before - self.pending.len()
    }

    /// Removes and returns the responses to one topic. Responses to other topics stay queued.
    pub fn responses_for(&mut self, topic_id: &str) -> Vec<Response> {
        let mut taken = Vec::new();
        let mut kept = Vec::new();
        for r in self.responses.drain(..) {
            if r.question.topic_id == topic_id {
                taken.push(r);
            } else {
                kept.push(r);
            }
        }
        self.responses = kept;
        taken
    }
}

fn is_well_formed(question: &Question, now: u64) -> bool {
    if question.topic_id.is_empty() || question.choices.is_empty() {
        return false;
    }
    if question.default_index >= question.choices.len() {
        return false;
    }
    if question.topic_id == COMMUNICATION_TOPIC && question.handles.is_empty() {
        return false;
    }
    if let Some(t) = question.expires_at {
        if t <= now {
            return false;
        }
    }
    let n = question.choices.len();
    for i in 0..n {
        for j in (i + 1)..n {
            if question.choices[i].id == question.choices[j].id {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center() -> Center {
        Center::new(Availability::new(true, true, true))
    }

    #[test]
    fn yes_no_defaults_to_decline_and_numbers_increase() {
        let mut c = center();
        let q = Question::yes_no(&SignificantAppUpdate);
        assert_eq!(q.default_choice().unwrap().answer(), Answer::Decline);
        assert_eq!(c.ask(q.clone(), 100), Ok(1));
        assert_eq!(c.ask(q, 100), Ok(2));
        assert_eq!(c.pending().len(), 2);
        assert_eq!(c.pending()[0].id(), 1);
    }

    #[test]
    fn malformed_questions_are_rejected() {
        let mut c = center();
        let empty = Question::new(&SignificantAppUpdate, vec![]);
        assert_eq!(c.ask(empty, 100), Err(Error::InvalidQuestion));
        let dup = Question::new(&SignificantAppUpdate, vec![Choice::approve(), Choice::approve()]);
        assert_eq!(c.ask(dup, 100), Err(Error::InvalidQuestion));
        let bad_default = Question::yes_no(&SignificantAppUpdate).with_default(5);
        assert_eq!(c.ask(bad_default, 100), Err(Error::InvalidQuestion));
        let stale = Question::yes_no(&SignificantAppUpdate).with_expiration(100);
        assert_eq!(c.ask(stale, 100), Err(Error::InvalidQuestion));
        let no_handles = Question::yes_no(&Communication::new(vec![]));
        assert_eq!(c.ask(no_handles, 100), Err(Error::InvalidQuestion));
        assert!(c.pending().is_empty());
    }

    #[test]
    fn switches_block_questions_in_order() {
        let mut no_limits = Center::new(Availability::new(true, false, true));
        let comm = Question::yes_no(&Communication::new(vec!["a".to_string()]));
        assert_eq!(no_limits.ask(comm.clone(), 100), Err(Error::CommunicationLimitsNotEnabled));

        let mut no_sync = Center::new(Availability::new(true, true, false));
        assert_eq!(no_sync.ask(comm, 100), Err(Error::ContactSyncNotSetup));

        let mut off = Center::new(Availability::new(false, true, true));
        let update = Question::yes_no(&SignificantAppUpdate);
        assert_eq!(off.ask(update, 100), Err(Error::NotAvailable));

        // A significant app update needs only the service switch.
        let mut only_service = Center::new(Availability::new(true, false, false));
        let update = Question::yes_no(&SignificantAppUpdate);
        assert_eq!(only_service.ask(update, 100), Ok(1));
    }

    #[test]
    fn approve_moves_the_question_to_responses() {
        let mut c = center();
        let id = c.ask(Question::yes_no(&SignificantAppUpdate), 100).unwrap();
        assert_eq!(c.respond(id, "approve", 150), Ok(()));
        assert!(c.pending().is_empty());
        let got = c.responses_for(SIGNIFICANT_APP_UPDATE_TOPIC);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].question().id(), 1);
        assert_eq!(got[0].choice().answer(), Answer::Approve);
        assert!(c.responses_for(SIGNIFICANT_APP_UPDATE_TOPIC).is_empty());
        assert_eq!(c.respond(id, "approve", 150), Err(Error::UnknownQuestion));
    }

    #[test]
    fn unknown_choice_keeps_the_question_pending() {
        let mut c = center();
        let id = c.ask(Question::yes_no(&SignificantAppUpdate), 100).unwrap();
        assert_eq!(c.respond(id, "maybe", 110), Err(Error::UnknownChoice));
        assert_eq!(c.pending().len(), 1);
        assert_eq!(c.respond(id, "decline", 110), Ok(()));
    }

    #[test]
    fn expiry_blocks_answers_and_expire_drops_questions() {
        let mut c = center();
        let first = c
            .ask(Question::yes_no(&SignificantAppUpdate).with_expiration(200), 100)
            .unwrap();
        assert_eq!(c.respond(first, "decline", 200), Err(Error::Expired));
        assert!(c.pending().is_empty());

        c.ask(Question::yes_no(&SignificantAppUpdate).with_expiration(300), 100)
            .unwrap();
        assert_eq!(c.expire(299), 0);
        assert_eq!(c.pending().len(), 1);
        assert_eq!(c.expire(300), 1);
        assert!(c.pending().is_empty());
    }

    #[test]
    fn responses_are_drained_per_topic() {
        let mut c = center();
        let update = c.ask(Question::yes_no(&SignificantAppUpdate), 100).unwrap();
        let comm = c
            .ask(Question::yes_no(&Communication::new(vec!["friend".to_string()])), 100)
            .unwrap();
        assert_eq!(c.respond(comm, "approve", 110), Ok(()));
        assert_eq!(c.respond(update, "decline", 110), Ok(()));

        let comm_out = c.responses_for(COMMUNICATION_TOPIC);
        assert_eq!(comm_out.len(), 1);
        assert_eq!(comm_out[0].question().handles(), &["friend".to_string()]);
        assert_eq!(comm_out[0].choice().answer(), Answer::Approve);

        let update_out = c.responses_for(SIGNIFICANT_APP_UPDATE_TOPIC);
        assert_eq!(update_out.len(), 1);
        assert_eq!(update_out[0].choice().answer(), Answer::Decline);
        assert!(c.responses_for(COMMUNICATION_TOPIC).is_empty());
    }

    #[test]
    fn disabled_center_drops_no_state() {
        let mut c = center();
        c.set_availability(Availability::new(false, true, true));
        assert_eq!(c.ask(Question::yes_no(&SignificantAppUpdate), 100), Err(Error::NotAvailable));
        c.set_availability(Availability::new(true, true, true));
        assert_eq!(c.ask(Question::yes_no(&SignificantAppUpdate), 100), Ok(1));
    }
}
