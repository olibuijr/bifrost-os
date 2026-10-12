//! Consent for voice-assistant and directions requests. Before the assistant or the directions service may send an
//! app a request, the user must allow it. The first request for an app and a service asks the user once, and the
//! answer is kept until the user revokes it or the app is removed; asking again does not change a decided answer.
//! The model shows no prompt and reads no settings: the caller shows the prompt with the usage text and passes the
//! answer back.

use std::collections::BTreeMap;

/// The two services that can send an app a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Service {
    /// Spoken and typed requests from the assistant.
    Assistant,
    /// Requests from the directions service that route to a place the app knows about.
    Directions,
}

impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Service::Assistant => "assistant",
            Service::Directions => "directions",
        }
    }
}

/// The user's answer for one app and service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NotDetermined,
    Authorized,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyApp,
    /// The usage text the prompt shows is empty; it is required before any request.
    MissingUsage,
}

/// The answers the user gave, per app and service. A missing entry is not determined.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Consent {
    answers: BTreeMap<(String, Service), bool>,
}

impl Consent {
    pub fn new() -> Consent {
        Consent::default()
    }

    pub fn status(&self, app: &str, service: Service) -> Status {
        match self.answers.get(&(app.to_string(), service)) {
            None => Status::NotDetermined,
            Some(true) => Status::Authorized,
            Some(false) => Status::Denied,
        }
    }

    /// Records the user's answer to the first request for this app and service. When the answer is already
    /// decided, `allow` is ignored. Returns the status after the call. `usage` is the text the prompt shows and must
    /// not be blank, whether or not the prompt is needed.
    pub fn request(&mut self, app: &str, service: Service, usage: &str, allow: bool) -> Result<Status, Error> {
        if app.is_empty() {
            return Err(Error::EmptyApp);
        }
        if usage.trim().is_empty() {
            return Err(Error::MissingUsage);
        }
        let key = (app.to_string(), service);
        if !self.answers.contains_key(&key) {
            self.answers.insert(key, allow);
        }
        Ok(self.status(app, service))
    }

    /// True only when the app has been authorized for this service.
    pub fn may_run(&self, app: &str, service: Service) -> bool {
        self.status(app, service) == Status::Authorized
    }

    /// Forgets the answer, so the next request asks again. Returns whether there was an answer.
    pub fn revoke(&mut self, app: &str, service: Service) -> bool {
        self.answers.remove(&(app.to_string(), service)).is_some()
    }

    /// Forgets every answer for an app and returns how many went.
    pub fn forget_app(&mut self, app: &str) -> usize {
        let before = self.answers.len();
        self.answers.retain(|k, _| k.0.as_str() != app);
        before - self.answers.len()
    }

    /// The apps authorized for a service, in name order.
    pub fn authorized(&self, service: Service) -> Vec<&str> {
        self.answers.iter().filter(|(k, v)| k.1 == service && **v).map(|(k, _)| k.0.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE: &str = "Reads your notes when you ask.";

    #[test]
    fn an_app_starts_not_determined() {
        let c = Consent::new();
        assert_eq!(c.status("notes", Service::Assistant), Status::NotDetermined);
        assert!(!c.may_run("notes", Service::Assistant));
    }

    #[test]
    fn an_allowed_app_may_run() {
        let mut c = Consent::new();
        assert_eq!(c.request("notes", Service::Assistant, USAGE, true), Ok(Status::Authorized));
        assert!(c.may_run("notes", Service::Assistant));
    }

    #[test]
    fn the_prompt_is_answered_once() {
        let mut c = Consent::new();
        c.request("notes", Service::Assistant, USAGE, true).unwrap();
        // a later request with a different answer keeps the first one
        assert_eq!(c.request("notes", Service::Assistant, USAGE, false), Ok(Status::Authorized));
        assert!(c.may_run("notes", Service::Assistant));
    }

    #[test]
    fn a_denied_app_may_not_run() {
        let mut c = Consent::new();
        assert_eq!(c.request("music", Service::Assistant, USAGE, false), Ok(Status::Denied));
        assert!(!c.may_run("music", Service::Assistant));
    }

    #[test]
    fn usage_text_and_app_id_are_required() {
        let mut c = Consent::new();
        assert_eq!(c.request("calendar", Service::Assistant, "   ", true), Err(Error::MissingUsage));
        assert_eq!(c.request("", Service::Directions, USAGE, true), Err(Error::EmptyApp));
        // a refused request records nothing
        assert_eq!(c.status("calendar", Service::Assistant), Status::NotDetermined);
    }

    #[test]
    fn each_service_is_answered_separately() {
        let mut c = Consent::new();
        c.request("notes", Service::Assistant, USAGE, true).unwrap();
        assert_eq!(c.request("notes", Service::Directions, "Shows places.", false), Ok(Status::Denied));
        assert!(c.may_run("notes", Service::Assistant));
        assert!(!c.may_run("notes", Service::Directions));
    }

    #[test]
    fn revoke_returns_the_app_to_not_determined() {
        let mut c = Consent::new();
        c.request("notes", Service::Assistant, USAGE, true).unwrap();
        assert!(c.revoke("notes", Service::Assistant));
        assert_eq!(c.status("notes", Service::Assistant), Status::NotDetermined);
        assert!(!c.revoke("notes", Service::Assistant));
        // after a revoke the next request asks again
        assert_eq!(c.request("notes", Service::Assistant, USAGE, false), Ok(Status::Denied));
    }

    #[test]
    fn authorized_lists_the_allowed_apps_in_name_order() {
        let mut c = Consent::new();
        c.request("notes", Service::Assistant, USAGE, true).unwrap();
        c.request("music", Service::Assistant, USAGE, true).unwrap();
        c.request("atlas", Service::Assistant, USAGE, false).unwrap();
        c.request("notes", Service::Directions, "Shows places.", true).unwrap();
        assert_eq!(c.authorized(Service::Assistant), vec!["music", "notes"]);
        assert_eq!(c.authorized(Service::Directions), vec!["notes"]);
    }

    #[test]
    fn forget_app_drops_every_service_of_that_app() {
        let mut c = Consent::new();
        c.request("notes", Service::Assistant, USAGE, true).unwrap();
        c.request("notes", Service::Directions, "Shows places.", true).unwrap();
        c.request("music", Service::Assistant, USAGE, false).unwrap();
        assert_eq!(c.forget_app("notes"), 2);
        assert_eq!(c.status("notes", Service::Assistant), Status::NotDetermined);
        assert_eq!(c.status("music", Service::Assistant), Status::Denied);
        assert_eq!(c.forget_app("notes"), 0);
        assert!(c.authorized(Service::Assistant).is_empty());
    }
}
