//! Background `URLSession` sessions and download resume data (Foundation parity).
//!
//! `BackgroundConfiguration` mirrors `URLSessionConfiguration.background(withIdentifier:)`, `ResumeData` the
//! `NSURLSessionDownloadTaskResumeData` blob, and `BackgroundSession` a session whose download tasks are persisted
//! under a directory (the app support dir) so they can be restored by session identifier after the app quit. Work
//! goes through `URLSession::bytes`; a resumed task sends `Range` and `If-Range`, and a `200` instead of `206`
//! restarts from zero.

use crate::url::{HTTPURLResponse, URLError, URLRequest, URLSession};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// `NSURLSessionDownloadTaskResumeData`: the key of the resume blob in an error's user info.
pub const URL_SESSION_DOWNLOAD_TASK_RESUME_DATA: &str = "NSURLSessionDownloadTaskResumeData";

const RESUME_MAGIC: &str = "bifrost-resume-v1";

/// `URLSessionConfiguration.background(withIdentifier:)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundConfiguration {
    pub identifier: String,
    /// `isDiscretionary`: the system may defer the transfer to a good moment.
    pub is_discretionary: bool,
    /// `sessionSendsLaunchEvents`: relaunch the app when transfers finish.
    pub sessions_send_launch_events: bool,
}

impl BackgroundConfiguration {
    /// `background(withIdentifier:)`.
    pub fn background(identifier: &str) -> Self {
        BackgroundConfiguration { identifier: identifier.to_string(), is_discretionary: false, sessions_send_launch_events: true }
    }
}

/// What a partial download needs to continue: the URL, the validator and the bytes already on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeData {
    pub url: String,
    pub etag: Option<String>,
    pub bytes_received: u64,
}

impl ResumeData {
    /// The opaque blob (`Data`) handed to the app.
    pub fn encode(&self) -> Vec<u8> {
        format!("{RESUME_MAGIC}\n{}\n{}\n{}\n", escape(&self.url), escape(self.etag.as_deref().unwrap_or("")), self.bytes_received).into_bytes()
    }
    /// `None` when the blob is not resume data from `encode`.
    pub fn decode(data: &[u8]) -> Option<ResumeData> {
        let text = std::str::from_utf8(data).ok()?;
        let mut lines = text.split('\n');
        if lines.next()? != RESUME_MAGIC {
            return None;
        }
        let url = unescape(lines.next()?);
        let etag = unescape(lines.next()?);
        let bytes_received = lines.next()?.parse().ok()?;
        if url.is_empty() {
            return None;
        }
        Some(ResumeData { url, etag: (!etag.is_empty()).then_some(etag), bytes_received })
    }
}

/// `URLSessionDownloadDelegate`.
pub trait DownloadDelegate {
    /// `urlSession(_:downloadTask:didWriteData:totalBytesWritten:totalBytesExpectedToWrite:)`.
    fn did_write_data(&mut self, task: u64, written: u64, total_written: u64, total_expected: Option<u64>);
    /// `urlSession(_:downloadTask:didFinishDownloadingTo:)`.
    fn did_finish_downloading_to(&mut self, task: u64, location: &Path);
    /// `urlSession(_:task:didCompleteWithError:)`; `Ok` carries the final response.
    fn did_complete(&mut self, task: u64, result: Result<HTTPURLResponse, URLError>);
}

/// A persisted download task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: u64,
    pub url: String,
    pub destination: PathBuf,
    pub etag: Option<String>,
    pub bytes_received: u64,
}

struct State {
    tasks: BTreeMap<u64, TaskRecord>,
    cancels: BTreeMap<u64, Arc<AtomicBool>>,
    next_id: u64,
}

/// A background session: tasks are saved to `<dir>/<identifier>.tasks` after every change.
#[derive(Clone)]
pub struct BackgroundSession {
    pub configuration: BackgroundConfiguration,
    session: URLSession,
    dir: PathBuf,
    state: Arc<Mutex<State>>,
}

impl BackgroundSession {
    /// Opens (or restores) the session named by `configuration.identifier` under `dir`.
    pub fn open(configuration: BackgroundConfiguration, session: URLSession, dir: &Path) -> BackgroundSession {
        let mut tasks = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(table_path(dir, &configuration.identifier)) {
            for rec in text.lines().filter_map(parse_record) {
                tasks.insert(rec.id, rec);
            }
        }
        let next_id = tasks.keys().next_back().map_or(1, |i| i + 1);
        BackgroundSession { configuration, session, dir: dir.to_path_buf(), state: Arc::new(Mutex::new(State { tasks, cancels: BTreeMap::new(), next_id })) }
    }

