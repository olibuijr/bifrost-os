//! Loading pages, style sheets and images on worker threads. The shell
//! polls for results from its frame loop; a request carries the tab and
//! navigation numbers it was made for so late answers to abandoned
//! navigations are dropped by the caller.
//!
//! HTML and CSS are parsed on the worker, images decoded there too, so the
//! UI thread only styles and lays out.

use super::css::{self, Rule};
use super::html::{self, Node};
use super::nav;
use crate::deildu::http::{Client, Jar, Stream, Transport, BROWSER_USER_AGENT};
use crate::gfx::Image;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

const MAX_WORKERS: usize = 6;
const PAGE_LIMIT: usize = 3 * 1024 * 1024;
const SHEET_LIMIT: usize = 512 * 1024;
const IMAGE_LIMIT: usize = 6 * 1024 * 1024;
const TEXT_LIMIT: usize = 200 * 1024;
const MAX_LINKED_SHEETS: usize = 6;
/// Decoded pictures are shrunk to this many px on a side to bound memory.
const IMAGE_SIDE: usize = 1600;

thread_local! {
    /// The cancel flag of the request the current worker thread is running.
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// True once the request this thread runs was cancelled by a navigation or a close.
fn cancelled() -> bool {
    CANCEL.with(|c| c.borrow().as_ref().is_some_and(|f| f.load(Ordering::SeqCst)))
}

/// A connection whose reads and writes stop once the request on the reading
/// thread is cancelled. Connections are pooled across requests, so the flag
/// is looked up on the thread doing the I/O, not stored with the connection.
struct Abortable(Box<dyn Stream>);

impl Read for Abortable {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if cancelled() {
            return Err(io::Error::other("request cancelled"));
        }
        self.0.read(buf)
    }
}

impl Write for Abortable {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if cancelled() {
            return Err(io::Error::other("request cancelled"));
        }
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// The transport the loader uses: every connection it opens is `Abortable`.
struct Interruptible {
    inner: Arc<dyn Transport>,
}

impl Transport for Interruptible {
    fn connect(&self, host: &str, port: u16, tls: bool) -> io::Result<Box<dyn Stream>> {
        self.inner.connect(host, port, tls).map(|s| Box::new(Abortable(s)) as Box<dyn Stream>)
    }
    fn upgrade(&self, host: &str, stream: Box<dyn Stream>) -> io::Result<Box<dyn Stream>> {
        self.inner.upgrade(host, stream).map(|s| Box::new(Abortable(s)) as Box<dyn Stream>)
    }
}

/// One style source in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum SheetSrc {
    Inline(Vec<Rule>),
    /// an absolute URL still to fetch
    Link(String),
}

#[derive(Debug)]
pub struct Fetched {
    /// where the page came from, after redirects
    pub url: String,
    pub dom: Vec<Node>,
    pub sheets: Vec<SheetSrc>,
    /// the picture itself when the URL was an image
    pub image: Option<Image>,
    /// a stand-in for a page that could not be loaded
    pub failed: bool,
}

#[derive(Debug)]
pub enum Msg {
    Page { tab: u64, nav: u64, result: Result<Fetched, String> },
    Sheet { tab: u64, nav: u64, slot: usize, rules: Vec<Rule> },
    Image { tab: u64, nav: u64, src: String, image: Option<Image> },
}

enum Job {
    Page { tab: u64, nav: u64, url: String },
    Sheet { tab: u64, nav: u64, slot: usize, url: String },
    Image { tab: u64, nav: u64, src: String, url: String },
}

