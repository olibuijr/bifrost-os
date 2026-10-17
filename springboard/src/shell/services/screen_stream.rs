//! The `screen_stream` service: a screen stream for an app that holds the Screen permission. The app calls
//! `start({width, height, minFrameInterval})` and `stop()`. While the stream runs, the shell draws the display the way
//! a screenshot does, once each time a frame is due (`bifrost_ui::capture_stream::ScreenStream`), and sends each frame
//! to the app as the service event `screen_stream` / `frame` with `{index, time, kind, path}`. `kind` is `fresh` when
//! the picture changed and `repeated` when it did not; a repeated frame names the last fresh frame's PNG file. The SDK
//! side is `bifrost_app::screen_stream`.
//!
//! Frames are PNG files in `ScreenStream/` under the shell's data directory, one file per app, replaced atomically.
//! The pointer is not drawn into the stream yet.

use super::display_services::require_screen_recording;
use super::privacy_permissions;
use super::{Event, Shell, SystemService};
use crate::gfx::{Image, Surface};
use bifrost_core::json::Json;
use bifrost_ui::capture_config::StreamConfig;
use bifrost_ui::capture_stream::{Frame, FrameKind, ScreenStream};
use std::collections::HashMap;
use std::path::PathBuf;

/// The service event name the frames arrive under.
pub const FRAME_EVENT: &str = "frame";
/// The default frame interval in seconds (30 frames a second).
pub const DEFAULT_INTERVAL: f64 = 1.0 / 30.0;

/// One running stream: its pacing, the hash of the last picture, and the file of the last fresh frame.
struct Active {
    stream: ScreenStream,
    hash: Option<u64>,
    path: Option<PathBuf>,
}

#[derive(Default)]
pub struct ScreenStreamService {
    active: HashMap<String, Active>,
}

/// The stream config an app asks for. `width` and `height` default to the display size; `minFrameInterval` is in
/// seconds and defaults to `DEFAULT_INTERVAL`.
pub fn config_from(params: &Json, app_id: &str, display_w: u32, display_h: u32) -> StreamConfig {
    let number = |key: &str| params.get(key).and_then(Json::number);
    StreamConfig {
        width: number("width").map_or(display_w, |v| v.max(0.0) as u32),
        height: number("height").map_or(display_h, |v| v.max(0.0) as u32),
        min_frame_interval: number("minFrameInterval").unwrap_or(DEFAULT_INTERVAL),
        stream_name: app_id.to_string(),
        ..StreamConfig::default()
    }
}

/// A 64-bit FNV-1a hash of the picture's pixels. Equal pictures hash equal; a change almost always changes the hash.
pub fn frame_hash(px: &[u32]) -> u64 {
    px.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &p| (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3))
}

/// The file name of an app's frames: the app id with every character but letters, digits, `-` and `.` replaced.
pub fn file_stem(app_id: &str) -> String {
    let clean: String = app_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    if clean.is_empty() {
        "app".to_string()
    } else {
        clean
    }
}

pub fn kind_name(kind: FrameKind) -> &'static str {
    match kind {
        FrameKind::Fresh => "fresh",
        FrameKind::Repeated => "repeated",
    }
}

/// The body of a frame event.
pub fn frame_body(frame: &Frame, path: &str) -> Json {
    Json::Obj(vec![
        ("index".into(), Json::Num(frame.index as f64)),
        ("time".into(), Json::Num(frame.time)),
        ("kind".into(), Json::Str(kind_name(frame.kind).into())),
        ("path".into(), Json::Str(path.into())),
    ])
}

