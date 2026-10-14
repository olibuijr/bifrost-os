//! The printers widget: one timeline entry the shell shows in Notification Centre. Its title and text come from
//! `model::widget_text` (the most important thing about the printers); the window sends it when it changes and again when
//! the shell asks for a new timeline. The service is the shell's `widgets_timeline` (see `bifrost_app::widgets_timeline`).

use bifrost_app::widgets_timeline::{self as wt, Entry, ReloadPolicy};
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// The timeline kind the shell keeps for this app.
pub const KIND: &str = "printers";

/// The timeline: one entry from `now` (Unix seconds).
pub fn entries(title: &str, text: &str, now: f64) -> Vec<Entry> {
    vec![Entry { date: now, title: title.to_string(), text: text.to_string() }]
}

/// Sends the timeline. The entry stays until the window sends another: it changes only when the printers do, so the
/// shell has no reload date to wait for. Returns the request id the reply will carry.
pub fn send(ctx: &Ctx, title: &str, text: &str, now: f64) -> u32 {
    wt::reload(ctx, KIND, &entries(title, text, now), ReloadPolicy::Never)
}

/// True when a service event is the shell asking for a new timeline of this widget.
pub fn reload_requested(service: &str, event: &str, body: &Json) -> bool {
    matches!(wt::parse_event(service, event, body), Some(wt::Event::Reload { kind }) if kind == KIND)
}

/// Unix seconds now.
pub fn now_unix() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(kind: &str) -> Json {
        Json::Obj(vec![("kind".to_string(), Json::Str(kind.to_string()))])
    }

    #[test]
    fn the_timeline_is_one_entry_from_now() {
        let list = entries("Printers", "Office is ready", 1000.0);
        assert_eq!(list, vec![Entry { date: 1000.0, title: "Printers".into(), text: "Office is ready".into() }]);
    }

    #[test]
    fn only_a_reload_of_this_widget_is_ours() {
        assert!(reload_requested(wt::SERVICE, wt::EVENT_RELOAD, &body(KIND)));
        assert!(!reload_requested(wt::SERVICE, wt::EVENT_RELOAD, &body("current-conditions")), "another app's widget");
        assert!(!reload_requested("dock", wt::EVENT_RELOAD, &body(KIND)), "another service");
        assert!(!reload_requested(wt::SERVICE, "other", &body(KIND)));
    }
}
