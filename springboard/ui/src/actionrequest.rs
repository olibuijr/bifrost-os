//! The request behind one chosen file action: the action, the items it runs on, and the single outcome the hosted
//! view reports back. The view shows its own UI for the request, may show an error in place of its content, and
//! must finish the request by completing it or cancelling it. Once finished, a request accepts no more changes.
use crate::fileaction::ActionId;

/// How a request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    /// The person dismissed the action before it finished.
    UserCancelled,
    /// The action failed; the message is shown to the person by the browser.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// The request already has an outcome, which never changes.
    Finished(Outcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRequest {
    action: ActionId,
    items: Vec<String>,
    error: Option<String>,
    outcome: Option<Outcome>,
}

impl ActionRequest {
    /// A pending request for `action` on `items` (item identifiers, in selection order).
    pub fn new(action: ActionId, items: Vec<String>) -> Self {
        Self { action, items, error: None, outcome: None }
    }

    pub fn action(&self) -> &ActionId {
        &self.action
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The error the view is showing in place of its content, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    pub fn is_pending(&self) -> bool {
        self.outcome.is_none()
    }

    /// Shows `message` in place of the view's content and keeps the request pending, so the person can still
    /// retry or cancel it.
    pub fn show_error(&mut self, message: &str) -> Result<(), RequestError> {
        self.ensure_pending()?;
        self.error = Some(message.to_string());
        Ok(())
    }

    /// Finishes the request as completed.
    pub fn complete(&mut self) -> Result<(), RequestError> {
        self.finish(Outcome::Completed)
    }

    /// Finishes the request: `None` when the person cancelled, `Some(message)` when the action failed.
    pub fn cancel(&mut self, failure: Option<&str>) -> Result<(), RequestError> {
        let outcome = match failure {
            None => Outcome::UserCancelled,
            Some(message) => Outcome::Failed(message.to_string()),
        };
        self.finish(outcome)
    }

    fn ensure_pending(&self) -> Result<(), RequestError> {
        match &self.outcome {
            Some(done) => Err(RequestError::Finished(done.clone())),
            None => Ok(()),
        }
    }

    fn finish(&mut self, outcome: Outcome) -> Result<(), RequestError> {
        self.ensure_pending()?;
        self.outcome = Some(outcome);
        self.error = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ActionRequest {
        let action = ActionId::new("org.example.notes", "export-pdf").unwrap();
        ActionRequest::new(action, vec!["item-2".to_string(), "item-1".to_string()])
    }

    #[test]
    fn a_new_request_is_pending_and_keeps_its_items_in_order() {
        let r = request();
        assert!(r.is_pending());
        assert_eq!(r.outcome(), None);
        assert_eq!(r.action().name(), "export-pdf");
        assert_eq!(r.items(), &["item-2".to_string(), "item-1".to_string()]);
        assert_eq!(r.error(), None);
    }

    #[test]
    fn completing_finishes_the_request_once() {
        let mut r = request();
        assert_eq!(r.complete(), Ok(()));
        assert!(!r.is_pending());
        assert_eq!(r.outcome(), Some(&Outcome::Completed));
        assert_eq!(r.complete(), Err(RequestError::Finished(Outcome::Completed)));
        assert_eq!(r.cancel(None), Err(RequestError::Finished(Outcome::Completed)));
    }

    #[test]
    fn cancelling_without_a_message_records_a_user_cancel() {
        let mut r = request();
        assert_eq!(r.cancel(None), Ok(()));
        assert_eq!(r.outcome(), Some(&Outcome::UserCancelled));
    }

    #[test]
    fn cancelling_with_a_message_records_the_failure() {
        let mut r = request();
        assert_eq!(r.cancel(Some("disk full")), Ok(()));
        assert_eq!(r.outcome(), Some(&Outcome::Failed("disk full".to_string())));
    }

    #[test]
    fn an_error_keeps_the_request_pending_until_it_finishes() {
        let mut r = request();
        assert_eq!(r.show_error("sign in again"), Ok(()));
        assert!(r.is_pending());
        assert_eq!(r.error(), Some("sign in again"));
        assert_eq!(r.complete(), Ok(()));
        assert_eq!(r.error(), None, "a finished request shows no error");
        assert_eq!(r.show_error("late"), Err(RequestError::Finished(Outcome::Completed)));
    }
}
