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
//!
//! The panel's pictures come from the `thumbnails` service: this service is its one client in the shell. It asks for the
//! panel's current file when the body waits for it (see `Panel::take_wanted`), and feeds the answer back to the panel.
use super::thumbnails::Thumbnails;
use super::{Event, SystemService};
use crate::gfx::Image;
use crate::shell::preview_gen::{Exchanger, Generators, Requests, Sockets};
use crate::shell::Shell;
use bifrost_core::json::Json;
use bifrost_proto::peek::{PreviewItem, MAX_ITEMS, MAX_TITLE, MAX_URL};
use bifrost_ui::preview_manifest::Outcome;
use bifrost_ui::thumb_service::{ThumbKind, ThumbRequest};
use std::path::Path;
use std::sync::Arc;

/// The asker of the panel's thumbnail requests: the shell itself, so the answers come back to this service.
const PANEL: &str = "shell.quick-look";
/// The edge of the panel's picture in points. The panel fits the picture into its body.
const PICTURE_PT: u32 = 512;

pub struct PreviewService {
    thumbs: Thumbnails,
    /// the app generators in flight (see `preview_gen`): a file of a declared type is drawn by its app first
    requests: Requests,
    exchanger: Arc<dyn Exchanger>,
    /// the shell's clock in milliseconds, as of the last call or tick: the generators' deadlines run on it
    now_ms: u64,
}

impl Default for PreviewService {
    fn default() -> Self {
        PreviewService { thumbs: Thumbnails::default(), requests: Requests::default(), exchanger: Arc::new(Sockets), now_ms: 0 }
    }
}

/// The picture a thumbnail answer names, when it is a ready picture the cache can read. None for an icon, a failure or
/// a pending answer.
fn picture_of(body: &Json) -> Option<Image> {
    if body.str_field("state").as_deref() != Some("ready") || body.str_field("art").as_deref() != Some("pixels") {
        return None;
    }
    let path = body.str_field("image")?;
    Image::from_raw(&std::fs::read(Path::new(&path)).ok()?)
}

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

impl PreviewService {
    /// Asks the thumbnails service for the panel's file when its body waits for a picture. A ready answer is applied at
    /// once; a pending one comes later as a `ready` or `failed` event (see `tick`).
    fn ask(&mut self, shell: &mut Shell) {
        let Some(file) = shell.quick_look_take_wanted() else { return };
        // an app that declared the type draws the picture; the thumbnails service is the fallback (see `fall_back`)
        if let Some(generator) = shell.quick_look_generator_for(&file) {
            self.requests.start(Arc::clone(&self.exchanger), &generator, &file, PICTURE_PT, PICTURE_PT, self.now_ms);
            return;
        }
        self.thumbnails(shell, &file);
    }

    /// Asks the thumbnails service for `file`'s picture: a ready answer is applied at once, a pending one comes later.
    fn thumbnails(&mut self, shell: &mut Shell, file: &str) {
        let answer = ThumbRequest::new(file, PICTURE_PT, 1.0, ThumbKind::Thumbnail).map_err(|e| e.to_string()).and_then(|req| self.thumbs.request(PANEL, req));
        match answer {
            Ok(body) if body.str_field("state").as_deref() == Some("pending") => {}
            Ok(body) => shell.quick_look_thumbnail(file, picture_of(&body)),
            Err(_) => shell.quick_look_thumbnail(file, None),
        }
    }

    /// A generator could not draw `file` (no answer, an error, a late answer, or an unreadable picture): the built-in
    /// preview is shown, and for a picture the thumbnails service is asked.
    fn fall_back(&mut self, shell: &mut Shell, file: &str) {
        if shell.quick_look_generator_fallback(file) {
            if let Some(wanted) = shell.quick_look_take_wanted() {
                self.thumbnails(shell, &wanted);
            }
        }
    }
}

impl SystemService for PreviewService {
    fn name(&self) -> &'static str {
        "quick-look"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        self.now_ms = (t.max(0.0) * 1000.0) as u64;
        match method {
            "show" => {
                let (items, index) = parse_show(params)?;
                // the installed apps' declarations are read per opening, so an install or update shows at once
                shell.quick_look_set_generators(Generators::installed());
                shell.quick_look_show(app_id, items, index);
                self.ask(shell);
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
                    self.ask(shell);
                }
                Ok(shell.quick_look_state())
            }
            _ => Err(format!("quick-look has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        self.now_ms = (t.max(0.0) * 1000.0) as u64;
        // the user's keys step the panel without a call: ask for what the body now waits for
        self.ask(shell);
        for done in self.thumbs.poll().into_iter().filter(|e| e.app.as_deref() == Some(PANEL)) {
            let file = done.body.str_field("file").unwrap_or_default();
            shell.quick_look_thumbnail(&file, picture_of(&done.body));
        }
        // the app generators' answers, and the requests whose deadline passed
        for (file, outcome) in self.requests.poll(self.now_ms) {
            match outcome {
                Outcome::Generated(path) => match std::fs::read(Path::new(&path)).ok().and_then(|bytes| crate::cg::imageio::decode(&bytes)) {
                    Some(picture) => shell.quick_look_thumbnail(&file, Some(picture)),
                    None => self.fall_back(shell, &file),
                },
                Outcome::Generic => self.fall_back(shell, &file),
            }
        }
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
        assert_eq!(PreviewService::default().name(), "quick-look");
    }
}
