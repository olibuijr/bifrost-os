//! App preview generators for the preview panel. An installed app declares `[[preview]]` blocks in its manifest (see
//! `bifrost_ui::preview_manifest`): the file types it can draw, the control method that draws them and how long the
//! shell waits. When the panel shows a file of a declared type, the shell asks the app over its control socket
//! (`preview.render` with `{"file", "width", "height"}`), on a worker thread so the desktop never waits. The app answers
//! `{"picture": "<absolute path to a PNG>"}`; that picture is drawn in the panel. A late, missing or unusable answer, or
//! no app at all, falls back to the built-in preview of the file (see `preview_panel::Panel::generator_fallback`).
//!
//! [`Generators`] holds the declarations and picks the generator for a file name through the preview registry, so a
//! type's most specific declaration wins, as the built-in providers do. [`Requests`] runs the calls and decides each
//! outcome with a [`Wait`]. The socket is behind [`Exchanger`], so the dispatch and the fallback are tested with a fake.

use bifrost_core::json::Json;
use bifrost_ui::preview::Registry;
use bifrost_ui::preview_manifest::{parse_manifest, Outcome, PreviewDecl, Wait};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

/// The control method a generator answers.
pub const METHOD: &str = "preview.render";

/// One declared generator: the app that answers and its `[[preview]]` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generator {
    pub app_id: String,
    pub decl: PreviewDecl,
}

impl Generator {
    /// The registry key of this declaration: unique across apps and blocks.
    fn key(&self) -> String {
        format!("{}#{}", self.app_id, self.decl.id)
    }
}

/// Sends one request line to an app and returns its raw reply. Blocking: the shell calls it on a worker thread.
pub trait Exchanger: Send + Sync {
    fn exchange(&self, app_id: &str, line: &str, wait: Duration) -> Result<String, String>;
}

/// The shell's exchanger: the app's control socket, as App Intents use it (`intents::exchange`).
pub struct Sockets;

impl Exchanger for Sockets {
    fn exchange(&self, app_id: &str, line: &str, wait: Duration) -> Result<String, String> {
        super::intents::exchange(app_id, line, wait)
    }
}

/// The generators of the installed apps, and the registry that picks one for a file name.
#[derive(Default)]
pub struct Generators {
    list: Vec<Generator>,
    registry: Option<Registry>,
}

impl Generators {
    /// Builds the lookup for `list`. A block whose types the registry refuses is kept in the list but never picked.
    pub fn new(list: Vec<Generator>) -> Self {
        let mut registry = Registry::new();
        for g in &list {
            let types: Vec<&str> = g.decl.types.iter().map(String::as_str).collect();
            let _ = registry.register(&g.key(), &types);
        }
        Generators { list, registry: Some(registry) }
    }

    /// The declarations of every installed app, read from the manifests the apps ship (`[[preview]]` blocks).
    pub fn installed() -> Self {
        let mut list = Vec::new();
        for app in super::apphost::installed().iter() {
            let Ok(text) = std::fs::read_to_string(app.dir.join("manifest.toml")) else { continue };
            let Ok(decls) = parse_manifest(&text) else { continue };
            list.extend(decls.into_iter().map(|decl| Generator { app_id: app.manifest.id.clone(), decl }));
        }
        Generators::new(list)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The generator for a file name, when an app declared its type. None for a file no app can draw.
    pub fn for_file(&self, file_name: &str) -> Option<&Generator> {
        let key = self.registry.as_ref()?.provider_for(file_name)?;
        self.list.iter().find(|g| g.key() == key)
    }
}

/// The request line that asks an app to draw `file` at `width` by `height` pixels.
pub fn request_line(file: &str, width: u32, height: u32) -> String {
    let params = Json::Obj(vec![
        ("file".into(), Json::Str(file.to_string())),
        ("width".into(), Json::Num(f64::from(width))),
        ("height".into(), Json::Num(f64::from(height))),
    ]);
    format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"{METHOD}\",\"params\":{}}}\n", params.encode())
}

