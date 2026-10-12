//! The `quick-look` service: the app side of the floating preview panel (`shell::preview_panel`). The SDK client is
//! `bifrost_app::quick_look`.
//!
//! Methods (JSON params, see the SDK for the typed calls):
//! - `show` `{items: [{url, title}], index}`: shows the panel for the calling app. Answers the state.
//! - `close` `{}`: hides the panel if the calling app opened it. Answers the state.
//! - `state` `{}`: answers `{visible, index, count}`.
//! - `step` `{delta}`: moves the index by `delta` (no wrapping). Answers the state.
//!
//! Events to the owning app: `closed` (the user closed the panel) and `index` `{index}` (the user stepped).
use super::{Event, SystemService};
use crate::shell::Shell;
use bifrost_core::json::Json;
use bifrost_proto::peek::{PreviewItem, MAX_ITEMS, MAX_TITLE, MAX_URL};

pub struct PreviewService;

/// `s` cut to `max` bytes on a character boundary.
fn cut(s: &str, max: usize) -> String {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    s[..n].to_string()
}

/// The `items` and `index` of a `show` call. Items past [`MAX_ITEMS`] are dropped; a missing url is an error.
pub fn parse_show(params: &Json) -> Result<(Vec<PreviewItem>, usize), String> {
    let Some(Json::Arr(list)) = params.get("items") else {
        return Err("show needs an items array".into());
    };
    let items = list
        .iter()
        .take(MAX_ITEMS)
        .map(|j| -> Result<PreviewItem, String> {
            let url = j.get("url").and_then(Json::as_str).ok_or("each item needs a url")?;
            let title = j.get("title").and_then(Json::as_str).unwrap_or("");
            Ok(PreviewItem { url: cut(url, MAX_URL), title: cut(title, MAX_TITLE) })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let index = params.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
    Ok((items, index))
}

impl SystemService for PreviewService {
    fn name(&self) -> &'static str {
        "quick-look"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "show" => {
                let (items, index) = parse_show(params)?;
                shell.quick_look_show(app_id, items, index);
                Ok(shell.quick_look_state())
            }
            "close" => {
                // only the app that opened the panel may close it
                if shell.quick_look_owner() == Some(app_id) {
                    shell.quick_look_hide();
                }
                Ok(shell.quick_look_state())
            }
            "state" => Ok(shell.quick_look_state()),
            "step" => {
                let delta = match params.get("delta") {
                    Some(Json::Num(n)) => n.round().clamp(-(MAX_ITEMS as f64), MAX_ITEMS as f64) as i32,
                    _ => return Err("step needs a numeric delta".into()),
                };
                if shell.quick_look_owner() == Some(app_id) {
                    shell.quick_look_step(delta);
                }
                Ok(shell.quick_look_state())
            }
            _ => Err(format!("quick-look has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let owner = shell.quick_look_owner().map(str::to_string);
        shell
            .quick_look_drain_events()
            .into_iter()
            .map(|(event, body)| Event { app: owner.clone(), event: event.to_string(), body })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: &[(&str, Json)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn show_params_become_items_and_an_index() {
        let params = obj(&[
            ("items", Json::Arr(vec![obj(&[("url", Json::Str("file:///a.txt".into())), ("title", Json::Str("A".into()))]), obj(&[("url", Json::Str("file:///b.png".into()))])])),
            ("index", Json::Num(1.0)),
        ]);
        let (items, index) = parse_show(&params).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[1].title, "");
        assert_eq!(index, 1);
    }

    #[test]
    fn show_refuses_a_missing_list_or_url_and_caps_the_length() {
        assert!(parse_show(&obj(&[])).is_err());
        assert!(parse_show(&obj(&[("items", Json::Arr(vec![obj(&[("title", Json::Str("x".into()))])]))])).is_err());
        let many = Json::Arr((0..40).map(|_| obj(&[("url", Json::Str("file:///x".into()))])).collect());
        let (items, _) = parse_show(&obj(&[("items", many)])).unwrap();
        assert_eq!(items.len(), MAX_ITEMS);
        let long = "é".repeat(400);
        let (items, _) = parse_show(&obj(&[("items", Json::Arr(vec![obj(&[("url", Json::Str(long.clone())), ("title", Json::Str(long))])]))])).unwrap();
        assert!(items[0].url.len() <= MAX_URL && items[0].title.len() <= MAX_TITLE);
        assert!(items[0].url.is_char_boundary(items[0].url.len()));
    }

    #[test]
    fn the_service_is_registered_under_its_name() {
        assert_eq!(PreviewService.name(), "quick-look");
    }
}
