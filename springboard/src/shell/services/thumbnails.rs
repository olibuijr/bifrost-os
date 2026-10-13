//! The `thumbnails` service: thumbnails and type icons of files for every app. The SDK client is
//! `bifrost_app::thumbnails`; the model and the cache key are `bifrost_ui::thumb_service`; the providers are in
//! `springboard::thumbs`.
//!
//! Method `request` `{path, size, scale, kind}` (`kind` is `"icon"` or `"thumbnail"`; `size` is points) answers at
//! once with `{state, key, file, px, ...}`:
//! - `state` `"ready"` with `art` `"pixels"` (`image`: the cached raw image file, `w`, `h`) or `"icon"` (`type`,
//!   `badge`): nothing is left to do.
//! - `state` `"pending"`: the thumbnail is being drawn on a worker thread. The same object arrives later as the
//!   event `ready` (or `failed` with an `error`), sent to every app that asked for that key.
//!
//! The service reads the file as the shell, so the path must be absolute with no `..` part and the file must be
//! one the shell user can read. At most [`WORKERS`] thumbnails are drawn at a time; the rest wait in order.
use super::{Event, SystemService};
use crate::shell::Shell;
use crate::thumbs::{Answer, Generator};
use bifrost_core::json::Json;
use bifrost_ui::thumb_service::{ThumbKind, ThumbRequest};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

/// Thumbnails drawn at the same time.
pub const WORKERS: usize = 3;
/// Requests allowed to wait.
pub const MAX_QUEUE: usize = 512;
/// Icon answers remembered for files whose thumbnail could not be drawn, so they are not tried again.
const MAX_MEMO: usize = 4096;

struct Job {
    key: String,
    req: ThumbRequest,
}

struct Done {
    key: String,
    file: String,
    px: u32,
    result: Result<Answer, String>,
}

pub struct Thumbnails {
    generator: Generator,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    /// Apps waiting for a key, and the request that key came from.
    waiting: HashMap<String, Vec<String>>,
    queue: VecDeque<Job>,
    running: usize,
    memo: HashMap<String, Answer>,
}

impl Default for Thumbnails {
    fn default() -> Self {
        Thumbnails::with_generator(Generator::standard())
    }
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The JSON of an answer, with the identity of the request it answers.
pub fn answer_json(key: &str, file: &str, px: u32, answer: &Answer) -> Json {
    let mut pairs = vec![("state", Json::Str("ready".into())), ("key", Json::Str(key.into())), ("file", Json::Str(file.into())), ("px", Json::Num(px as f64))];
    match answer {
        Answer::Pixels { path, w, h, .. } => {
            pairs.push(("art", Json::Str("pixels".into())));
            pairs.push(("image", Json::Str(path.to_string_lossy().into_owned())));
            pairs.push(("w", Json::Num(*w as f64)));
            pairs.push(("h", Json::Num(*h as f64)));
        }
        Answer::Icon { type_id, badge } => {
            pairs.push(("art", Json::Str("icon".into())));
            pairs.push(("type", Json::Str(type_id.clone())));
            pairs.push(("badge", badge.clone().map_or(Json::Null, Json::Str)));
        }
    }
    obj(pairs)
}

fn pending_json(key: &str, file: &str, px: u32) -> Json {
    obj(vec![("state", Json::Str("pending".into())), ("key", Json::Str(key.into())), ("file", Json::Str(file.into())), ("px", Json::Num(px as f64))])
}

/// The request in a call's parameters.
pub fn parse_request(params: &Json) -> Result<ThumbRequest, String> {
    let path = params.get("path").and_then(Json::as_str).ok_or("request needs a path")?;
    let size = params.get("size").and_then(Json::as_u64).ok_or("request needs a size in points")?;
    let scale = params.get("scale").and_then(Json::number).unwrap_or(1.0) as f32;
    let kind = match params.get("kind").and_then(Json::as_str) {
        None => ThumbKind::Thumbnail,
        Some(k) => ThumbKind::parse(k).ok_or("kind is icon or thumbnail")?,
    };
    ThumbRequest::new(path, size.min(u32::MAX as u64) as u32, scale, kind).map_err(|e| e.to_string())
}

impl Thumbnails {
    pub fn with_generator(generator: Generator) -> Self {
        let (tx, rx) = channel();
        Thumbnails { generator, tx, rx, waiting: HashMap::new(), queue: VecDeque::new(), running: 0, memo: HashMap::new() }
    }

