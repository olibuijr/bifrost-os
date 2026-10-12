//! Speech recognition requests, tasks and authorization (parity: Speech, `SFSpeechRecognizer`,
//! `SFSpeechRecognitionRequest`, `SFSpeechRecognitionTask`).
//!
//! A [`Recognizer`] for one locale starts a [`Task`] from a [`Request`]. The request names the audio source (live
//! buffers or a recorded file), whether partial results are wanted, a task hint, contextual phrases and whether the
//! recognition must stay on the device. A task moves `Starting` -> `Running` -> `Finishing` -> `Completed`, or
//! `Canceling` -> `Completed` when the caller cancels it. The recognizer that does the work reports into the task with
//! `mark_running`, `deliver`, `complete` and `fail`; the app reads the results as they arrive.
//!
//! Authorization follows `SFSpeechRecognizer.authorizationStatus`: `NotDetermined` until the person answers the
//! prompt, then `Authorized` or `Denied`. The system can set `Restricted` at any time, which no answer overrides.
//! Only an authorized recognizer starts tasks.
//!
//! Pure model with no I/O: the shell shows the prompt and reports the answer, and the audio and recognition engine
//! that produce the readings is separate. Results reuse [`crate::recognition`] (readings, time ranges, sessions).

use crate::recognition::{ContextualStrings, Session, Transcription};

/// Whether the person allowed speech recognition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationStatus {
    /// The prompt has not been answered yet.
    NotDetermined,
    /// The person refused. The app cannot ask again; only the person can change it in Settings.
    Denied,
    /// The system does not allow recognition (managed devices, for example).
    Restricted,
    /// The person allowed recognition.
    Authorized,
}

/// Why an authorization answer was not recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationError {
    /// The status is already settled; an answer is taken only while it is `NotDetermined`.
    AlreadyAnswered(AuthorizationStatus),
}

/// The speech recognition authorization for the device. Consent is asked once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    status: AuthorizationStatus,
}

impl Default for Authorization {
    fn default() -> Self {
        Authorization { status: AuthorizationStatus::NotDetermined }
    }
}

impl Authorization {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> AuthorizationStatus {
        self.status
    }

    /// Records the answer to the prompt. Only an undetermined status takes an answer; a later one is refused and
    /// the stored value stands.
    pub fn answer(&mut self, granted: bool) -> Result<AuthorizationStatus, AuthorizationError> {
        if self.status != AuthorizationStatus::NotDetermined {
            return Err(AuthorizationError::AlreadyAnswered(self.status));
        }
        self.status = if granted { AuthorizationStatus::Authorized } else { AuthorizationStatus::Denied };
        Ok(self.status)
    }

    /// The system restricts recognition. Overrides any earlier answer.
    pub fn restrict(&mut self) {
        self.status = AuthorizationStatus::Restricted;
    }
}

/// Where the audio comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Live audio buffers the app appends while the task runs (`SFSpeechAudioBufferRecognitionRequest`).
    Buffer,
    /// A recorded file at this path (`SFSpeechURLRecognitionRequest`).
    Url(String),
}

/// What the recognized speech is for. Helps the recognizer pick its vocabulary, as `SFSpeechRecognitionTaskHint`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskHint {
    #[default]
    Unspecified,
    /// Free text, such as a message or a note.
    Dictation,
    /// A short query.
    Search,
    /// A yes or no style answer.
    Confirmation,
}

/// One recognition request. Built with `new` and the `with_*` setters; defaults match the reference: partial results
/// on, no hint, recognition may use the network, no interaction id and no contextual phrases.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub source: Source,
    pub should_report_partial_results: bool,
    pub task_hint: TaskHint,
    pub requires_on_device_recognition: bool,
    pub interaction_identifier: Option<String>,
    contextual: ContextualStrings,
}

impl Request {
    pub fn new(source: Source) -> Self {
        Request {
            source,
            should_report_partial_results: true,
            task_hint: TaskHint::Unspecified,
            requires_on_device_recognition: false,
            interaction_identifier: None,
            contextual: ContextualStrings::new(),
        }
    }

    pub fn with_partial_results(mut self, on: bool) -> Self {
        self.should_report_partial_results = on;
        self
    }

