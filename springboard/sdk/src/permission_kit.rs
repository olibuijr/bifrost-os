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