    /// Answers a request now when it can, else queues it for `app_id`.
    pub fn request(&mut self, app_id: &str, req: ThumbRequest) -> Result<Json, String> {
        let (key, quick) = self.generator.quick(&req)?;
        if let Some(a) = quick.or_else(|| self.memo.get(&key).cloned()) {
            return Ok(answer_json(&key, &req.path, req.px(), &a));
        }
        let (file, px) = (req.path.clone(), req.px());
        match self.waiting.get_mut(&key) {
            Some(apps) => {
                if !apps.iter().any(|a| a == app_id) {
                    apps.push(app_id.to_string());
                }
            }
            None => {
                if self.queue.len() >= MAX_QUEUE {
                    return Err("too many thumbnails waiting".into());
                }
                self.waiting.insert(key.clone(), vec![app_id.to_string()]);
                self.queue.push_back(Job { key: key.clone(), req });
            }
        }
        Ok(pending_json(&key, &file, px))
    }

    /// Starts queued jobs while workers are free.
    fn start_jobs(&mut self) {
        while self.running < WORKERS {
            let Some(job) = self.queue.pop_front() else { break };
            self.running += 1;
            let tx = self.tx.clone();
            let cache = self.generator.cache().clone();
            std::thread::spawn(move || {
                // providers are not Send: each worker builds its own generator over the shared cache directory
                let g = Generator::new(cache);
                let result = g.generate(&job.req);
                let _ = tx.send(Done { key: job.key, file: job.req.path.clone(), px: job.req.px(), result });
            });
        }
    }

    /// Collects finished jobs and returns the events for the apps that waited.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.running = self.running.saturating_sub(1);
            let apps = self.waiting.remove(&done.key).unwrap_or_default();
            let (name, body) = match &done.result {
                Ok(a) => {
                    if matches!(a, Answer::Icon { .. }) {
                        if self.memo.len() >= MAX_MEMO {
                            self.memo.clear();
                        }
                        self.memo.insert(done.key.clone(), a.clone());
                    }
                    ("ready", answer_json(&done.key, &done.file, done.px, a))
                }
                Err(e) => {
                    let mut b = pending_json(&done.key, &done.file, done.px);
                    if let Json::Obj(p) = &mut b {
                        p[0].1 = Json::Str("failed".into());
                        p.push(("error".into(), Json::Str(e.clone())));
                    }
                    ("failed", b)
                }
            };
            for app in apps {
                events.push(Event { app: Some(app), event: name.to_string(), body: body.clone() });
            }
        }
        self.start_jobs();
        events
    }
}