    pub fn with_task_hint(mut self, hint: TaskHint) -> Self {
        self.task_hint = hint;
        self
    }

    pub fn with_on_device(mut self, required: bool) -> Self {
        self.requires_on_device_recognition = required;
        self
    }

    /// Blank identifiers are stored as none.
    pub fn with_interaction_identifier(mut self, id: &str) -> Self {
        let id = id.trim();
        self.interaction_identifier = if id.is_empty() { None } else { Some(id.into()) };
        self
    }

    pub fn with_contextual_strings(mut self, strings: ContextualStrings) -> Self {
        self.contextual = strings;
        self
    }

    pub fn contextual_strings(&self) -> &ContextualStrings {
        &self.contextual
    }
}

/// Why a task could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    /// The recognizer is not authorized. Carries the status the person left it in.
    NotAuthorized(AuthorizationStatus),
    /// The request requires on-device recognition and this device has no on-device model for the locale.
    OnDeviceUnavailable,
}

/// The recognizer for one locale: its authorization and whether on-device recognition is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recognizer {
    locale: String,
    authorization: Authorization,
    on_device_available: bool,
}

impl Recognizer {
    /// `locale` is a BCP 47 tag such as `en-GB`. It is stored as given; the speech model inventory checks it.
    pub fn new(locale: &str) -> Self {
        Recognizer { locale: locale.trim().into(), authorization: Authorization::new(), on_device_available: false }
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn authorization_status(&self) -> AuthorizationStatus {
        self.authorization.status()
    }

    /// True when tasks can start: the recognizer is authorized.
    pub fn is_available(&self) -> bool {
        self.authorization.status() == AuthorizationStatus::Authorized
    }

    pub fn answer_authorization(&mut self, granted: bool) -> Result<AuthorizationStatus, AuthorizationError> {
        self.authorization.answer(granted)
    }

    pub fn restrict_authorization(&mut self) {
        self.authorization.restrict();
    }

    /// Set by the speech model inventory when the locale's model is installed on the device.
    pub fn set_on_device_available(&mut self, available: bool) {
        self.on_device_available = available;
    }

    pub fn supports_on_device_recognition(&self) -> bool {
        self.on_device_available
    }

    /// Starts a task for the request. Refused unless authorized, and refused when on-device recognition is required
    /// but not installed.
    pub fn start(&self, request: Request) -> Result<Task, StartError> {
        match self.authorization.status() {
            AuthorizationStatus::Authorized => {}
            other => return Err(StartError::NotAuthorized(other)),
        }
        if request.requires_on_device_recognition && !self.on_device_available {
            return Err(StartError::OnDeviceUnavailable);
        }
        Ok(Task::new(request))
    }
}

/// Where a task stands, as `SFSpeechRecognitionTaskState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Starting,
    Running,
    /// The app ended the audio; results still arrive until the task completes.
    Finishing,
    Canceling,
    Completed,
}

/// How a task ended badly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskError {
    Canceled,
    Failed(String),
}

/// One recognition task: its request, state, results so far and the error it ended with, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    request: Request,
    state: TaskState,
    session: Session,
    error: Option<TaskError>,
}

