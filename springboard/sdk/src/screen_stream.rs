//! Screen streams for an app that holds the Screen permission. `start` asks the shell for a stream of the display;
//! while it runs, the shell sends one service event per frame, `screen_stream` / `frame`. `frame_from_event` reads
//! that event. A `Fresh` frame names a new PNG file; a `Repeated` frame means the picture did not change, and the
//! file named in the last fresh frame still shows it.
//!
//! ```ignore
//! screen_stream::start(ctx, 640, 360, 1.0 / 15.0);
//! // in the app's service_event handler:
//! if let Some(frame) = screen_stream::frame_from_event(service, event, body) { /* read frame.path */ }
//! screen_stream::stop(ctx);
//! ```

use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The shell service name.
pub const SERVICE: &str = "screen_stream";
/// The event name a frame arrives under.
pub const FRAME: &str = "frame";

/// Whether a frame changed the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    Fresh,
    Repeated,
}

/// One frame of the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Counts the frames from 0 for the stream.
    pub index: u64,
    /// When the frame is due, in seconds on the shell's clock.
    pub time: f64,
    pub kind: FrameKind,
    /// The PNG file of the last fresh frame. Empty when the shell could not write it.
    pub path: String,
}

/// What a stream shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    /// The whole display minus these apps (by process id). An empty list is the whole display.
    DisplayExcept(Vec<u32>),
    /// One window, by the id the shell reports for it.
    Window(u32),
    /// The windows of one app (by process id).
    App(u32),
}

/// The `start` params: the size, `minFrameInterval` (seconds between frames), the filter, and `record` (a movie file
/// name, when the stream should also be written to a movie).
pub fn start_params(width: u32, height: u32, min_frame_interval: f64, filter: &Filter, record: Option<&str>) -> Json {
    let mut fields = vec![
        ("width".into(), Json::Num(f64::from(width))),
        ("height".into(), Json::Num(f64::from(height))),
        ("minFrameInterval".into(), Json::Num(min_frame_interval)),
    ];
    match filter {
        Filter::DisplayExcept(pids) => fields.push(("excludeApps".into(), Json::Arr(pids.iter().map(|p| Json::Num(f64::from(*p))).collect()))),
        Filter::Window(id) => fields.push(("window".into(), Json::Num(f64::from(*id)))),
        Filter::App(pid) => fields.push(("app".into(), Json::Num(f64::from(*pid)))),
    }
    if let Some(name) = record {
        fields.push(("record".into(), Json::Str(name.into())));
    }
    Json::Obj(fields)
}

/// Starts a stream. Returns the request id; the shell refuses the start unless the app holds the Screen permission.
/// A later `start` replaces the running stream.
pub fn start(ctx: &Ctx, width: u32, height: u32, min_frame_interval: f64, filter: &Filter, record: Option<&str>) -> u32 {
    service::call(ctx, SERVICE, "start", &start_params(width, height, min_frame_interval, filter, record))
}

/// Stops the stream.
pub fn stop(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "stop", &Json::Obj(Vec::new()))
}

/// The frame a shell service event carries, or None when it is another event or its body is malformed.
pub fn frame_from_event(service: &str, event: &str, body: &Json) -> Option<Frame> {
    if service != SERVICE || event != FRAME {
        return None;
    }
    let index = body.get("index")?.number()?;
    let time = body.get("time")?.number()?;
    if index < 0.0 || index.fract() != 0.0 || !time.is_finite() {
        return None;
    }
    let kind = match body.get("kind")?.as_str()? {
        "fresh" => FrameKind::Fresh,
        "repeated" => FrameKind::Repeated,
        _ => return None,
    };
    let path = body.get("path").and_then(Json::as_str).unwrap_or_default().to_string();
    Some(Frame { index: index as u64, time, kind, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(kind: &str) -> Json {
        Json::Obj(vec![
            ("index".into(), Json::Num(4.0)),
            ("time".into(), Json::Num(0.5)),
            ("kind".into(), Json::Str(kind.into())),
            ("path".into(), Json::Str("/tmp/app.png".into())),
        ])
    }

    #[test]
    fn a_frame_event_is_read_back() {
        let f = frame_from_event(SERVICE, FRAME, &body("repeated")).unwrap();
        assert_eq!(f, Frame { index: 4, time: 0.5, kind: FrameKind::Repeated, path: "/tmp/app.png".into() });
        assert_eq!(frame_from_event(SERVICE, FRAME, &body("fresh")).unwrap().kind, FrameKind::Fresh);
    }

    #[test]
    fn other_events_and_bad_bodies_are_not_frames() {
        assert!(frame_from_event("dock", FRAME, &body("fresh")).is_none());
        assert!(frame_from_event(SERVICE, "stopped", &body("fresh")).is_none());
        assert!(frame_from_event(SERVICE, FRAME, &body("blurry")).is_none());
        let no_index = Json::Obj(vec![("time".into(), Json::Num(0.0)), ("kind".into(), Json::Str("fresh".into()))]);
        assert!(frame_from_event(SERVICE, FRAME, &no_index).is_none());
    }

    #[test]
    fn a_missing_path_is_empty_not_an_error() {
        let b = Json::Obj(vec![("index".into(), Json::Num(0.0)), ("time".into(), Json::Num(0.0)), ("kind".into(), Json::Str("fresh".into()))]);
        assert_eq!(frame_from_event(SERVICE, FRAME, &b).unwrap().path, "");
    }

    #[test]
    fn the_start_params_carry_the_size_and_interval() {
        let p = start_params(640, 360, 0.1, &Filter::DisplayExcept(Vec::new()), None);
        assert_eq!(p.get("width").and_then(Json::number), Some(640.0));
        assert_eq!(p.get("height").and_then(Json::number), Some(360.0));
        assert_eq!(p.get("minFrameInterval").and_then(Json::number), Some(0.1));
        assert!(p.get("record").is_none());
    }

    #[test]
    fn the_filter_and_movie_name_go_in_the_start_params() {
        let window = start_params(1, 1, 0.1, &Filter::Window(7), None);
        assert_eq!(window.get("window").and_then(Json::number), Some(7.0));
        let app = start_params(1, 1, 0.1, &Filter::App(42), None);
        assert_eq!(app.get("app").and_then(Json::number), Some(42.0));
        let except = start_params(1, 1, 0.1, &Filter::DisplayExcept(vec![3, 9]), Some("clip.mov"));
        let list = except.get("excludeApps").and_then(Json::as_array).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(except.get("record").and_then(Json::as_str), Some("clip.mov"));
    }
}
