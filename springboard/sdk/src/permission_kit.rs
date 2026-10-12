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
