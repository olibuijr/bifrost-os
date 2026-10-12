//! News' connections to the shell's system services: the network path (reading offline) and the widget timeline
//! (the top stories). The service clients live in the SDK (`bifrost_app::network_path`,
//! `bifrost_app::widgets_timeline`); this file decides when News calls them and decodes what comes back.
//! Calls are made only on real changes: a new set of headlines, a reload the shell asks for, a path change.

use crate::feed::Item;
use bifrost_app::controls::timeline::ReloadPolicy;
use bifrost_app::network_path::{self, Path, Status};
use bifrost_app::widgets_timeline::{self as wt, Entry, Event};
use bifrost_app::Ctx;
use bifrost_core::json::Json;
use std::time::{SystemTime, UNIX_EPOCH};

/// The widget timeline News gives the shell.
pub const TOP_KIND: &str = "top-stories";
/// How many headlines the timeline holds, in Today's order.
pub const TOP_COUNT: usize = 4;
/// How long each headline stays on screen before the next one, in seconds.
pub const SLOT_SECS: f64 = 1800.0;

/// Unix seconds now, as the timeline dates use them.
pub fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The timeline entries for the top headlines: Today's order (the first item of each feed, then the second, and so
/// on), at most `TOP_COUNT`, each shown `SLOT_SECS` after the one before it, starting at `now`.
pub fn top_entries(feeds: &[Vec<Item>], now: f64) -> Vec<Entry> {
    let longest = feeds.iter().map(Vec::len).max().unwrap_or(0);
    (0..longest)
        .flat_map(|i| feeds.iter().filter_map(move |f| f.get(i)))
        .take(TOP_COUNT)
        .enumerate()
        .map(|(n, item)| Entry { date: now + n as f64 * SLOT_SECS, title: item.title.clone(), text: item.source.clone() })
        .collect()
}

/// Sends the top stories to the shell. The timeline runs to its last entry, and the shell then asks for a reload.
/// With no headlines the timeline is removed.
pub fn publish_top(ctx: &Ctx, feeds: &[Vec<Item>], now: f64) -> u32 {
    let entries = top_entries(feeds, now);
    if entries.is_empty() {
        return wt::remove(ctx, TOP_KIND);
    }
    wt::reload(ctx, TOP_KIND, &entries, ReloadPolicy::AtEnd)
}

/// Whether a service event is the shell asking for a new top-stories timeline.
pub fn is_top_reload(service: &str, event: &str, body: &Json) -> bool {
    matches!(wt::parse_event(service, event, body), Some(Event::Reload { kind }) if kind == TOP_KIND)
}

/// Asks the shell for the network path. The reply comes back as `App::service_reply` with the returned id.
pub fn request_path(ctx: &Ctx) -> u32 {
    network_path::request_path(ctx)
}

/// Whether a path lets traffic leave the machine. Anything but a satisfied path counts as offline.
pub fn is_online(path: &Path) -> bool {
    path.status == Status::Satisfied
}

/// The online state in a reply to `request_path`. None for an error (an older shell answers "no service") or a
/// malformed body; the caller then keeps what it had.
pub fn online_from_reply(result: &Result<Json, String>) -> Option<bool> {
    network_path::decode_reply(result.clone()).ok().map(|p| is_online(&p))
}

/// The online state in a service event. None for any event that is not a path update.
pub fn online_from_event(service: &str, event: &str, body: &Json) -> Option<bool> {
    network_path::path_event(service, event, body)?.ok().map(|p| is_online(&p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::Font;
    use bifrost_app::proto::{Configure, ToShell};

    fn testing_ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(bifrost_app::Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn item(title: &str, source: &str) -> Item {
        Item { title: title.into(), link: String::new(), date: String::new(), body: String::new(), source: source.into(), image: String::new() }
    }

    fn json(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn top_entries_follow_today_order_and_space_the_headlines() {
        let feeds = vec![vec![item("a1", "RUV"), item("a2", "RUV")], vec![item("b1", "mbl.is")], vec![item("c1", "Visir"), item("c2", "Visir")]];
        let entries = top_entries(&feeds, 1000.0);
        let titles: Vec<&str> = entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["a1", "b1", "c1", "a2"], "first of each feed, then the second round, capped");
        assert_eq!(entries[0].date, 1000.0);
        assert_eq!(entries[1].date, 1000.0 + SLOT_SECS);
        assert_eq!(entries[0].text, "RUV", "the feed name is the entry's text");
    }

    #[test]
    fn top_entries_are_empty_without_headlines() {
        assert!(top_entries(&[], 0.0).is_empty());
        assert!(top_entries(&[vec![]], 0.0).is_empty());
    }

    #[test]
    fn publish_top_sends_a_reload_with_the_entries_and_the_end_policy() {
        let ctx = testing_ctx();
        let feeds = vec![vec![item("Headline", "RUV")]];
        let id = publish_top(&ctx, &feeds, 500.0);
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("one service call") };
        assert_eq!(*request, id);
        assert_eq!(service, "widgets_timeline");
        assert_eq!(method, "reload");
        let body = json(body);
        assert_eq!(body.str_field("kind").as_deref(), Some(TOP_KIND));
        assert_eq!(body.str_field("policy").as_deref(), Some("at_end"));
        let first = &body.get("entries").and_then(Json::array).unwrap()[0];
        assert_eq!(first.str_field("title").as_deref(), Some("Headline"));
    }

    #[test]
    fn publish_top_removes_the_timeline_when_there_is_nothing_to_show() {
        let ctx = testing_ctx();
        publish_top(&ctx, &[], 0.0);
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { method, body, .. } = &sent[0] else { panic!("one service call") };
        assert_eq!(method, "remove");
        assert_eq!(json(body).str_field("kind").as_deref(), Some(TOP_KIND));
    }

    #[test]
    fn the_reload_event_is_recognised_for_our_timeline_only() {
        let ours = json(&format!(r#"{{"kind":"{TOP_KIND}"}}"#));
        assert!(is_top_reload("widgets_timeline", "reload", &ours));
        assert!(!is_top_reload("widgets_timeline", "reload", &json(r#"{"kind":"other"}"#)));
        assert!(!is_top_reload("dock", "reload", &ours));
    }

    #[test]
    fn a_satisfied_path_is_online_and_anything_else_is_not() {
        let reply = |status: &str| json(&format!(r#"{{"status":"{status}","interfaces":[],"expensive":false,"ipv4":false,"ipv6":false}}"#));
        assert_eq!(online_from_reply(&Ok(reply("satisfied"))), Some(true));
        assert_eq!(online_from_reply(&Ok(reply("unsatisfied"))), Some(false));
        assert_eq!(online_from_reply(&Ok(reply("requiresConnection"))), Some(false));
    }

    #[test]
    fn an_error_reply_or_an_unrelated_event_leaves_the_state_alone() {
        assert_eq!(online_from_reply(&Err("no service network".into())), None);
        assert_eq!(online_from_event("dock", "pathUpdate", &Json::Null), None);
        assert_eq!(online_from_event("network", "wifi", &Json::Null), None);
        let event = json(r#"{"status":"unsatisfied","interfaces":[],"expensive":false,"ipv4":false,"ipv6":false}"#);
        assert_eq!(online_from_event("network", "pathUpdate", &event), Some(false));
    }
}