/// The picture path an app's reply names. None for an error reply, a reply with no `picture`, or one that is not JSON.
pub fn parse_reply(reply: &str) -> Option<String> {
    let json = bifrost_core::json::parse(reply.trim()).ok()?;
    if json.get("error").is_some() {
        return None;
    }
    json.get("result")?.get("picture")?.as_str().map(str::to_string)
}

/// A generator request in flight.
struct Request {
    file: String,
    wait: Wait,
    rx: Receiver<Result<String, String>>,
}

/// The requests in flight. Each starts a worker thread; [`Requests::poll`] takes the answers as they come.
#[derive(Default)]
pub struct Requests {
    pending: Vec<Request>,
}

impl Requests {
    /// Asks `generator` for a picture of `file`, at `now_ms` on the shell's clock. The reply is due within the
    /// generator's declared timeout, which is also how long the connect to its socket may take.
    pub fn start(&mut self, exchanger: Arc<dyn Exchanger>, generator: &Generator, file: &str, width: u32, height: u32, now_ms: u64) {
        let wait_for = Duration::from_millis(u64::from(generator.decl.timeout_ms));
        let line = request_line(file, width, height);
        let (tx, rx) = channel();
        let app = generator.app_id.clone();
        let wait = Wait::start(now_ms, generator.decl.timeout_ms);
        std::thread::spawn(move || {
            // the panel may have moved on; a reply nobody takes is dropped with the channel
            let _ = tx.send(exchanger.exchange(&app, &line, wait_for));
        });
        self.pending.push(Request { file: file.to_string(), wait, rx });
    }

    /// Takes the answers that came in and the requests whose deadline passed, at `now_ms`. Each request is decided once,
    /// and the result is `(file, outcome)`. A request still inside its deadline stays pending.
    pub fn poll(&mut self, now_ms: u64) -> Vec<(String, Outcome)> {
        let mut done = Vec::new();
        for mut r in std::mem::take(&mut self.pending) {
            match r.rx.try_recv() {
                Ok(Ok(reply)) => {
                    r.wait.answer(now_ms, parse_reply(&reply).as_deref());
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    r.wait.answer(now_ms, None);
                }
                Err(TryRecvError::Empty) => {}
            }
            match r.wait.poll(now_ms).cloned() {
                Some(outcome) => done.push((r.file, outcome)),
                None => self.pending.push(r),
            }
        }
        done
    }

    /// How many requests are waiting for an answer.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Answers every call with a canned reply, after no wait.
    struct Fake(Result<String, String>);

    impl Exchanger for Fake {
        fn exchange(&self, _app_id: &str, _line: &str, _wait: Duration) -> Result<String, String> {
            self.0.clone()
        }
    }

    fn generator(id: &str, types: &[&str], timeout_ms: u32) -> Generator {
        Generator {
            app_id: format!("app.{id}"),
            decl: PreviewDecl { id: id.into(), types: types.iter().map(|t| t.to_string()).collect(), generator: METHOD.into(), timeout_ms },
        }
    }

