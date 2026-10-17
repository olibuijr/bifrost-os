//! The control methods Locate serves to scripts and the assistant (bifrost-app-control): `locate.locate` and
//! `locate.play_sound` name a device by id, and `locate.share` sets the Share My Location switch. This module only
//! parses the parameters into a [`Request`]; the window decides what the request does, and the cloud client sends
//! it. A device id is checked here so that nothing reaches a request path with symbols in it.

use bifrost_core::json::Json;

/// The methods the app serves, as the control socket lists them.
pub const METHODS: [&str; 3] = ["locate.locate", "locate.play_sound", "locate.share"];

/// One parsed control request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Locate(String),
    PlaySound(String),
    Share(bool),
}

/// Why a control request was refused, with the message the reply carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    UnknownMethod(String),
    MissingDevice,
    BadDevice(String),
    MissingFlag,
}

impl ParseError {
    /// The text sent back in the JSON-RPC error.
    pub fn message(&self) -> String {
        match self {
            ParseError::UnknownMethod(m) => format!("no method {m}"),
            ParseError::MissingDevice => "the device parameter is required".into(),
            ParseError::BadDevice(d) => format!("{d:?} is not a device id"),
            ParseError::MissingFlag => "the sharing parameter must be true or false".into(),
        }
    }
}

/// A device id is the cloud's id: letters and digits only, at most 32 bytes.
pub fn valid_device_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn device_param(params: &Json) -> Result<String, ParseError> {
    let id = params.get("device").and_then(Json::as_str).ok_or(ParseError::MissingDevice)?;
    if valid_device_id(id) { Ok(id.to_string()) } else { Err(ParseError::BadDevice(id.to_string())) }
}

/// Parses one control call. `params` is the JSON object the caller sent; `locate.share` takes `{"on": true|false}`.
pub fn parse(method: &str, params: &Json) -> Result<Request, ParseError> {
    match method {
        "locate.locate" => Ok(Request::Locate(device_param(params)?)),
        "locate.play_sound" => Ok(Request::PlaySound(device_param(params)?)),
        "locate.share" => match params.get("on") {
            Some(Json::Bool(on)) => Ok(Request::Share(*on)),
            _ => Err(ParseError::MissingFlag),
        },
        other => Err(ParseError::UnknownMethod(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json;

    fn p(text: &str) -> Json {
        json::parse(text).unwrap()
    }

    #[test]
    fn locate_and_play_sound_take_a_device_id() {
        assert_eq!(parse("locate.locate", &p(r#"{"device":"d1"}"#)), Ok(Request::Locate("d1".into())));
        assert_eq!(parse("locate.play_sound", &p(r#"{"device":"a9f3"}"#)), Ok(Request::PlaySound("a9f3".into())));
    }

    #[test]
    fn a_missing_or_malformed_device_is_refused() {
        assert_eq!(parse("locate.locate", &p("{}")), Err(ParseError::MissingDevice));
        assert_eq!(parse("locate.play_sound", &p(r#"{"device":"../x"}"#)), Err(ParseError::BadDevice("../x".into())));
        assert_eq!(parse("locate.locate", &p(r#"{"device":""}"#)), Err(ParseError::BadDevice(String::new())));
        let long = "a".repeat(33);
        assert!(!valid_device_id(&long));
    }

    #[test]
    fn share_takes_a_boolean_switch_value() {
        assert_eq!(parse("locate.share", &p(r#"{"on":true}"#)), Ok(Request::Share(true)));
        assert_eq!(parse("locate.share", &p(r#"{"on":false}"#)), Ok(Request::Share(false)));
        assert_eq!(parse("locate.share", &p(r#"{"on":"yes"}"#)), Err(ParseError::MissingFlag));
        assert_eq!(parse("locate.share", &p("{}")), Err(ParseError::MissingFlag));
    }

    #[test]
    fn an_unknown_method_names_itself_in_the_error() {
        let err = parse("locate.erase", &p("{}")).unwrap_err();
        assert_eq!(err.message(), "no method locate.erase");
    }

    #[test]
    fn the_method_list_matches_what_parse_accepts() {
        for m in METHODS {
            assert!(!matches!(parse(m, &p("{}")), Err(ParseError::UnknownMethod(_))), "{m} must be known");
        }
    }
}