impl SystemService for ScreenStreamService {
    fn name(&self) -> &'static str {
        "screen_stream"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        match method {
            "start" => {
                require_screen_recording(app_id)?;
                let config = config_from(params, app_id, shell.w as u32, shell.h as u32);
                let stream = ScreenStream::start(&privacy_permissions::access(app_id), config, t / 1000.0).map_err(|e| e.describe().to_string())?;
                self.active.insert(app_id.to_string(), Active { stream, hash: None, path: None });
                Ok(Json::Obj(Vec::new()))
            }
            "stop" => {
                self.active.remove(app_id);
                Ok(Json::Obj(Vec::new()))
            }
            _ => Err(format!("screen_stream has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let now = t / 1000.0;
        let mut due: Vec<String> = self.active.iter().filter(|(_, a)| a.stream.is_due(now)).map(|(app, _)| app.clone()).collect();
        if due.is_empty() {
            return Vec::new();
        }
        due.sort();
        // one draw serves every stream that is due this tick
        let mut img = Image::new(shell.w as usize, shell.h as usize);
        shell.drawn = super::super::Drawn::Nothing;
        shell.draw(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, t);
        shell.drawn = super::super::Drawn::Nothing;
        shell.dirty = true;
        let hash = frame_hash(&img.px);
        let (unix, _) = shell.wall_now(t);
        let dir = shell.data_dir.join("ScreenStream");
        let mut out = Vec::new();
        for app in due {
            let Some(a) = self.active.get_mut(&app) else { continue };
            let changed = a.hash != Some(hash);
            let Some(frame) = a.stream.tick(now, changed) else { continue };
            a.hash = Some(hash);
            if frame.kind == FrameKind::Fresh {
                let name = format!("{}.png", file_stem(&app));
                match super::super::screenshot::write(&dir, &name, &super::super::screenshot::encode(&img), unix) {
                    Ok(path) => a.path = Some(path),
                    Err(e) => {
                        eprintln!("springboard: screen stream {app}: {e}");
                        continue;
                    }
                }
            }
            let path = a.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            out.push(Event { app: Some(app.clone()), event: FRAME_EVENT.into(), body: frame_body(&frame, &path) });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_takes_the_app_size_and_interval_or_the_display_defaults() {
        let asked = Json::Obj(vec![("width".into(), Json::Num(640.0)), ("minFrameInterval".into(), Json::Num(0.1))]);
        let c = config_from(&asked, "is.olibuijr.test", 1920, 1080);
        assert_eq!((c.width, c.height), (640, 1080));
        assert_eq!(c.min_frame_interval, 0.1);
        assert_eq!(c.stream_name, "is.olibuijr.test");
        let bare = config_from(&Json::Obj(Vec::new()), "a", 1920, 1080);
        assert_eq!((bare.width, bare.height), (1920, 1080));
        assert_eq!(bare.min_frame_interval, DEFAULT_INTERVAL);
    }

    #[test]
    fn equal_pictures_hash_equal_and_changed_ones_do_not() {
        let a = vec![1u32, 2, 3, 4];
        assert_eq!(frame_hash(&a), frame_hash(&[1, 2, 3, 4]));
        assert_ne!(frame_hash(&a), frame_hash(&[1, 2, 3, 5]));
        assert_ne!(frame_hash(&a), frame_hash(&[1, 2, 3]));
    }

    #[test]
    fn file_names_are_safe_for_any_app_id() {
        assert_eq!(file_stem("is.olibuijr.mail"), "is.olibuijr.mail");
        assert_eq!(file_stem("../etc/passwd"), ".._etc_passwd");
        assert_eq!(file_stem(""), "app");
    }

    #[test]
    fn a_frame_body_names_its_index_time_kind_and_file() {
        let frame = Frame { index: 3, time: 0.25, kind: FrameKind::Repeated };
        let body = frame_body(&frame, "/data/ScreenStream/app.png");
        assert_eq!(body.get("index").and_then(Json::number), Some(3.0));
        assert_eq!(body.get("time").and_then(Json::number), Some(0.25));
        assert_eq!(body.get("kind").and_then(Json::as_str), Some("repeated"));
        assert_eq!(body.get("path").and_then(Json::as_str), Some("/data/ScreenStream/app.png"));
    }
}