    /// Polls until the request is decided, for up to two seconds of real time.
    fn settle(reqs: &mut Requests, now_ms: u64) -> Vec<(String, Outcome)> {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let done = reqs.poll(now_ms);
            if !done.is_empty() || Instant::now() >= until {
                return done;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_declared_type_picks_its_app_and_others_fall_back() {
        let gens = Generators::new(vec![generator("text", &["public.plain-text"], 3000)]);
        assert_eq!(gens.for_file("notes.md").map(|g| g.app_id.as_str()), Some("app.text"), "Markdown conforms to plain text");
        assert!(gens.for_file("pic.png").is_none(), "an image is not text");
        assert!(gens.for_file("README").is_none(), "no type, no generator");
        assert!(Generators::default().for_file("a.txt").is_none());
        assert!(Generators::default().is_empty());
    }

    #[test]
    fn the_most_specific_declaration_wins() {
        let gens = Generators::new(vec![generator("plain", &["public.plain-text"], 3000), generator("md", &["net.daringfireball.markdown"], 3000)]);
        assert_eq!(gens.for_file("notes.md").map(|g| g.decl.id.as_str()), Some("md"));
        assert_eq!(gens.for_file("main.rs").map(|g| g.decl.id.as_str()), Some("plain"));
    }

    #[test]
    fn the_request_names_the_file_and_the_size() {
        let line = request_line("/home/u/a.md", 512, 384);
        assert!(line.ends_with('\n'));
        let json = bifrost_core::json::parse(line.trim()).unwrap();
        assert_eq!(json.get("method").and_then(Json::as_str), Some(METHOD));
        let params = json.get("params").unwrap();
        assert_eq!(params.get("file").and_then(Json::as_str), Some("/home/u/a.md"));
        assert_eq!(params.get("width").and_then(Json::as_u64), Some(512));
        assert_eq!(params.get("height").and_then(Json::as_u64), Some(384));
    }

    #[test]
    fn a_picture_reply_names_its_path_and_an_error_names_none() {
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"picture":"/tmp/p.png"}}"#), Some("/tmp/p.png".into()));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"no"}}"#), None);
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":null}}"#), None);
        assert_eq!(parse_reply("not json"), None);
    }

    #[test]
    fn a_picture_the_app_returns_is_the_outcome() {
        let mut reqs = Requests::default();
        let g = generator("text", &["public.plain-text"], 3000);
        let fake: Arc<dyn Exchanger> = Arc::new(Fake(Ok(r#"{"result":{"picture":"/tmp/out.png"}}"#.into())));
        reqs.start(fake, &g, "/home/u/a.md", 512, 384, 0);
        assert_eq!(reqs.len(), 1);
        let done = settle(&mut reqs, 10);
        assert_eq!(done, vec![("/home/u/a.md".to_string(), Outcome::Generated("/tmp/out.png".into()))]);
        assert!(reqs.is_empty(), "a decided request leaves the queue");
    }

    #[test]
    fn an_error_from_the_app_falls_back_to_the_built_in_preview() {
        let mut reqs = Requests::default();
        let g = generator("text", &["public.plain-text"], 3000);
        let fake: Arc<dyn Exchanger> = Arc::new(Fake(Err("no socket".into())));
        reqs.start(fake, &g, "/home/u/a.md", 512, 384, 0);
        assert_eq!(settle(&mut reqs, 10), vec![("/home/u/a.md".to_string(), Outcome::Generic)]);
    }

    #[test]
    fn an_answer_after_the_declared_timeout_is_refused() {
        let mut reqs = Requests::default();
        let g = generator("text", &["public.plain-text"], 100);
        let fake: Arc<dyn Exchanger> = Arc::new(Fake(Ok(r#"{"result":{"picture":"/tmp/late.png"}}"#.into())));
        reqs.start(fake, &g, "/home/u/a.md", 512, 384, 0);
        // the clock is past the deadline: whatever arrives, the fallback is the outcome
        assert_eq!(settle(&mut reqs, 500), vec![("/home/u/a.md".to_string(), Outcome::Generic)]);
    }

    #[test]
    fn a_request_inside_its_deadline_stays_pending() {
        let mut reqs = Requests::default();
        let g = generator("text", &["public.plain-text"], 3000);
        // a fake that never answers in time: nothing is decided before the deadline
        struct Silent;
        impl Exchanger for Silent {
            fn exchange(&self, _: &str, _: &str, _: Duration) -> Result<String, String> {
                std::thread::sleep(Duration::from_millis(200));
                Err("slow".into())
            }
        }
        reqs.start(Arc::new(Silent), &g, "/home/u/a.md", 512, 384, 0);
        assert!(reqs.poll(10).is_empty(), "inside the deadline and no answer yet: still pending");
        assert_eq!(reqs.len(), 1);
    }
}