impl Task {
    fn new(request: Request) -> Self {
        Task { request, state: TaskState::Starting, session: Session::new(), error: None }
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn is_finishing(&self) -> bool {
        self.state == TaskState::Finishing
    }

    pub fn is_cancelled(&self) -> bool {
        self.state == TaskState::Canceling || matches!(self.error, Some(TaskError::Canceled))
    }

    pub fn error(&self) -> Option<&TaskError> {
        self.error.as_ref()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The text recognized so far, with the partial reading at the end.
    pub fn text(&self) -> String {
        self.session.text()
    }

    /// The recognizer has started on the audio. Returns false unless the task was starting.
    pub fn mark_running(&mut self) -> bool {
        if self.state != TaskState::Starting {
            return false;
        }
        self.state = TaskState::Running;
        true
    }

    /// Adds a result. Accepted while running or finishing. Contextual phrases are applied first. A partial result is
    /// dropped when the request did not ask for partial results; a final one is always kept.
    pub fn deliver(&mut self, mut result: Transcription) -> bool {
        if !matches!(self.state, TaskState::Running | TaskState::Finishing) {
            return false;
        }
        if !self.request.should_report_partial_results && !result.is_final {
            return false;
        }
        result.apply_context(&self.request.contextual);
        self.session.push(result);
        true
    }

    /// The app ended the audio. Only a running task finishes.
    pub fn finish(&mut self) -> bool {
        if self.state != TaskState::Running {
            return false;
        }
        self.state = TaskState::Finishing;
        true
    }

    /// The app cancels. Any task that has not completed can be canceled.
    pub fn cancel(&mut self) -> bool {
        match self.state {
            TaskState::Starting | TaskState::Running | TaskState::Finishing => {
                self.state = TaskState::Canceling;
                true
            }
            TaskState::Canceling | TaskState::Completed => false,
        }
    }

    /// The recognizer is done. A task that was canceled completes with `TaskError::Canceled`.
    pub fn complete(&mut self) -> bool {
        self.end(None)
    }

    /// The recognizer failed. A blank reason is replaced by a generic one. A task that was canceled keeps the
    /// cancel error.
    pub fn fail(&mut self, reason: &str) -> bool {
        let reason = reason.trim();
        let reason = if reason.is_empty() { "Recognition failed" } else { reason };
        self.end(Some(TaskError::Failed(reason.into())))
    }

    fn end(&mut self, failure: Option<TaskError>) -> bool {
        if self.state == TaskState::Completed {
            return false;
        }
        if self.error.is_none() {
            self.error = if self.state == TaskState::Canceling {
                Some(TaskError::Canceled)
            } else {
                failure
            };
        }
        self.state = TaskState::Completed;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognition::Alternative;

    fn final_result(text: &str, start: f64, end: f64) -> Transcription {
        Transcription::new(start..end, true, vec![Alternative::new(text, 0.9)])
    }

    fn partial_result(text: &str, start: f64, end: f64) -> Transcription {
        Transcription::new(start..end, false, vec![Alternative::new(text, 0.5)])
    }

    fn authorized(locale: &str) -> Recognizer {
        let mut r = Recognizer::new(locale);
        assert_eq!(r.answer_authorization(true), Ok(AuthorizationStatus::Authorized));
        r
    }

    #[test]
    fn the_first_answer_is_kept_and_a_second_one_is_refused() {
        let mut a = Authorization::new();
        assert_eq!(a.status(), AuthorizationStatus::NotDetermined);
        assert_eq!(a.answer(false), Ok(AuthorizationStatus::Denied));
        assert_eq!(a.answer(true), Err(AuthorizationError::AlreadyAnswered(AuthorizationStatus::Denied)));
        assert_eq!(a.status(), AuthorizationStatus::Denied);
    }

    #[test]
    fn restriction_overrides_an_answer() {
        let mut a = Authorization::new();
        a.answer(true).unwrap();
        a.restrict();
        assert_eq!(a.status(), AuthorizationStatus::Restricted);
    }

    #[test]
    fn a_recognizer_refuses_tasks_until_it_is_authorized() {
        let mut r = Recognizer::new(" en-GB ");
        assert_eq!(r.locale(), "en-GB");
        assert!(!r.is_available());
        let refused = r.start(Request::new(Source::Buffer));
        assert_eq!(refused.err(), Some(StartError::NotAuthorized(AuthorizationStatus::NotDetermined)));
        r.answer_authorization(false).unwrap();
        let refused = r.start(Request::new(Source::Buffer));
        assert_eq!(refused.err(), Some(StartError::NotAuthorized(AuthorizationStatus::Denied)));
    }

    #[test]
    fn an_on_device_request_needs_an_installed_model() {
        let mut r = authorized("en-US");
        let request = Request::new(Source::Buffer).with_on_device(true);
        assert_eq!(r.start(request.clone()).err(), Some(StartError::OnDeviceUnavailable));
        r.set_on_device_available(true);
        assert!(r.supports_on_device_recognition());
        assert!(r.start(request).is_ok());
    }

    #[test]
    fn request_defaults_match_the_reference() {
        let r = Request::new(Source::Url("/tmp/a.wav".into()));
        assert!(r.should_report_partial_results);
        assert_eq!(r.task_hint, TaskHint::Unspecified);
        assert!(!r.requires_on_device_recognition);
        assert_eq!(r.interaction_identifier, None);
        assert_eq!(r.contextual_strings().phrases().count(), 0);
        let r = r.with_interaction_identifier("  ");
        assert_eq!(r.interaction_identifier, None);
        let r = r.with_interaction_identifier("note-42");
        assert_eq!(r.interaction_identifier.as_deref(), Some("note-42"));
    }

    #[test]
    fn a_task_runs_from_start_to_a_completed_final_text() {
        let mut task = authorized("en-US").start(Request::new(Source::Buffer)).unwrap();
        assert_eq!(task.state(), TaskState::Starting);
        assert!(!task.deliver(partial_result("too early", 0.0, 0.5)), "results wait for the task to run");
        assert!(task.mark_running());
        assert!(task.deliver(partial_result("hel", 0.0, 0.4)));
        assert!(task.deliver(final_result("hello", 0.0, 0.8)));
        assert!(task.finish());
        assert!(task.is_finishing());
        assert!(task.deliver(final_result("hello world", 0.8, 1.5)));
        assert!(task.complete());
        assert_eq!(task.state(), TaskState::Completed);
        assert_eq!(task.error(), None);
        assert!(!task.is_cancelled());
        assert_eq!(task.text(), "hello hello world");
        assert!(!task.complete(), "a completed task cannot complete again");
    }

    #[test]
    fn partial_results_can_be_turned_off() {
        let request = Request::new(Source::Buffer).with_partial_results(false);
        let mut task = authorized("en-US").start(request).unwrap();
        task.mark_running();
        assert!(!task.deliver(partial_result("hel", 0.0, 0.4)));
        assert!(task.deliver(final_result("hello", 0.0, 0.8)));
        assert_eq!(task.session().results().len(), 1);
    }

    #[test]
    fn a_canceled_task_completes_with_the_cancel_error() {
        let mut task = authorized("en-US").start(Request::new(Source::Buffer)).unwrap();
        task.mark_running();
        assert!(task.cancel());
        assert_eq!(task.state(), TaskState::Canceling);
        assert!(task.is_cancelled());
        assert!(!task.deliver(final_result("late", 0.0, 1.0)), "a canceled task takes no results");
        assert!(!task.cancel());
        assert!(task.complete());
        assert_eq!(task.error(), Some(&TaskError::Canceled));
        assert!(!task.fail("network down"), "a completed task cannot fail");
    }

    #[test]
    fn a_failure_keeps_its_reason_and_a_cancel_wins_over_it() {
        let mut failed = authorized("en-US").start(Request::new(Source::Buffer)).unwrap();
        failed.mark_running();
        assert!(failed.fail("   "));
        assert_eq!(failed.error(), Some(&TaskError::Failed("Recognition failed".into())));
        assert!(!failed.is_cancelled());

        let mut canceled = authorized("en-US").start(Request::new(Source::Buffer)).unwrap();
        canceled.mark_running();
        canceled.cancel();
        assert!(canceled.fail("late error"));
        assert_eq!(canceled.error(), Some(&TaskError::Canceled));
    }

    #[test]
    fn finish_only_applies_to_a_running_task() {
        let mut task = authorized("en-US").start(Request::new(Source::Buffer)).unwrap();
        assert!(!task.finish(), "a starting task cannot finish");
        task.mark_running();
        assert!(task.finish());
        assert!(!task.finish());
    }

    #[test]
    fn contextual_phrases_are_applied_to_delivered_results() {
        let mut strings = ContextualStrings::new();
        strings.add("devices", "Lamp");
        let request = Request::new(Source::Buffer).with_contextual_strings(strings);
        let mut task = authorized("en-US").start(request).unwrap();
        task.mark_running();
        let close = Transcription::new(
            0.0..2.0,
            true,
            vec![Alternative::new("open the lampshade", 0.7), Alternative::new("open the lamp", 0.62)],
        );
        assert!(task.deliver(close));
        assert_eq!(task.session().final_text(), "open the lamp");
    }
}
