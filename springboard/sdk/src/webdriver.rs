//! WebDriver: the wire model of the W3C WebDriver protocol, the protocol a client uses to drive a page. It holds the
//! parts a client and an automation endpoint share: the error codes with their HTTP statuses, element locators with
//! their JSON bodies, the timeouts a session sets, and the key that names an element in a response. It opens no
//! connection and starts no server. The browser shell has no automation endpoint yet, and its page scripts report
//! `navigator.webdriver` as the fixed value `false`.

use std::fmt;

/// The error codes of the protocol. Each has a wire name and the HTTP status the specification gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    ElementClickIntercepted,
    ElementNotInteractable,
    InsecureCertificate,
    InvalidArgument,
    InvalidCookieDomain,
    InvalidElementState,
    InvalidSelector,
    InvalidSessionId,
    JavascriptError,
    MoveTargetOutOfBounds,
    NoSuchAlert,
    NoSuchCookie,
    NoSuchElement,
    NoSuchFrame,
    NoSuchWindow,
    ScriptTimeout,
    SessionNotCreated,
    StaleElementReference,
    Timeout,
    UnableToCaptureScreen,
    UnableToSetCookie,
    UnexpectedAlertOpen,
    UnknownCommand,
    UnknownError,
    UnknownMethod,
    UnsupportedOperation,
}

impl ErrorCode {
    /// Every code, in the order of the specification's table.
    pub const ALL: [ErrorCode; 26] = [
        ErrorCode::ElementClickIntercepted,
        ErrorCode::ElementNotInteractable,
        ErrorCode::InsecureCertificate,
        ErrorCode::InvalidArgument,
        ErrorCode::InvalidCookieDomain,
        ErrorCode::InvalidElementState,
        ErrorCode::InvalidSelector,
        ErrorCode::InvalidSessionId,
        ErrorCode::JavascriptError,
        ErrorCode::MoveTargetOutOfBounds,
        ErrorCode::NoSuchAlert,
        ErrorCode::NoSuchCookie,
        ErrorCode::NoSuchElement,
        ErrorCode::NoSuchFrame,
        ErrorCode::NoSuchWindow,
        ErrorCode::ScriptTimeout,
        ErrorCode::SessionNotCreated,
        ErrorCode::StaleElementReference,
        ErrorCode::Timeout,
        ErrorCode::UnableToCaptureScreen,
        ErrorCode::UnableToSetCookie,
        ErrorCode::UnexpectedAlertOpen,
        ErrorCode::UnknownCommand,
        ErrorCode::UnknownError,
        ErrorCode::UnknownMethod,
        ErrorCode::UnsupportedOperation,
    ];

    /// The `error` string in a response body.
    pub fn wire_name(self) -> &'static str {
        match self {
            ErrorCode::ElementClickIntercepted => "element click intercepted",
            ErrorCode::ElementNotInteractable => "element not interactable",
            ErrorCode::InsecureCertificate => "insecure certificate",
            ErrorCode::InvalidArgument => "invalid argument",
            ErrorCode::InvalidCookieDomain => "invalid cookie domain",
            ErrorCode::InvalidElementState => "invalid element state",
            ErrorCode::InvalidSelector => "invalid selector",
            ErrorCode::InvalidSessionId => "invalid session id",
            ErrorCode::JavascriptError => "javascript error",
            ErrorCode::MoveTargetOutOfBounds => "move target out of bounds",
            ErrorCode::NoSuchAlert => "no such alert",
            ErrorCode::NoSuchCookie => "no such cookie",
            ErrorCode::NoSuchElement => "no such element",
            ErrorCode::NoSuchFrame => "no such frame",
            ErrorCode::NoSuchWindow => "no such window",
            ErrorCode::ScriptTimeout => "script timeout",
            ErrorCode::SessionNotCreated => "session not created",
            ErrorCode::StaleElementReference => "stale element reference",
            ErrorCode::Timeout => "timeout",
            ErrorCode::UnableToCaptureScreen => "unable to capture screen",
            ErrorCode::UnableToSetCookie => "unable to set cookie",
            ErrorCode::UnexpectedAlertOpen => "unexpected alert open",
            ErrorCode::UnknownCommand => "unknown command",
            ErrorCode::UnknownError => "unknown error",
            ErrorCode::UnknownMethod => "unknown method",
            ErrorCode::UnsupportedOperation => "unsupported operation",
        }
    }

    /// The HTTP status of a response that carries this code.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::ElementClickIntercepted
            | ErrorCode::ElementNotInteractable
            | ErrorCode::InsecureCertificate
            | ErrorCode::InvalidArgument
            | ErrorCode::InvalidCookieDomain
            | ErrorCode::InvalidElementState
            | ErrorCode::InvalidSelector
            | ErrorCode::NoSuchAlert
            | ErrorCode::NoSuchFrame
            | ErrorCode::NoSuchWindow => 400,
            ErrorCode::UnknownMethod => 405,
            ErrorCode::ScriptTimeout => 408,
            ErrorCode::InvalidSessionId
            | ErrorCode::NoSuchCookie
            | ErrorCode::NoSuchElement
            | ErrorCode::StaleElementReference
            | ErrorCode::UnknownCommand => 404,
            ErrorCode::JavascriptError
            | ErrorCode::MoveTargetOutOfBounds
            | ErrorCode::SessionNotCreated
            | ErrorCode::Timeout
            | ErrorCode::UnableToCaptureScreen
            | ErrorCode::UnableToSetCookie
            | ErrorCode::UnexpectedAlertOpen
            | ErrorCode::UnknownError
            | ErrorCode::UnsupportedOperation => 500,
        }
    }

    /// The code for a wire name. The specification asks a client to treat a name it does not know as `unknown error`.
    pub fn from_wire(name: &str) -> ErrorCode {
        ErrorCode::ALL.into_iter().find(|code| code.wire_name() == name).unwrap_or(ErrorCode::UnknownError)
    }
}

