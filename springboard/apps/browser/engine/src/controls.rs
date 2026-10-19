//! Native picker replies and retirement of page-owned prompts.
use super::*;
use std::path::PathBuf;

pub(super) struct UnloadRequest(Option<servo::AllowOrDenyRequest>);
impl UnloadRequest {
    pub(super) fn new(request: servo::AllowOrDenyRequest) -> Self { Self(Some(request)) }
    fn answer(mut self, allow: bool) {
        if let Some(request) = self.0.take() {
            if allow { request.allow(); } else { request.deny(); }
        }
    }
}
impl Drop for UnloadRequest {
    fn drop(&mut self) {
        // Servo's default is allow: unresolved confirmation must explicitly deny instead.
        if let Some(request) = self.0.take() { request.deny(); }
    }
}

impl Engine {
    pub(super) fn retire_prompts(&mut self, view: Option<u32>) {
        let tokens: Vec<_> = self.controls.iter().filter(|(_, (id, _, _))| view.is_none_or(|view| view == *id))
            .map(|(token, (id, _, _))| (*token, *id)).collect();
        for (token, id) in tokens {
            self.controls.remove(&token);
            say(&format!("control-hide {id} {token}"));
        }
        let tokens: Vec<_> = self.unloads.iter().filter(|(_, (id, _))| view.is_none_or(|view| view == *id))
            .map(|(token, (id, _))| (*token, *id)).collect();
        for (token, id) in tokens {
            self.unloads.remove(&token);
            say(&format!("control-hide {id} {token}"));
        }
    }

    pub(super) fn show_unload(&mut self, view: u32, request: UnloadRequest) {
        if self.unloads.len() >= MAX_CONTROLS {
            // Reject excess requests; never silently allow unloading a document with unsaved work.
            return;
        }
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.unloads.insert(token, (view, request));
        say(&format!("unload {view} {token}"));
    }

    pub(super) fn picker_reply(&mut self, verb: &str, view: u32, token: u32, payload: &str) {
        match verb {
            "unload-reply" => {
                if !matches!(payload, "allow" | "cancel") || !self.unloads.get(&token).is_some_and(|(id, _)| *id == view) { return; }
                if let Some((_, request)) = self.unloads.remove(&token) { request.answer(payload == "allow"); }
            }
            "file-reply" => {
                if payload.len() > 64 * 1024 { return; }
                let Some((id, _, Pending::File(picker))) = self.controls.get(&token) else { return };
                if *id != view { return; }
                let Ok(paths) = serde_json::from_str::<Option<Vec<String>>>(payload) else { return };
                let paths = match paths {
                    None => None,
                    Some(paths) => {
                        if paths.len() > 128 || (!picker.allow_select_multiple() && paths.len() > 1) { return; }
                        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
                        if paths.iter().any(|path| !path.is_absolute() || path.as_os_str().len() > 4096 || path.as_os_str().as_encoded_bytes().contains(&0) || !path.is_file()) { return; }
                        Some(paths)
                    }
                };
                if let Some((_, _, Pending::File(mut picker))) = self.controls.remove(&token) {
                    match paths {
                        Some(paths) => { picker.select(&paths); picker.submit(); }
                        None => picker.dismiss(),
                    }
                }
            }
            "color-reply" => {
                if !self.controls.get(&token).is_some_and(|(id, _, pending)| *id == view && matches!(pending, Pending::Color(_))) { return; }
                let fields: Vec<_> = payload.split(' ').collect();
                let color = match fields.as_slice() {
                    ["cancel"] => None,
                    ["ok", red, green, blue] => {
                        let (Ok(red), Ok(green), Ok(blue)) = (red.parse::<u8>(), green.parse::<u8>(), blue.parse::<u8>()) else { return };
                        Some(servo::RgbColor { red, green, blue })
                    }
                    _ => return,
                };
                if let Some((_, _, Pending::Color(mut picker))) = self.controls.remove(&token) {
                    // Dropping an unchanged picker cancels; select(None) would reset to default.
                    if let Some(color) = color { picker.select(Some(color)); picker.submit(); }
                }
            }
            _ => return,
        }
        self.dirty = true;
    }
}
