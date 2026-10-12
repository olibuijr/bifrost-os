//! Weather's connections to the shell's system services: the widget timeline (the current-conditions entry that
//! Notification Centre shows) and the network path (offline state, and an immediate retry when the path is back).
//! The shell answers both; an older shell answers "no service ..." and the app then stops sending to it. The
//! functions here are the pure parts, so they can be checked without a shell.

use bifrost_app::controls::timeline::ReloadPolicy;
use bifrost_app::network_path::{Path, Status};
use bifrost_app::widgets_timeline::{self as wt, Entry};
use bifrost_app::Ctx;
use bifrost_core::json::Json;
use springboard::shell::weather::data::Summary;
use std::time::{SystemTime, UNIX_EPOCH};

/// The timeline kind the shell keeps for this app.
pub const TIMELINE_KIND: &str = "current-conditions";
/// How long the shell keeps an entry before it asks for a new timeline (seconds): the forecast refresh interval.
pub const RELOAD_SECS: f64 = 15.0 * 60.0;

/// Unix seconds now.
pub fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The timeline entries for `summary`: one entry from `now`, with the place as the title and the temperature and
/// sky as the text. Empty when no forecast is known.
pub fn entries(summary: Option<&Summary>, now: f64) -> Vec<Entry> {
    summary
        .map(|s| {
            vec![Entry {
                date: now,
                title: s.place.clone(),
                text: format!("{} {}", s.units.degrees(s.temp), s.sky().label()),
            }]
        })
        .unwrap_or_default()
}

/// Sends the current-conditions timeline for `summary`: a reload whose reload date is one refresh interval away, or
/// a removal when no forecast is known. Returns the request id the reply will carry.
pub fn push_timeline(ctx: &Ctx, summary: Option<&Summary>, now: f64) -> u32 {
    let entries = entries(summary, now);
    if entries.is_empty() {
        wt::remove(ctx, TIMELINE_KIND)
    } else {
        wt::reload(ctx, TIMELINE_KIND, &entries, ReloadPolicy::After(now + RELOAD_SECS))
    }
}

/// Whether a network path lets the app reach its forecast service. Only a satisfied path counts: a path that
/// requires a connection (an address is still pending) is treated as offline until it is satisfied.
pub fn online(path: &Path) -> bool {
    path.status == Status::Satisfied
}

/// Whether a service reply says the shell has no such service (an older shell). The app stops calling it then.
pub fn missing_service(reply: &Result<Json, String>) -> bool {
    matches!(reply, Err(e) if e.starts_with("no service"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::network_path::Interface;
    use bifrost_app::proto::{Configure, ToShell};
    use springboard::shell::weather::data::Units;

    fn ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn summary(units: Units) -> Summary {
        Summary { place: "Reykjavik".into(), temp: 4.4, code: 3, day: true, max: 6.0, min: 1.0, units }
    }

    fn path(status: Status) -> Path {
        Path {
            status,
            interfaces: vec![Interface { name: "eth0".into(), kind: bifrost_app::network_path::IfaceKind::Wired }],
            expensive: false,
            ipv4: true,
            ipv6: false,
        }
    }

    #[test]
    fn a_known_forecast_reloads_the_timeline_with_an_entry_and_a_refresh_date() {
        let ctx = ctx();
        let id = push_timeline(&ctx, Some(&summary(Units::Celsius)), 1000.0);
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("a service call: {sent:?}") };
        assert_eq!((*request, service.as_str(), method.as_str()), (id, "widgets_timeline", "reload"));
        let json = bifrost_core::json::parse(body).unwrap();
        assert_eq!(json.str_field("kind").as_deref(), Some(TIMELINE_KIND));
        assert_eq!(json.str_field("policy").as_deref(), Some("after"));
        assert_eq!(json.get("reload_at").and_then(Json::number), Some(1000.0 + RELOAD_SECS));
        let shown = json.get("entries").and_then(Json::as_array).unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].str_field("title").as_deref(), Some("Reykjavik"));
        assert_eq!(shown[0].get("date").and_then(Json::number), Some(1000.0));
    }

    #[test]
    fn the_entry_text_is_the_temperature_in_the_app_units_and_the_sky() {
        let s = summary(Units::Fahrenheit);
        let got = entries(Some(&s), 5.0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, format!("{} {}", Units::Fahrenheit.degrees(4.4), s.sky().label()));
    }

    #[test]
    fn with_no_forecast_the_timeline_is_removed_not_reloaded() {
        let ctx = ctx();
        assert!(entries(None, 5.0).is_empty());
        push_timeline(&ctx, None, 5.0);
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { method, body, .. } = &sent[0] else { panic!("a service call") };
        assert_eq!(method, "remove");
        assert_eq!(bifrost_core::json::parse(body).unwrap().str_field("kind").as_deref(), Some(TIMELINE_KIND));
    }

    #[test]
    fn only_a_satisfied_path_is_online() {
        assert!(online(&path(Status::Satisfied)));
        assert!(!online(&path(Status::Unsatisfied)));
        assert!(!online(&path(Status::RequiresConnection)));
    }

    #[test]
    fn an_older_shell_answering_no_service_is_detected() {
        assert!(missing_service(&Err("no service widgets_timeline".into())));
        assert!(!missing_service(&Err("reload needs an entry date".into())));
        assert!(!missing_service(&Ok(Json::Null)));
    }
}