/// A protocol error: its code and the message the response carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebDriverError {
    pub code: ErrorCode,
    pub message: String,
}

impl WebDriverError {
    pub fn new(code: ErrorCode, message: &str) -> WebDriverError {
        WebDriverError { code, message: message.to_string() }
    }

    /// The error of a response, from its `error` and `message` fields.
    pub fn from_wire(error: &str, message: &str) -> WebDriverError {
        WebDriverError::new(ErrorCode::from_wire(error), message)
    }
}

impl fmt::Display for WebDriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code.wire_name())
        } else {
            write!(f, "{}: {}", self.code.wire_name(), self.message)
        }
    }
}

impl std::error::Error for WebDriverError {}

/// The ways a locator can find elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    CssSelector,
    LinkText,
    PartialLinkText,
    TagName,
    XPath,
}

impl Strategy {
    /// The `using` string of a locator body.
    pub fn wire_name(self) -> &'static str {
        match self {
            Strategy::CssSelector => "css selector",
            Strategy::LinkText => "link text",
            Strategy::PartialLinkText => "partial link text",
            Strategy::TagName => "tag name",
            Strategy::XPath => "xpath",
        }
    }
}

/// A locator that has passed the checks a page would otherwise report later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locator {
    strategy: Strategy,
    value: String,
}

impl Locator {
    /// A locator for `value` under `strategy`. A blank value or one with a control character is an `invalid argument`.
    /// A tag name with whitespace in it is an `invalid selector`.
    pub fn new(strategy: Strategy, value: &str) -> Result<Locator, WebDriverError> {
        if value.trim().is_empty() {
            return Err(WebDriverError::new(ErrorCode::InvalidArgument, "the locator value is empty"));
        }
        if value.chars().any(char::is_control) {
            return Err(WebDriverError::new(ErrorCode::InvalidArgument, "the locator value holds a control character"));
        }
        if strategy == Strategy::TagName && value.chars().any(char::is_whitespace) {
            return Err(WebDriverError::new(ErrorCode::InvalidSelector, "a tag name holds no whitespace"));
        }
        Ok(Locator { strategy, value: value.to_string() })
    }

    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// The body of `POST /session/{id}/element`: `{"using":..., "value":...}`.
    pub fn to_json(&self) -> String {
        format!("{{\"using\":{},\"value\":{}}}", json_string(self.strategy.wire_name()), json_string(&self.value))
    }
}

/// The largest integer a timeout may hold: the largest integer a JavaScript number represents exactly.
pub const MAX_TIMEOUT_MS: u64 = 9_007_199_254_740_991;

/// The three timeouts of a session, in milliseconds. `script_ms` of `None` means no script timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub script_ms: Option<u64>,
    pub page_load_ms: u64,
    pub implicit_ms: u64,
}

impl Default for Timeouts {
    /// The specification's defaults: a 30 second script timeout, a 300 second page load, and no implicit wait.
    fn default() -> Timeouts {
        Timeouts { script_ms: Some(30_000), page_load_ms: 300_000, implicit_ms: 0 }
    }
}

impl Timeouts {
    /// Timeouts from the three values. Each is checked against [`MAX_TIMEOUT_MS`]; a larger one is an `invalid argument`.
    pub fn new(script_ms: Option<u64>, page_load_ms: u64, implicit_ms: u64) -> Result<Timeouts, WebDriverError> {
        for ms in script_ms.into_iter().chain([page_load_ms, implicit_ms]) {
            if ms > MAX_TIMEOUT_MS {
                return Err(WebDriverError::new(ErrorCode::InvalidArgument, &format!("timeout {ms} ms is above {MAX_TIMEOUT_MS}")));
            }
        }
        Ok(Timeouts { script_ms, page_load_ms, implicit_ms })
    }

    /// The body of `POST /session/{id}/timeouts`.
    pub fn to_json(&self) -> String {
        let script = self.script_ms.map_or_else(|| "null".to_string(), |ms| ms.to_string());
        format!("{{\"script\":{script},\"pageLoad\":{},\"implicit\":{}}}", self.page_load_ms, self.implicit_ms)
    }
}

/// The key that names an element in a response. The specification fixes this value.
pub const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// The JSON object that refers to an element found earlier in the session. A blank id is an `invalid argument`.
pub fn element_reference(id: &str) -> Result<String, WebDriverError> {
    if id.trim().is_empty() {
        return Err(WebDriverError::new(ErrorCode::InvalidArgument, "the element id is empty"));
    }
    Ok(format!("{{{}:{}}}", json_string(ELEMENT_KEY), json_string(id)))
}

/// `text` as a JSON string literal. Quotes, backslashes and control characters are escaped.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