    /// `getTasksWithCompletionHandler`: tasks not yet finished, by id.
    pub fn pending_tasks(&self) -> Vec<TaskRecord> {
        self.state.lock().unwrap().tasks.values().cloned().collect()
    }

    /// `downloadTask(with:)`: registers the task; `run` performs it.
    pub fn download_task(&self, url: &str, destination: &Path) -> u64 {
        self.add(TaskRecord { id: 0, url: url.to_string(), destination: destination.to_path_buf(), etag: None, bytes_received: 0 })
    }

    /// `downloadTask(withResumeData:)`. `None` when the blob does not decode.
    pub fn download_task_with_resume_data(&self, data: &[u8], destination: &Path) -> Option<u64> {
        let r = ResumeData::decode(data)?;
        Some(self.add(TaskRecord { id: 0, url: r.url, destination: destination.to_path_buf(), etag: r.etag, bytes_received: r.bytes_received }))
    }

    /// `cancel(byProducingResumeData:)`: stops a running task and returns its resume blob when anything was
    /// received. The partial file and the task are kept, so the blob can also be resumed with `run`.
    pub fn cancel_by_producing_resume_data(&self, id: u64) -> Option<Vec<u8>> {
        let st = self.state.lock().unwrap();
        st.cancels.get(&id)?.store(true, Ordering::Relaxed);
        let rec = st.tasks.get(&id)?;
        let on_disk = std::fs::metadata(part_path(&rec.destination)).map_or(0, |m| m.len());
        (on_disk > 0).then(|| ResumeData { url: rec.url.clone(), etag: rec.etag.clone(), bytes_received: on_disk }.encode())
    }

    /// `cancel()`: stops the task, drops it and removes its partial file.
    pub fn cancel(&self, id: u64) {
        let mut st = self.state.lock().unwrap();
        if let Some(flag) = st.cancels.remove(&id) {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(rec) = st.tasks.remove(&id) {
            let _ = std::fs::remove_file(part_path(&rec.destination));
        }
        self.save(&st);
    }

    /// Runs task `id` to the end on the calling thread (call from a background queue).
    pub fn run(&self, id: u64, delegate: &mut dyn DownloadDelegate) {
        let (rec, flag) = {
            let mut st = self.state.lock().unwrap();
            let Some(rec) = st.tasks.get(&id).cloned() else { return };
            let flag = Arc::new(AtomicBool::new(false));
            st.cancels.insert(id, flag.clone());
            (rec, flag)
        };
        let result = self.transfer(&rec, &flag, delegate);
        let mut st = self.state.lock().unwrap();
        st.cancels.remove(&id);
        match &result {
            Err(URLError::Cancelled) => {}
            _ => {
                st.tasks.remove(&id);
                if !matches!(&result, Ok(r) if (200..300).contains(&r.status_code)) {
                    let _ = std::fs::remove_file(part_path(&rec.destination));
                }
            }
        }
        self.save(&st);
        drop(st);
        delegate.did_complete(id, result);
    }

    fn transfer(&self, rec: &TaskRecord, cancel: &AtomicBool, delegate: &mut dyn DownloadDelegate) -> Result<HTTPURLResponse, URLError> {
        let part = part_path(&rec.destination);
        let on_disk = std::fs::metadata(&part).map_or(0, |m| m.len());
        let start = if rec.bytes_received > 0 && on_disk >= rec.bytes_received { rec.bytes_received } else { 0 };
        let mut req = URLRequest::new(&rec.url);
        if start > 0 {
            req.set_header("Range", &format!("bytes={start}-"));
            if let Some(tag) = &rec.etag {
                req.set_header("If-Range", tag);
            }
        }
        struct Run<'a> {
            ok: bool,
            offset: u64,
            expected: Option<u64>,
            file: Option<std::fs::File>,
            err: Option<String>,
            delegate: &'a mut dyn DownloadDelegate,
        }
        let run = RefCell::new(Run { ok: false, offset: start, expected: None, file: None, err: None, delegate });
        let id = rec.id;
        let result = self.session.bytes(
            &req,
            cancel,
            &mut |r| {
                let mut s = run.borrow_mut();
                s.ok = (200..300).contains(&r.status_code);
                let len = r.value("Content-Length").and_then(|v| v.parse::<u64>().ok());
                if r.status_code == 206 {
                    s.expected = len.map(|l| l + start);
                } else {
                    // 200 ignores the range: restart from zero.
                    s.offset = 0;
                    s.expected = len;
                }
                if s.ok {
                    let etag = r.value("ETag").map(str::to_string).or_else(|| rec.etag.clone());
                    let mut st = self.state.lock().unwrap();
                    if let Some(t) = st.tasks.get_mut(&id) {
                        t.etag = etag;
                        t.bytes_received = s.offset;
                    }
                    self.save(&st);
                }
            },
            &mut |chunk| {
                let mut s = run.borrow_mut();
                if !s.ok {
                    return true;
                }
                if s.file.is_none() {
                    let mut o = OpenOptions::new();
                    o.create(true);
                    if s.offset == 0 {
                        o.write(true).truncate(true);
                    } else {
                        o.append(true);
                    }
                    match o.open(&part) {
                        Ok(f) => s.file = Some(f),
                        Err(e) => {
                            s.err = Some(e.to_string());
                            return false;
                        }
                    }
                }
                if let Err(e) = s.file.as_mut().unwrap().write_all(chunk) {
                    s.err = Some(e.to_string());
                    return false;
                }
                s.offset += chunk.len() as u64;
                let (total, expected) = (s.offset, s.expected);
                s.delegate.did_write_data(id, chunk.len() as u64, total, expected);
                let mut st = self.state.lock().unwrap();
                if let Some(t) = st.tasks.get_mut(&id) {
                    t.bytes_received = total;
                }
                true
            },
        );
        let mut s = run.into_inner();
        drop(s.file.take());
        if let Some(e) = s.err {
            return Err(URLError::FileError(e));
        }
        if matches!(result, Err(URLError::Cancelled)) {
            let st = self.state.lock().unwrap();
            self.save(&st);
        }
        let resp = result?;
        if s.ok {
            std::fs::rename(&part, &rec.destination).map_err(|e| URLError::FileError(e.to_string()))?;
            s.delegate.did_finish_downloading_to(id, &rec.destination);
        }
        Ok(resp)
    }

