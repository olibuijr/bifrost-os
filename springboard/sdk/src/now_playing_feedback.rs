//! Feedback and rating for the Lock Screen and Control Center: the like, dislike and bookmark toggles and the rating
//! scale of the playing item. The app keeps the state in `bifrost_ui::media_feedback` and publishes it with
//! [`feedback_body`]; the shell sends the user's choice back as a `command` event on the `now_playing` service, which
//! [`feedback_from_event`] and [`rate_from_event`] decode. The app applies the change to its own state before it
//! republishes, so a command the app refuses is not shown as done.

use bifrost_core::json::Json;
use bifrost_ui::media_feedback::{FeedbackKind, FeedbackState, RatingCommand};

/// The wire name of a feedback command.
pub fn kind_name(kind: FeedbackKind) -> &'static str {
    match kind {
        FeedbackKind::Like => "like",
        FeedbackKind::Dislike => "dislike",
        FeedbackKind::Bookmark => "bookmark",
    }
}

/// The feedback kind named on the wire.
pub fn kind_from_name(name: &str) -> Option<FeedbackKind> {
    FeedbackKind::ALL.iter().copied().find(|k| kind_name(*k) == name)
}

/// The `feedback` publish body: each toggle as on or off, and the rating scale with its value when the item has a
/// rating command. Sent with the shell's `now_playing` service `feedback` call.
pub fn feedback_body(state: &FeedbackState, rating: Option<&RatingCommand>) -> Json {
    let toggles: Vec<(String, Json)> = FeedbackKind::ALL
        .iter()
        .map(|k| (kind_name(*k).to_string(), Json::Bool(state.is_active(*k))))
        .collect();
    let mut fields = vec![("toggles".to_string(), Json::Obj(toggles))];
    if let Some(r) = rating {
        fields.push((
            "rating".to_string(),
            Json::Obj(vec![
                ("min".to_string(), Json::Num(r.minimum() as f64)),
                ("max".to_string(), Json::Num(r.maximum() as f64)),
                ("value".to_string(), Json::Num(r.rating() as f64)),
            ]),
        ));
    }
    Json::Obj(fields)
}

/// The feedback toggle a shell `command` event asks for, with the state it asks for (`active`). `None` for another
/// service, another event, or an unknown name.
pub fn feedback_from_event(service: &str, event: &str, body: &Json) -> Option<(FeedbackKind, bool)> {
    if service != super::now_playing::SERVICE || event != super::now_playing::COMMAND_EVENT {
        return None;
    }
    let kind = kind_from_name(&body.str_field("command")?)?;
    let active = match body.get("active") {
        Some(Json::Bool(b)) => *b,
        _ => return None,
    };
    Some((kind, active))
}

/// The rating a shell `command` event asks for (`rate`, with a `rating` number). `None` for anything else.
pub fn rate_from_event(service: &str, event: &str, body: &Json) -> Option<f32> {
    if service != super::now_playing::SERVICE || event != super::now_playing::COMMAND_EVENT {
        return None;
    }
    if body.str_field("command")?.as_str() != "rate" {
        return None;
    }
    body.get("rating").and_then(Json::number).map(|v| v as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::now_playing::{COMMAND_EVENT, SERVICE};

    fn parse(text: &str) -> Json {
        bifrost_core::json::parse(text).expect("valid json")
    }

    #[test]
    fn names_round_trip_for_every_feedback_kind() {
        for kind in FeedbackKind::ALL {
            assert_eq!(kind_from_name(kind_name(kind)), Some(kind));
        }
        assert_eq!(kind_from_name("fly"), None);
    }

    #[test]
    fn the_feedback_body_reports_each_toggle_and_the_rating_scale() {
        let mut state = FeedbackState::new();
        state.set(FeedbackKind::Like, true);
        let rating = RatingCommand::new(0.0, 5.0).expect("a scale");
        let body = feedback_body(&state, Some(&rating));
        let toggles = body.get("toggles").expect("toggles");
        assert_eq!(toggles.get("like"), Some(&Json::Bool(true)));
        assert_eq!(toggles.get("dislike"), Some(&Json::Bool(false)));
        let r = body.get("rating").expect("rating");
        assert_eq!(r.get("max").and_then(Json::number), Some(5.0));
        assert!(feedback_body(&state, None).get("rating").is_none(), "no rating command, no rating field");
    }

    #[test]
    fn a_shell_toggle_event_decodes_to_kind_and_state() {
        let body = parse(r#"{"command":"dislike","active":true}"#);
        assert_eq!(feedback_from_event(SERVICE, COMMAND_EVENT, &body), Some((FeedbackKind::Dislike, true)));
        let no_state = parse(r#"{"command":"dislike"}"#);
        assert_eq!(feedback_from_event(SERVICE, COMMAND_EVENT, &no_state), None);
        assert_eq!(feedback_from_event("dock", COMMAND_EVENT, &body), None, "another service");
    }

    #[test]
    fn a_shell_rate_event_decodes_to_the_rating() {
        let body = parse(r#"{"command":"rate","rating":4}"#);
        assert_eq!(rate_from_event(SERVICE, COMMAND_EVENT, &body), Some(4.0));
        let toggle = parse(r#"{"command":"like","active":true}"#);
        assert_eq!(rate_from_event(SERVICE, COMMAND_EVENT, &toggle), None);
    }
}