/// Style sources of a parsed page, in the order the cascade sees them.
pub fn collect_sheets(dom: &[Node], base: &str) -> Vec<SheetSrc> {
    fn walk(nodes: &[Node], base: &str, out: &mut Vec<SheetSrc>) {
        for n in nodes {
            let Node::Element(e) = n else { continue };
            let print = e.attr("media").is_some_and(|m| m.to_ascii_lowercase().contains("print"));
            match e.tag.as_str() {
                "style" if !print => out.push(SheetSrc::Inline(css::parse_stylesheet(&e.children.iter().map(Node::text_content).collect::<String>()))),
                "link" if !print && e.attr("rel").is_some_and(|r| r.to_ascii_lowercase().split_ascii_whitespace().any(|x| x == "stylesheet")) => {
                    let linked = out.iter().filter(|s| matches!(s, SheetSrc::Link(_))).count();
                    if let (Some(href), true) = (e.attr("href"), linked < MAX_LINKED_SHEETS) {
                        if let Some(url) = nav::resolve(base, href) {
                            out.push(SheetSrc::Link(url));
                        }
                    }
                }
                _ => walk(&e.children, base, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(dom, base, &mut out);
    out
}

/// A parsed page ready for the tab: its style sources are collected against `url`.
pub fn fetched(url: &str, dom: Vec<Node>) -> Fetched {
    let sheets = collect_sheets(&dom, url);
    Fetched { url: url.to_string(), dom, sheets, image: None, failed: false }
}

/// The page shown in place of one that failed to load.
pub fn error_page(url: &str, message: &str) -> Fetched {
    let dom = html::parse(&format!("<html><head><title>Can't open page</title></head><body><h1>Can't open page</h1><p>{}</p><p>{}</p><p><a href=\"about:home\">Start page</a></p></body></html>", nav::esc(message), nav::esc(url)));
    Fetched { failed: true, ..fetched(url, dom) }
}

/// Turns an HTTP response into a page.
fn page_from(resp: crate::deildu::http::Response) -> Result<Fetched, String> {
    let ct = resp.header("Content-Type").unwrap_or("").to_ascii_lowercase();
    let url = resp.url.clone();
    if crate::cg::imageio::is_image(&resp.body) || ct.starts_with("image/") {
        let img = crate::cg::imageio::decode(&resp.body).ok_or("This picture format is not supported.")?;
        let name = url.rsplit('/').next().unwrap_or("Image").to_string();
        let dom = html::parse(&format!("<html><head><title>{}</title></head><body style=\"background:#000\"><img src=\"{}\"></body></html>", nav::esc(&name), nav::esc(&url)));
        let mut f = fetched(&url, dom);
        f.image = Some(crate::imgdec::contain(&img, IMAGE_SIDE, IMAGE_SIDE * 3));
        return Ok(f);
    }
    if resp.status >= 400 && resp.body.is_empty() {
        return Err(format!("The server answered {}.", resp.status));
    }
    let looks_html = ct.contains("html") || ct.contains("xml") || (ct.is_empty() && resp.body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<'));
    if looks_html {
        return Ok(fetched(&url, html::parse(&resp.text())));
    }
    if ct.starts_with("text/") || ct.contains("json") || ct.contains("javascript") {
        let mut text = resp.text();
        if text.len() > TEXT_LIMIT {
            let mut cut = TEXT_LIMIT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        return Ok(fetched(&url, html::parse(&format!("<html><head><title>{}</title></head><body><pre>{}</pre></body></html>", nav::esc(url.rsplit('/').next().unwrap_or("Text")), nav::esc(&text)))));
    }
    Err(format!("This kind of file ({}) cannot be shown.", if ct.is_empty() { "unknown" } else { ct.split(';').next().unwrap_or("unknown") }))
}

fn run(client: &Client, job: Job) -> Msg {
    match job {
        Job::Page { tab, nav, url } => {
            let result = match client.get(&url, None, PAGE_LIMIT) {
                Ok(resp) => page_from(resp),
                Err(e) => Err(e.to_string()),
            };
            Msg::Page { tab, nav, result }
        }
        Job::Sheet { tab, nav, slot, url } => {
            let rules = client.get(&url, None, SHEET_LIMIT).ok().filter(|r| r.status < 400).map(|r| css::parse_stylesheet(&r.text())).unwrap_or_default();
            Msg::Sheet { tab, nav, slot, rules }
        }
        Job::Image { tab, nav, src, url } => {
            let image = client.get(&url, None, IMAGE_LIMIT).ok().filter(|r| r.status < 400).and_then(|r| crate::cg::imageio::decode(&r.body)).map(|i| crate::imgdec::contain(&i, IMAGE_SIDE, IMAGE_SIDE * 3));
            Msg::Image { tab, nav, src, image }
        }
    }
}

pub struct Loader {
    client: Arc<Client>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    active: Arc<AtomicUsize>,
    /// queued work, each with the cancel flag of its tab at the time it was asked for
    queue: VecDeque<(Job, Arc<AtomicBool>)>,
    /// the live cancel flag of each tab that has work outstanding
    flags: HashMap<u64, Arc<AtomicBool>>,
}

impl Loader {
    pub fn new(transport: Arc<dyn Transport>) -> Loader {
        let (tx, rx) = channel();
        let transport: Arc<dyn Transport> = Arc::new(Interruptible { inner: transport });
        // the Browser loads pages for the person, signs in to sites with their cookies and needs a browser agent
        let client = Client::new(transport, Jar::default()).with_cookies().user_agent(BROWSER_USER_AGENT);
        Loader { client: Arc::new(client), tx, rx, active: Arc::new(AtomicUsize::new(0)), queue: VecDeque::new(), flags: HashMap::new() }
    }

    fn flag(&mut self, tab: u64) -> Arc<AtomicBool> {
        self.flags.entry(tab).or_insert_with(|| Arc::new(AtomicBool::new(false))).clone()
    }

    pub fn page(&mut self, tab: u64, nav: u64, url: &str) {
        // a new page request goes ahead of queued sub-resources
        let flag = self.flag(tab);
        self.queue.push_front((Job::Page { tab, nav, url: url.to_string() }, flag));
        self.pump();
    }

    pub fn sheet(&mut self, tab: u64, nav: u64, slot: usize, url: &str) {
        let flag = self.flag(tab);
        self.queue.push_back((Job::Sheet { tab, nav, slot, url: url.to_string() }, flag));
        self.pump();
    }

    pub fn image(&mut self, tab: u64, nav: u64, src: &str, url: &str) {
        let flag = self.flag(tab);
        self.queue.push_back((Job::Image { tab, nav, src: src.to_string(), url: url.to_string() }, flag));
        self.pump();
    }

    /// Cancels a tab's work (it navigated away or closed): queued requests
    /// are dropped and running ones stop at their next read or write.
    pub fn cancel(&mut self, tab: u64) {
        self.queue.retain(|(j, _)| match j {
            Job::Page { tab: t, .. } | Job::Sheet { tab: t, .. } | Job::Image { tab: t, .. } => *t != tab,
        });
        if let Some(flag) = self.flags.remove(&tab) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn pump(&mut self) {
        while self.active.load(Ordering::SeqCst) < MAX_WORKERS {
            let Some((job, flag)) = self.queue.pop_front() else { break };
            self.active.fetch_add(1, Ordering::SeqCst);
            let (client, tx, active) = (self.client.clone(), self.tx.clone(), self.active.clone());
            std::thread::spawn(move || {
                crate::gfx::affinity::background_thread();
                CANCEL.with(|c| *c.borrow_mut() = Some(flag));
                let msg = run(&client, job);
                // sent before the slot frees, so an idle loader has delivered everything
                let _ = tx.send(msg);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }

    /// Finished work since the last call.
    pub fn poll(&mut self) -> Vec<Msg> {
        let msgs: Vec<Msg> = self.rx.try_iter().collect();
        self.pump();
        msgs
    }

    /// Requests running or waiting.
    pub fn busy(&self) -> bool {
        self.active.load(Ordering::SeqCst) > 0 || !self.queue.is_empty()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::deildu::http::Stream;
    use std::collections::HashMap;
    use std::io::{self, Cursor, Read, Write};
    use std::sync::Mutex;

    type Sites = Arc<Mutex<HashMap<String, (String, Vec<u8>)>>>;

    #[test]
    fn the_browser_is_the_client_that_keeps_cookies() {
        let loader = Loader::new(crate::deildu::http::tests::Fake::new(vec![]));
        assert!(loader.client.keeps_cookies(), "sites sign the person in with cookies");
    }

    /// One connection: remembers the request, answers it from the site table.
    struct Pipe {
        host: String,
        sites: Sites,
        request: Vec<u8>,
        reply: Option<Cursor<Vec<u8>>>,
    }

    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.reply.is_none() {
                let head = String::from_utf8_lossy(&self.request).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let hit = {
                    let sites = self.sites.lock().unwrap_or_else(|p| p.into_inner());
                    let bare = path.split('?').next().unwrap_or(&path);
                    sites.get(&format!("{}{path}", self.host)).or_else(|| sites.get(&format!("{}{bare}", self.host))).or_else(|| sites.get(&self.host)).cloned()
                };
                let (status, ct, body) = match hit {
                    Some((ct, body)) => ("200 OK", ct, body),
                    None => ("404 Not Found", "text/plain".to_string(), b"not found".to_vec()),
                };
                let mut reply = format!("HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                reply.extend_from_slice(&body);
                self.reply = Some(Cursor::new(reply));
            }
            self.reply.as_mut().unwrap().read(buf)
        }
    }

    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.request.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Serves canned pages: keys are `host` or `host/path`, values `(content type, body)`.
    pub struct Fake {
        pub sites: Sites,
    }

    impl Fake {
        pub fn new(sites: &[(&str, &str, &[u8])]) -> Arc<Fake> {
            Arc::new(Fake { sites: Arc::new(Mutex::new(sites.iter().map(|(k, ct, b)| (k.to_string(), (ct.to_string(), b.to_vec()))).collect())) })
        }
    }

    impl Transport for Fake {
        fn connect(&self, host: &str, _port: u16, _tls: bool) -> io::Result<Box<dyn Stream>> {
            let known = self.sites.lock().unwrap_or_else(|p| p.into_inner()).keys().any(|k| k == host || k.starts_with(&format!("{host}/")));
            if !known {
                return Err(io::Error::other("no such host"));
            }
            Ok(Box::new(Pipe { host: host.to_string(), sites: self.sites.clone(), request: vec![], reply: None }))
        }
    }

    pub fn wait(loader: &mut Loader, want: usize) -> Vec<Msg> {
        let mut got = vec![];
        for _ in 0..400 {
            got.extend(loader.poll());
            if got.len() >= want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn sheets_are_collected_in_document_order() {
        let dom = html::parse("<head><link rel=stylesheet href=/a.css><style>p{color:red}</style><link rel=stylesheet media=print href=/p.css><link rel=icon href=/i.png><link rel='preload stylesheet' href=b.css></head>");
        let s = collect_sheets(&dom, "https://x.test/dir/page");
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], SheetSrc::Link("https://x.test/a.css".into()));
        assert!(matches!(&s[1], SheetSrc::Inline(r) if r.len() == 1));
        assert_eq!(s[2], SheetSrc::Link("https://x.test/dir/b.css".into()));
    }

    #[test]
    fn at_most_a_few_linked_sheets_are_followed() {
        let links: String = (0..20).map(|i| format!("<link rel=stylesheet href=/{i}.css>")).collect();
        assert_eq!(collect_sheets(&html::parse(&links), "https://x.test/").len(), MAX_LINKED_SHEETS);
    }

    #[test]
    fn a_page_loads_and_parses_on_a_worker() {
        let t = Fake::new(&[("x.test", "text/html; charset=utf-8", b"<title>Hi</title><p>there</p>")]);
        let mut l = Loader::new(t);
        l.page(1, 7, "http://x.test/");
        let m = wait(&mut l, 1);
        let Some(Msg::Page { tab: 1, nav: 7, result: Ok(f) }) = m.into_iter().next() else { panic!("a page") };
        assert_eq!(f.url, "http://x.test/");
        assert_eq!(html::title(&f.dom).as_deref(), Some("Hi"));
        assert!(!l.busy());
    }

    #[test]
    fn plain_text_becomes_a_preformatted_page_and_binaries_are_refused() {
        let t = Fake::new(&[("t.test", "text/plain", b"a < b\nline2"), ("b.test", "application/pdf", b"%PDF")]);
        let mut l = Loader::new(t);
        l.page(1, 1, "http://t.test/notes.txt");
        l.page(1, 2, "http://b.test/x.pdf");
        let mut m = wait(&mut l, 2);
        m.sort_by_key(|m| if let Msg::Page { nav, .. } = m { *nav } else { 0 });
        let Msg::Page { result: Ok(f), .. } = &m[0] else { panic!("text page") };
        let mut pre = vec![];
        html::find_all(&f.dom, "pre", &mut pre);
        assert_eq!(pre[0].children[0], Node::Text("a < b\nline2".into()));
        let Msg::Page { result: Err(e), .. } = &m[1] else { panic!("refused") };
        assert!(e.contains("application/pdf"));
    }

    #[test]
    fn unreachable_hosts_report_an_error() {
        let mut l = Loader::new(Fake::new(&[]));
        l.page(2, 3, "http://nowhere.test/");
        let m = wait(&mut l, 1);
        assert!(matches!(&m[0], Msg::Page { tab: 2, nav: 3, result: Err(e) } if e.contains("no such host")));
    }

    #[test]
    fn sheets_and_images_load_and_failures_are_empty() {
        let png = crate::cg::imageio::tests::png_bytes(4, 2, [255, 0, 0, 255]);
        let t = Fake::new(&[("c.test", "text/css", b"p { color: red }"), ("i.test", "image/png", &png)]);
        let mut l = Loader::new(t);
        l.sheet(1, 1, 3, "http://c.test/a.css");
        l.image(1, 1, "pic.png", "http://i.test/pic.png");
        l.image(1, 1, "gone.png", "http://nowhere.test/x.png");
        let m = wait(&mut l, 3);
        for msg in m {
            match msg {
                Msg::Sheet { slot, rules, .. } => assert_eq!((slot, rules.len()), (3, 1)),
                Msg::Image { src, image, .. } if src == "pic.png" => assert_eq!(image.map(|i| (i.w, i.h)), Some((4, 2))),
                Msg::Image { src, image, .. } => assert!(src == "gone.png" && image.is_none()),
                Msg::Page { .. } => panic!("no page was requested"),
            }
        }
    }

    #[test]
    fn cancel_drops_queued_work_for_a_tab() {
        let mut l = Loader::new(Fake::new(&[]));
        l.active.store(MAX_WORKERS, Ordering::SeqCst);
        l.image(1, 1, "a", "http://x.test/a");
        l.image(2, 1, "b", "http://x.test/b");
        l.cancel(1);
        assert_eq!(l.queue.len(), 1);
    }

    #[test]
    fn a_cancelled_connection_stops_reading_and_writing() {
        let t = Fake::new(&[("x.test", "text/plain", b"hello")]);
        let wrapped = Interruptible { inner: t };
        let mut s = wrapped.connect("x.test", 80, false).unwrap();
        let mut buf = [0u8; 8];
        let flag = Arc::new(AtomicBool::new(false));
        CANCEL.with(|c| *c.borrow_mut() = Some(flag.clone()));
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        assert!(s.read(&mut buf).unwrap() > 0, "a live request reads");
        flag.store(true, Ordering::SeqCst);
        assert!(s.read(&mut buf).is_err(), "a cancelled request stops reading");
        assert!(s.write_all(b"x").is_err(), "and stops writing");
        CANCEL.with(|c| *c.borrow_mut() = None);
    }
}