    fn add(&self, mut rec: TaskRecord) -> u64 {
        let mut st = self.state.lock().unwrap();
        rec.id = st.next_id;
        st.next_id += 1;
        let id = rec.id;
        st.tasks.insert(id, rec);
        self.save(&st);
        id
    }

    fn save(&self, st: &State) {
        let text: String = st.tasks.values().map(format_record).collect();
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(table_path(&self.dir, &self.configuration.identifier), text);
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

fn table_path(dir: &Path, identifier: &str) -> PathBuf {
    let safe: String = identifier.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    dir.join(format!("{safe}.tasks"))
}

fn format_record(r: &TaskRecord) -> String {
    format!("{}\t{}\t{}\t{}\t{}\n", r.id, escape(&r.url), escape(&r.destination.to_string_lossy()), escape(r.etag.as_deref().unwrap_or("")), r.bytes_received)
}

fn parse_record(line: &str) -> Option<TaskRecord> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() != 5 {
        return None;
    }
    let etag = unescape(f[3]);
    Some(TaskRecord { id: f[0].parse().ok()?, url: unescape(f[1]), destination: PathBuf::from(unescape(f[2])), etag: (!etag.is_empty()).then_some(etag), bytes_received: f[4].parse().ok()? })
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::deildu::http::{Stream, Transport};
    use std::io::{self, Read};

    /// Serves one canned reply per connection and logs the request text.
    struct Fake {
        replies: Mutex<Vec<Vec<u8>>>,
        log: Arc<Mutex<Vec<String>>>,
    }
    struct Conn {
        reply: io::Cursor<Vec<u8>>,
        req: Vec<u8>,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl Read for Conn {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.reply.read(b)
        }
    }
    impl Write for Conn {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.req.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.req.ends_with(b"\r\n\r\n") {
                self.log.lock().unwrap().push(String::from_utf8_lossy(&self.req).to_ascii_lowercase());
                self.req.clear();
            }
            Ok(())
        }
    }
    impl Transport for Fake {
        fn connect(&self, _: &str, _: u16, _: bool) -> io::Result<Box<dyn Stream>> {
            let reply = self.replies.lock().unwrap().remove(0);
            Ok(Box::new(Conn { reply: io::Cursor::new(reply), req: Vec::new(), log: self.log.clone() }))
        }
    }

    fn session(replies: Vec<Vec<u8>>) -> (URLSession, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (URLSession::with_transport(Arc::new(Fake { replies: Mutex::new(replies), log: log.clone() }), None), log)
    }

    fn reply(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut v = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        v.extend_from_slice(body);
        v
    }

