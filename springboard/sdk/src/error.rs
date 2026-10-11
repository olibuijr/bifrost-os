//! Foundation error parity: an error with a domain, a numeric code, a user info dictionary and the strings an alert
//! shows (`localizedDescription`, `localizedFailureReason`, `localizedRecoverySuggestion`, `localizedRecoveryOptions`,
//! `helpAnchor`, `underlyingErrors`). A caller tests `is(domain, code)` instead of parsing the text.
//!
//! Not here yet: per-domain user info value providers and a recovery attempter that runs a chosen option. The
//! caller reads `recovery_options` and acts on the choice itself. Each module keeps its own error enum; `AppError`
//! is the form that crosses to a caller that only needs the domain, code and text.

use crate::url::URLError;
use std::collections::BTreeMap;
use std::fmt;

/// The domain of errors that came from the URL loading stack.
pub const URL_DOMAIN: &str = "bifrost.url";

/// A domain-and-code error with localized text. Build it with `new` and the `with_*` methods.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AppError {
    domain: String,
    code: i64,
    user_info: BTreeMap<String, String>,
    description: Option<String>,
    failure_reason: Option<String>,
    recovery_suggestion: Option<String>,
    recovery_options: Vec<String>,
    help_anchor: Option<String>,
    underlying: Vec<AppError>,
}

impl AppError {
    /// `init(domain:code:userInfo:)` with an empty user info dictionary.
    pub fn new(domain: impl Into<String>, code: i64) -> Self {
        AppError { domain: domain.into(), code, ..Self::default() }
    }

    /// `domain`.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// `code`.
    pub fn code(&self) -> i64 {
        self.code
    }

    /// True when the error has this domain and code.
    pub fn is(&self, domain: &str, code: i64) -> bool {
        self.domain == domain && self.code == code
    }

    /// `userInfo`: extra text for a caller, such as a file name.
    pub fn user_info(&self) -> &BTreeMap<String, String> {
        &self.user_info
    }

    pub fn with_user_info(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.user_info.insert(key.into(), value.into());
        self
    }

    pub fn with_description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    pub fn with_failure_reason(mut self, text: impl Into<String>) -> Self {
        self.failure_reason = Some(text.into());
        self
    }

    pub fn with_recovery_suggestion(mut self, text: impl Into<String>) -> Self {
        self.recovery_suggestion = Some(text.into());
        self
    }

    /// The titles of the buttons an alert shows, in order.
    pub fn with_recovery_options(mut self, titles: Vec<String>) -> Self {
        self.recovery_options = titles;
        self
    }

    pub fn with_help_anchor(mut self, anchor: impl Into<String>) -> Self {
        self.help_anchor = Some(anchor.into());
        self
    }

    pub fn with_underlying(mut self, error: AppError) -> Self {
        self.underlying.push(error);
        self
    }

    /// `localizedDescription`. Without one set, a generic sentence names the domain and code.
    pub fn localized_description(&self) -> String {
        match &self.description {
            Some(text) => text.clone(),
            None => format!("The operation could not be completed. ({} error {}.)", self.domain, self.code),
        }
    }

    /// `localizedFailureReason`.
    pub fn localized_failure_reason(&self) -> Option<&str> {
        self.failure_reason.as_deref()
    }

    /// `localizedRecoverySuggestion`.
    pub fn localized_recovery_suggestion(&self) -> Option<&str> {
        self.recovery_suggestion.as_deref()
    }

    /// `localizedRecoveryOptions`.
    pub fn recovery_options(&self) -> &[String] {
        &self.recovery_options
    }

    /// `helpAnchor`: the topic an alert's help button opens.
    pub fn help_anchor(&self) -> Option<&str> {
        self.help_anchor.as_deref()
    }

    /// `underlyingErrors`, in the order they were added.
    pub fn underlying_errors(&self) -> &[AppError] {
        &self.underlying
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized_description())
    }
}

impl std::error::Error for AppError {
    /// The first underlying error, which the standard error chain walks.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.underlying.first().map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// Maps each URL loading failure to a stable code in [`URL_DOMAIN`]: the codes follow declaration order, 1 to 7.
impl From<URLError> for AppError {
    fn from(error: URLError) -> Self {
        let code = match &error {
            URLError::BadURL(_) => 1,
            URLError::CannotConnect(_) => 2,
            URLError::TimedOut => 3,
            URLError::Cancelled => 4,
            URLError::BadServerResponse(_) => 5,
            URLError::TooManyRedirects => 6,
            URLError::FileError(_) => 7,
        };
        AppError::new(URL_DOMAIN, code).with_description(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_description_the_text_names_the_domain_and_code() {
        let e = AppError::new("bifrost.test", 3);
        assert_eq!(e.localized_description(), "The operation could not be completed. (bifrost.test error 3.)");
        assert_eq!(e.to_string(), e.localized_description());
        assert!(e.is("bifrost.test", 3) && !e.is("bifrost.test", 4) && !e.is("other", 3));
    }

    #[test]
    fn an_explicit_description_is_what_display_shows() {
        let e = AppError::new("bifrost.test", 1).with_description("The file is locked.");
        assert_eq!(e.localized_description(), "The file is locked.");
        assert_eq!(format!("{e}"), "The file is locked.");
    }

    #[test]
    fn url_failures_keep_their_domain_and_a_distinct_code_per_case() {
        let cases = [
            URLError::BadURL("x".into()),
            URLError::CannotConnect("x".into()),
            URLError::TimedOut,
            URLError::Cancelled,
            URLError::BadServerResponse("x".into()),
            URLError::TooManyRedirects,
            URLError::FileError("x".into()),
        ];
        let mut codes: Vec<i64> = cases.into_iter().map(|c| AppError::from(c).code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes, vec![1, 2, 3, 4, 5, 6, 7]);

        let timed_out: AppError = URLError::TimedOut.into();
        assert!(timed_out.is(URL_DOMAIN, 3));
        assert_eq!(timed_out.localized_description(), "The request timed out");
    }

    #[test]
    fn recovery_options_keep_their_order_and_help_text_is_optional() {
        let e = AppError::new("bifrost.test", 9)
            .with_failure_reason("The disk is full.")
            .with_recovery_suggestion("Free some space and try again.")
            .with_recovery_options(vec!["Try Again".into(), "Cancel".into()])
            .with_help_anchor("disk-space");
        assert_eq!(e.recovery_options(), ["Try Again".to_string(), "Cancel".to_string()]);
        assert_eq!(e.localized_failure_reason(), Some("The disk is full."));
        assert_eq!(e.localized_recovery_suggestion(), Some("Free some space and try again."));
        assert_eq!(e.help_anchor(), Some("disk-space"));
        assert_eq!(AppError::new("bifrost.test", 9).help_anchor(), None);
    }

    #[test]
    fn user_info_is_sorted_and_the_first_underlying_error_is_the_source() {
        let inner = AppError::new("bifrost.test", 1).with_description("inner");
        let second = AppError::new("bifrost.test", 2);
        let outer = AppError::new("bifrost.test", 5)
            .with_user_info("name", "b.txt")
            .with_user_info("folder", "/notes")
            .with_underlying(inner.clone())
            .with_underlying(second);
        let keys: Vec<&str> = outer.user_info().keys().map(String::as_str).collect();
        assert_eq!(keys, ["folder", "name"]);
        assert_eq!(outer.underlying_errors().len(), 2);
        let source = std::error::Error::source(&outer).expect("a source");
        assert_eq!(source.to_string(), "inner");
        assert!(std::error::Error::source(&inner).is_none());
    }
}
