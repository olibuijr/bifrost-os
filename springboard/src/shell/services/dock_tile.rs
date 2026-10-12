//! The `dock` service: an app's own Dock tile. `badge` {"text"} sets the badge (the `notifications` permission is needed,
//! as for `ToShell::SetBadge`; empty clears it), `progress` {"fraction": 0..1} shows a bar and {"indeterminate": true} a
//! sweeping one (an empty body clears it), `bounce` {"critical": bool} bounces the icon (critical bounces for longer), and
//! `state` reads the three back. The shell draws them: the badge in `desk.rs`, the bar and the bounce in `desktop_dock.rs`.
use super::{Event, Shell, SystemService};
use crate::shell::apphost::{clean_badge, installed};
use crate::shell::AppId;
use bifrost_core::json::Json;
use bifrost_render::motion::{self, Kind};
use bifrost_ui::values::Progress;

/// An informational bounce: this many hops of the Dock bounce.
const INFO_HOPS: f64 = 3.0;
/// A critical bounce keeps going this long, or until the app comes to the front (see `tick`).
const CRITICAL_S: f64 = 10.0;

/// The `dock` service. The per-app state lives on the app's host (`AppHost::badge` and `AppHost::tile`).
pub struct DockService;

/// The badge text in `params` (`text`); a missing or null text is empty.
fn badge_text(params: &Json) -> Result<String, String> {
    match params.get("text") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Str(s)) => Ok(s.clone()),
        Some(_) => Err("dock: badge text must be a string".into()),
    }
}

/// The progress in `params`: `fraction` (clamped to 0..1 by the model), `indeterminate: true`, or neither to clear.
fn progress_of(params: &Json) -> Result<Option<Progress>, String> {
    match params.get("fraction") {
        Some(Json::Num(f)) if f.is_finite() => Ok(Some(Progress::determinate(*f as f32))),
        Some(Json::Num(_)) => Err("dock: fraction must be a finite number".into()),
        None | Some(Json::Null) if matches!(params.get("indeterminate"), Some(Json::Bool(true))) => Ok(Some(Progress::indeterminate())),
        None | Some(Json::Null) => Ok(None),
        Some(_) => Err("dock: fraction must be a number".into()),
    }
}

/// The length in seconds of a bounce: `critical` or three hops of `hop` seconds.
fn bounce_len(params: &Json, hop: f64) -> f64 {
    if matches!(params.get("critical"), Some(Json::Bool(true))) {
        CRITICAL_S
    } else {
        INFO_HOPS * hop
    }
}

/// The slot of the installed app `app_id`. Only an installed app has a Dock tile.
fn slot_of(app_id: &str) -> Result<usize, String> {
    installed().iter().position(|a| a.manifest.id == app_id).ok_or_else(|| "dock: only an installed app has a Dock tile".to_string())
}

impl SystemService for DockService {
    fn name(&self) -> &'static str {
        "dock"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        let slot = slot_of(app_id)?;
        let now = t / 1000.0;
        let hop = motion::duration(Kind::DockBounce) as f64;
        let permitted = installed().get(slot).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == "notifications"));
        // an app that is already in front has nothing to attract
        let fronted = matches!(shell.desk.as_ref().and_then(|d| d.front()).map(|w| w.app), Some(AppId::External(s)) if s as usize == slot);
        let Some(host) = shell.ext.get_mut(slot).and_then(Option::as_mut) else {
            return Err("dock: the app is not running".into());
        };
        let reply = match method {
            "badge" => {
                if !permitted {
                    return Err("dock: the badge needs the notifications permission".into());
                }
                host.badge = clean_badge(&badge_text(params)?);
                Json::Null
            }
            "progress" => {
                host.tile.progress = progress_of(params)?;
                Json::Null
            }
            "bounce" => {
                if !fronted {
                    host.tile.bounce = Some((now, now + bounce_len(params, hop)));
                }
                Json::Null
            }
            "state" => Json::Obj(vec![
                ("badge".into(), Json::Str(host.badge.clone())),
                (
                    "progress".into(),
                    match host.tile.progress.and_then(|p| p.fraction()) {
                        Some(f) => Json::Num(f as f64),
                        None => Json::Null,
                    },
                ),
                ("indeterminate".into(), Json::Bool(host.tile.progress.is_some_and(|p| p.fraction().is_none()))),
                ("bouncing".into(), Json::Bool(host.tile.bounce.is_some_and(|(_, end)| now < end))),
            ]),
            _ => return Err(format!("dock has no method {method}")),
        };
        shell.dirty = true;
        Ok(reply)
    }

    /// Ends a bounce whose window has passed, or whose app has come to the front (activating the app cancels the
    /// request), so the icon settles on its own and the dock is drawn once more.
    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let now = t / 1000.0;
        let front = shell.desk.as_ref().and_then(|d| d.front()).map(|w| w.app);
        let mut ended = false;
        for (slot, host) in shell.ext.iter_mut().enumerate() {
            let Some(host) = host else { continue };
            let fronted = matches!(front, Some(AppId::External(s)) if s as usize == slot);
            if host.tile.bounce.is_some_and(|(_, end)| now >= end || fronted) {
                host.tile.bounce = None;
                ended = true;
            }
        }
        if ended {
            shell.dirty = true;
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;
    use bifrost_ui::dock_tile::DockTile;

    fn json(text: &str) -> Json {
        parse(text).unwrap()
    }

    #[test]
    fn the_service_is_registered_under_dock() {
        assert!(super::super::names(&super::super::all()).contains(&"dock"));
    }

    #[test]
    fn the_badge_text_is_read_and_cleaned_like_the_dock_model() {
        assert_eq!(badge_text(&json(r#"{"text":"12"}"#)), Ok("12".into()));
        assert_eq!(badge_text(&json("{}")), Ok(String::new()));
        assert!(badge_text(&json(r#"{"text":3}"#)).is_err());
        // the shell's cleaning and the dock tile model agree on what is visible
        for raw in ["123456", "99+", "  7 \n", "éééééé"] {
            let mut model = DockTile::new();
            model.set_badge_label(&clean_badge(raw));
            assert_eq!(model.displayed_badge().unwrap_or_default(), clean_badge(raw), "{raw:?}");
        }
    }

    #[test]
    fn progress_is_a_fraction_an_indeterminate_bar_or_cleared() {
        let p = progress_of(&json(r#"{"fraction":0.4}"#)).unwrap().unwrap();
        assert!((p.fraction().unwrap() - 0.4).abs() < 1e-6);
        assert_eq!(progress_of(&json(r#"{"fraction":2}"#)).unwrap().unwrap().fraction(), Some(1.0), "clamped");
        assert_eq!(progress_of(&json(r#"{"indeterminate":true}"#)).unwrap().unwrap().fraction(), None);
        assert_eq!(progress_of(&json("{}")).unwrap(), None);
        assert_eq!(progress_of(&Json::Null).unwrap(), None);
        assert!(progress_of(&json(r#"{"fraction":"half"}"#)).is_err());
    }

    #[test]
    fn a_critical_bounce_lasts_longer_than_an_informational_one() {
        assert_eq!(bounce_len(&json(r#"{"critical":true}"#), 0.5), CRITICAL_S);
        assert!((bounce_len(&json("{}"), 0.5) - 1.5).abs() < 1e-9);
        assert_eq!(bounce_len(&json("{}"), 0.0), 0.0, "no bounce under Reduce Motion");
    }

    #[test]
    fn an_app_that_is_not_installed_has_no_tile() {
        assert!(slot_of("no.such.app.for.the.dock").is_err());
    }
}