    #[derive(Default)]
    struct Rec {
        writes: Vec<(u64, u64, Option<u64>)>,
        finished: Option<PathBuf>,
        completed: Option<Result<u16, URLError>>,
    }
    impl DownloadDelegate for Rec {
        fn did_write_data(&mut self, _: u64, w: u64, t: u64, e: Option<u64>) {
            self.writes.push((w, t, e));
        }
        fn did_finish_downloading_to(&mut self, _: u64, p: &Path) {
            self.finished = Some(p.to_path_buf());
        }
        fn did_complete(&mut self, _: u64, r: Result<HTTPURLResponse, URLError>) {
            self.completed = Some(r.map(|x| x.status_code));
        }
    }

    #[test]
    fn configuration_round_trips_identifier_and_flags() {
        let mut c = BackgroundConfiguration::background("com.example.dl");
        c.is_discretionary = true;
        assert_eq!(c.identifier, "com.example.dl");
        assert!(c.is_discretionary && c.sessions_send_launch_events);
    }

    #[test]
    fn resume_data_encodes_and_decodes() {
        let r = ResumeData { url: "http://h/a b\nc".into(), etag: Some("\"v1\"".into()), bytes_received: 42 };
        assert_eq!(ResumeData::decode(&r.encode()), Some(r));
        let none = ResumeData { url: "http://h/x".into(), etag: None, bytes_received: 0 };
        assert_eq!(ResumeData::decode(&none.encode()), Some(none));
        assert_eq!(ResumeData::decode(b"junk"), None);
    }

    #[test]
    fn fresh_download_reports_progress_and_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let (s, log) = session(vec![reply("200 OK", "ETag: \"v1\"\r\n", b"hello")]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let dest = dir.path().join("out.bin");
        let id = bg.download_task("http://h/f", &dest);
        let mut d = Rec::default();
        bg.run(id, &mut d);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert_eq!(d.writes.last(), Some(&(5, 5, Some(5))));
        assert_eq!(d.finished, Some(dest));
        assert_eq!(d.completed, Some(Ok(200)));
        assert!(!log.lock().unwrap()[0].contains("range:"));
        assert!(bg.pending_tasks().is_empty());
    }

    #[test]
    fn resumed_task_sends_range_and_if_range_and_appends_on_206() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), b"hel").unwrap();
        let (s, log) = session(vec![reply("206 Partial Content", "", b"lo")]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let blob = ResumeData { url: "http://h/f".into(), etag: Some("\"v1\"".into()), bytes_received: 3 }.encode();
        let id = bg.download_task_with_resume_data(&blob, &dest).unwrap();
        let mut d = Rec::default();
        bg.run(id, &mut d);
        let req = log.lock().unwrap()[0].clone();
        assert!(req.contains("range: bytes=3-") && req.contains("if-range: \"v1\""));
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert_eq!(d.writes.last(), Some(&(2, 5, Some(5))));
    }

    #[test]
    fn status_200_instead_of_206_restarts_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), b"old").unwrap();
        let (s, _) = session(vec![reply("200 OK", "", b"fresh")]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let blob = ResumeData { url: "http://h/f".into(), etag: None, bytes_received: 3 }.encode();
        let id = bg.download_task_with_resume_data(&blob, &dest).unwrap();
        bg.run(id, &mut Rec::default());
        assert_eq!(std::fs::read(&dest).unwrap(), b"fresh");
    }

    #[test]
    fn pending_tasks_restore_by_session_identifier() {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) = session(vec![]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("one"), s.clone(), dir.path());
        let id = bg.download_task("http://h/f", &dir.path().join("o"));
        let again = BackgroundSession::open(BackgroundConfiguration::background("one"), s.clone(), dir.path());
        assert_eq!(again.pending_tasks().len(), 1);
        assert_eq!(again.pending_tasks()[0].id, id);
        let other = BackgroundSession::open(BackgroundConfiguration::background("two"), s, dir.path());
        assert!(other.pending_tasks().is_empty());
        assert!(again.download_task("http://h/g", &dir.path().join("p")) > id);
    }

    #[test]
    fn cancel_by_producing_resume_data_needs_received_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) = session(vec![]);
        let bg = BackgroundSession::open(BackgroundConfiguration::background("a"), s, dir.path());
        let dest = dir.path().join("o");
        let id = bg.download_task("http://h/f", &dest);
        bg.state.lock().unwrap().cancels.insert(id, Arc::new(AtomicBool::new(false)));
        assert_eq!(bg.cancel_by_producing_resume_data(id), None);
        std::fs::write(part_path(&dest), b"abcd").unwrap();
        let r = ResumeData::decode(&bg.cancel_by_producing_resume_data(id).unwrap()).unwrap();
        assert_eq!((r.url.as_str(), r.bytes_received), ("http://h/f", 4));
    }
}