impl SystemService for Thumbnails {
    fn name(&self) -> &'static str {
        "thumbnails"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "request" => {
                let r = self.request(app_id, parse_request(params)?);
                self.start_jobs();
                r
            }
            _ => Err(format!("thumbnails has no method {method}")),
        }
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::photos::thumbs::DiskCache;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-thumbsvc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn service(dir: &std::path::Path) -> Thumbnails {
        Thumbnails::with_generator(Generator::new(DiskCache::new(dir.join("cache"), 100)))
    }

    fn png_file(dir: &std::path::Path, name: &str) -> String {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 32, 32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&vec![50u8; 32 * 32 * 4]).unwrap();
        }
        let p = dir.join(name);
        std::fs::write(&p, out).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn wait_events(s: &mut Thumbnails, n: usize) -> Vec<Event> {
        let end = Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while all.len() < n && Instant::now() < end {
            all.extend(s.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    #[test]
    fn parameters_are_checked() {
        let j = |s: &str| bifrost_core::json::parse(s).unwrap();
        let r = parse_request(&j(r#"{"path":"/a/b.png","size":64,"scale":2,"kind":"icon"}"#)).unwrap();
        assert_eq!((r.size, r.scale, r.kind), (64, 2.0, ThumbKind::Icon));
        assert_eq!(parse_request(&j(r#"{"path":"/a","size":8}"#)).unwrap().kind, ThumbKind::Thumbnail);
        assert!(parse_request(&j(r#"{"size":8}"#)).is_err());
        assert!(parse_request(&j(r#"{"path":"/a"}"#)).is_err());
        assert!(parse_request(&j(r#"{"path":"rel","size":8}"#)).is_err());
        assert!(parse_request(&j(r#"{"path":"/a","size":8,"kind":"poster"}"#)).is_err());
    }

    #[test]
    fn a_thumbnail_is_pending_then_ready_for_every_asker() {
        let dir = scratch("flow");
        let file = png_file(&dir, "a.png");
        let mut s = service(&dir);
        let req = || ThumbRequest::new(&file, 16, 1.0, ThumbKind::Thumbnail).unwrap();
        let a = s.request("app.one", req()).unwrap();
        assert_eq!(a.get("state").and_then(Json::as_str), Some("pending"));
        // a second asker and a repeat from the first share the one job
        s.request("app.two", req()).unwrap();
        s.request("app.one", req()).unwrap();
        assert_eq!(s.queue.len(), 1);
        s.start_jobs();
        let events = wait_events(&mut s, 2);
        assert_eq!(events.len(), 2, "{events:?}");
        let mut apps: Vec<_> = events.iter().map(|e| e.app.clone().unwrap()).collect();
        apps.sort();
        assert_eq!(apps, vec!["app.one", "app.two"]);
        let body = &events[0].body;
        assert_eq!(events[0].event, "ready");
        assert_eq!(body.get("art").and_then(Json::as_str), Some("pixels"));
        assert_eq!(body.get("w").and_then(Json::as_u64), Some(16));
        assert!(std::path::Path::new(body.get("image").and_then(Json::as_str).unwrap()).exists());
        // the next ask is answered from the cache
        let again = s.request("app.one", req()).unwrap();
        assert_eq!(again.get("state").and_then(Json::as_str), Some("ready"));
    }

    #[test]
    fn icons_and_folders_answer_at_once() {
        let dir = scratch("icon");
        let mut s = service(&dir);
        let r = ThumbRequest::new("/x/y.docx", 32, 1.0, ThumbKind::Icon).unwrap();
        let a = s.request("app", r).unwrap();
        assert_eq!(a.get("art").and_then(Json::as_str), Some("icon"));
        assert_eq!(a.get("badge").and_then(Json::as_str), Some("DOCX"));
        let f = ThumbRequest::new(dir.to_str().unwrap(), 32, 1.0, ThumbKind::Thumbnail).unwrap();
        assert_eq!(s.request("app", f).unwrap().get("type").and_then(Json::as_str), Some("public.folder"));
    }

    #[test]
    fn an_undrawable_file_gets_its_icon_and_is_remembered() {
        let dir = scratch("memo");
        let path = dir.join("bad.png");
        std::fs::write(&path, b"nope").unwrap();
        let mut s = service(&dir);
        let req = || ThumbRequest::new(path.to_str().unwrap(), 32, 1.0, ThumbKind::Thumbnail).unwrap();
        assert_eq!(s.request("app", req()).unwrap().get("state").and_then(Json::as_str), Some("pending"));
        s.start_jobs();
        let ev = wait_events(&mut s, 1);
        assert_eq!(ev[0].body.get("art").and_then(Json::as_str), Some("icon"));
        let again = s.request("app", req()).unwrap();
        assert_eq!(again.get("state").and_then(Json::as_str), Some("ready"));
    }

    #[test]
    fn a_missing_file_is_an_error_reply() {
        let dir = scratch("gone");
        let mut s = service(&dir);
        let r = ThumbRequest::new(dir.join("gone.png").to_str().unwrap(), 32, 1.0, ThumbKind::Thumbnail).unwrap();
        assert!(s.request("app", r).is_err());
    }

    #[test]
    fn the_queue_is_bounded() {
        let dir = scratch("bound");
        let mut s = service(&dir);
        let file = png_file(&dir, "q.png");
        for i in 0..MAX_QUEUE {
            // a distinct pixel edge per request makes a distinct key (scale changes can land on an edge already used)
            let r = ThumbRequest::new(&file, 16 + i as u32, 1.0, ThumbKind::Thumbnail).unwrap();
            s.request("app", r).unwrap();
        }
        let extra = ThumbRequest::new(&file, 1000, 1.0, ThumbKind::Thumbnail).unwrap();
        assert!(s.request("app", extra).is_err());
    }
}
