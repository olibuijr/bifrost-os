//! Live internet radio: an MP3, AAC (ADTS or LATM/LOAS, HE-AAC included),
//! Ogg or FLAC stream over http(s) as an `AudioDecoder`; `codec::sniff`
//! picks the decoder from the content type and the first bytes.
//!
//! A worker thread owns the connection. It sends `Icy-MetaData: 1`, strips
//! the metadata blocks out of the audio (their `StreamTitle` goes to the
//! `LiveHandle`), feeds the bytes into a bounded buffer that drops its
//! oldest data when full (a live stream does not wait), and reconnects with
//! a growing delay whenever the connection ends or fails. The audio thread
//! decodes from that buffer; the MP3 and AAC readers resync on the next
//! frame after a reconnect, so a drop is a short gap, not an end.
//!
//! The reader holds back the first bytes until `prebuffer` have arrived (and
//! again after every underrun), so the device is not fed in dribbles; the
//! `LiveHandle` says whether the stream is buffering, live or reconnecting.

use super::aacstream::AacStream;
use super::audiofile::AudioDecoder;
use super::codec::{self, Kind};
use super::mp3::Mp3;
use super::symfile::SymFile;
use super::Error;
use crate::deildu::http::Transport;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use symphonia_core::io::MediaSource;
use url::Url;

const USER_AGENT: &str = "SpringBoard/1.0 (radio)";
const MAX_REDIRECTS: usize = 5;
const HEADER_LIMIT: usize = 16 * 1024;
/// Longest chunk-size line we accept, extensions included.
const CHUNK_LINE_MAX: usize = 1024;
/// the newest this many bytes are kept (about 30 s of a 128 kbit/s stream)
const RING_MAX: usize = 512 * 1024;
const POLL: Duration = Duration::from_millis(50);
/// Bytes looked at to tell what codec a stream carries (room for a few AAC frames).
const SNIFF_BYTES: usize = 4096;
/// Bytes a decoder's probe may read looking for a frame before the stream is
/// declared not to be audio it knows (an HTML page would otherwise be scanned forever).
const PROBE_LIMIT: usize = 64 * 1024;

/// Timing knobs, so tests can run in milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// bytes to hold before playback starts or resumes
    pub prebuffer: usize,
    /// delay before the first reconnect, doubled up to `backoff_max_ms`
    pub backoff_first_ms: u64,
    pub backoff_max_ms: u64,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { prebuffer: 24 * 1024, backoff_first_ms: 1000, backoff_max_ms: 15_000 }
    }
}

/// Where the stream is on its way to the speaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Buffering,
    Live,
    /// the n-th attempt to get the connection back
    Reconnecting(u32),
}

/// What the UI shows about a live stream.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveStatus {
    pub phase: Phase,
    /// the stream's own "now playing" (ICY `StreamTitle`)
    pub title: Option<String>,
    /// why the last connection attempt failed, until data flows again
    pub error: Option<String>,
}

/// Shared status of one live stream; `notify` pokes the UI when it changes.
#[derive(Clone)]
pub struct LiveHandle {
    status: Arc<Mutex<LiveStatus>>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl LiveHandle {
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        LiveHandle { status: Arc::new(Mutex::new(LiveStatus { phase: Phase::Buffering, title: None, error: None })), notify }
    }

    pub fn status(&self) -> LiveStatus {
        self.status.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn update(&self, f: impl FnOnce(&mut LiveStatus)) {
        let changed = {
            let mut s = self.status.lock().unwrap_or_else(|p| p.into_inner());
            let before = s.clone();
            f(&mut s);
            *s != before
        };
        if changed {
            (self.notify)();
        }
    }
}

/// The audio bytes between the worker and the decoder.
struct Ring {
    inner: Mutex<RingState>,
    ready: Condvar,
    /// the `Content-Type` of the latest connection
    content_type: Mutex<Option<String>>,
}

struct RingState {
    data: VecDeque<u8>,
    stop: bool,
}

impl Ring {
    fn new() -> Arc<Self> {
        Arc::new(Ring { inner: Mutex::new(RingState { data: VecDeque::new(), stop: false }), ready: Condvar::new(), content_type: Mutex::new(None) })
    }

    /// The first `SNIFF_BYTES` of the stream, left in the ring, once they
    /// have arrived; None when `abort` says to give up first.
    fn peek_head(&self, abort: &dyn Fn() -> bool) -> Option<Vec<u8>> {
        let mut s = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if s.data.len() >= SNIFF_BYTES {
                return Some(s.data.iter().take(SNIFF_BYTES).copied().collect());
            }
            if abort() || s.stop {
                return None;
            }
            s = self.ready.wait_timeout(s, POLL).unwrap().0;
        }
    }

    fn push(&self, bytes: &[u8]) {
        let mut s = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        s.data.extend(bytes);
        let excess = s.data.len().saturating_sub(RING_MAX);
        s.data.drain(..excess);
        self.ready.notify_all();
    }

    fn stop(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).stop = true;
        self.ready.notify_all();
    }

    fn stopped(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).stop
    }
}

/// The decoder's end of the ring, as a non-seekable media source. It ends
/// (reads 0) only when `abort` says so.
struct LiveReader {
    ring: Arc<Ring>,
    live: LiveHandle,
    abort: Box<dyn Fn() -> bool + Send + Sync>,
    prebuffer: usize,
    /// waiting for `prebuffer` bytes before handing out any
    rebuffering: bool,
    /// set while the decoder is still looking for the first frame
    probing: Arc<AtomicBool>,
    served: usize,
}

impl Read for LiveReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut s = self.ring.inner.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if self.probing.load(Ordering::SeqCst) && self.served > PROBE_LIMIT {
                return Ok(0);
            }
            if !s.data.is_empty() && (!self.rebuffering || s.data.len() >= self.prebuffer) {
                self.rebuffering = false;
                let n = out.len().min(s.data.len());
                for (o, b) in out.iter_mut().zip(s.data.drain(..n)) {
                    *o = b;
                }
                drop(s);
                self.served += n;
                self.live.update(|st| {
                    if st.phase == Phase::Buffering {
                        st.phase = Phase::Live;
                    }
                });
                return Ok(n);
            }
            if s.data.is_empty() && !self.rebuffering {
                self.rebuffering = true;
                self.live.update(|st| {
                    if st.phase == Phase::Live {
                        st.phase = Phase::Buffering;
                    }
                });
            }
            if (self.abort)() {
                return Ok(0);
            }
            s = self.ring.ready.wait_timeout(s, POLL).unwrap().0;
        }
    }
}

impl Seek for LiveReader {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "a live stream cannot seek"))
    }
}

impl MediaSource for LiveReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl Drop for LiveReader {
    fn drop(&mut self) {
        self.ring.stop();
    }
}

/// Opens `url` as a live stream. Returns once the first frames decode, so
/// it blocks while connecting; `abort` ends the wait (and the stream).
pub fn open(url: &str, transport: Arc<dyn Transport>, live: LiveHandle, abort: Box<dyn Fn() -> bool + Send + Sync>, tuning: Tuning) -> Result<Box<dyn AudioDecoder>, Error> {
    let ring = Ring::new();
    {
        let (url, ring, live) = (url.to_string(), ring.clone(), live.clone());
        std::thread::Builder::new().name("radio-net".into()).spawn(move || pump(&url, &*transport, &ring, &live, tuning))?;
    }
    // until the reader owns the ring (and stops it on drop), an early return must stop the pump
    let mut guard = StopRing(Some(ring.clone()));
    let head = ring.peek_head(&*abort).ok_or(Error::Invalid("radio stream (no audio)"))?;
    let content_type = ring.content_type.lock().unwrap_or_else(|p| p.into_inner()).clone();
    // nothing recognisable: try MP3, whose probe then rejects the stream
    let kind = codec::sniff(content_type.as_deref(), &head).unwrap_or(Kind::Mp3);
    let probing = Arc::new(AtomicBool::new(true));
    let reader = Box::new(LiveReader { ring, live, abort, prebuffer: tuning.prebuffer, rebuffering: true, probing: probing.clone(), served: 0 });
    let unsupported = |_| Error::Invalid("radio stream (unsupported audio)");
    let decoder: Box<dyn AudioDecoder> = match kind {
        Kind::Mp3 => Box::new(Mp3::from_source(reader).map_err(unsupported)?.without_length()),
        Kind::Adts | Kind::Latm => Box::new(AacStream::open(reader, kind.aac_framing().expect("an AAC kind"), false).map_err(unsupported)?),
        Kind::Ogg | Kind::Flac => Box::new(SymFile::from_source(kind, reader).map_err(unsupported)?.without_length()),
        Kind::M4a | Kind::Wav => return Err(Error::Invalid("radio stream (not a live format)")),
    };
    probing.store(false, Ordering::SeqCst);
    guard.0 = None;
    Ok(decoder)
}

/// Stops the ring's pump thread when dropped, unless disarmed by taking the ring out.
struct StopRing(Option<Arc<Ring>>);

impl Drop for StopRing {
    fn drop(&mut self) {
        if let Some(ring) = &self.0 {
            ring.stop();
        }
    }
}

/// The worker: connect, copy audio into the ring, reconnect on any end.
fn pump(url: &str, transport: &dyn Transport, ring: &Ring, live: &LiveHandle, tuning: Tuning) {
    let titles = {
        let live = live.clone();
        move |title: Option<String>| live.update(|st| st.title = title)
    };
    let (mut attempt, mut delay) = (0u32, tuning.backoff_first_ms);
    let mut buf = [0u8; 4096];
    while !ring.stopped() {
        let why = match connect_typed(transport, url, &titles) {
            Ok((mut body, content_type)) => {
                *ring.content_type.lock().unwrap_or_else(|p| p.into_inner()) = content_type;
                loop {
                    if ring.stopped() {
                        return;
                    }
                    match body.read(&mut buf) {
                        Ok(0) => break io::Error::other("the station closed the stream"),
                        Ok(n) => {
                            if attempt > 0 {
                                attempt = 0;
                                delay = tuning.backoff_first_ms;
                                live.update(|st| {
                                    st.phase = Phase::Buffering;
                                    st.error = None;
                                });
                            }
                            ring.push(&buf[..n]);
                        }
                        Err(e) => break e,
                    }
                }
            }
            Err(e) => e,
        };
        attempt += 1;
        live.update(|st| {
            st.phase = Phase::Reconnecting(attempt);
            st.error = Some(why.to_string());
        });
        let mut waited = 0;
        while waited < delay && !ring.stopped() {
            std::thread::sleep(Duration::from_millis(10));
            waited += 10;
        }
        delay = (delay * 2).min(tuning.backoff_max_ms);
    }
}

/// `connect_typed` without the content type.
#[cfg(test)]
fn connect(transport: &dyn Transport, url: &str, on_title: &(impl Fn(Option<String>) + Clone + Send + 'static)) -> io::Result<Box<dyn Read + Send>> {
    connect_typed(transport, url, on_title).map(|(body, _)| body)
}

/// One connection: request, redirects, then the audio with the ICY and
/// chunked framing removed; also returns the response's `Content-Type`.
fn connect_typed(transport: &dyn Transport, url: &str, on_title: &(impl Fn(Option<String>) + Clone + Send + 'static)) -> io::Result<(Box<dyn Read + Send>, Option<String>)> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let u = Url::parse(&url).map_err(|_| io::Error::other("bad stream url"))?;
        let host = u.host_str().ok_or_else(|| io::Error::other("stream url has no host"))?;
        let tls = u.scheme() == "https";
        let mut conn = transport.connect(host, u.port_or_known_default().unwrap_or(80), tls)?;
        conn.write_all(request(&u, host).as_bytes())?;
        let mut reader = BufReader::new(conn);
        let (status, headers) = read_head(&mut reader)?;
        let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
        match (status, header("location")) {
            (301 | 302 | 303 | 307 | 308, Some(loc)) => {
                url = u.join(loc).map_err(|_| io::Error::other("bad redirect"))?.to_string();
                continue;
            }
            (200, _) => {}
            _ => return Err(io::Error::other(format!("the station answered {status}"))),
        }
        let metaint = header("icy-metaint").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
        let body: Box<dyn Read + Send> = if header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) { Box::new(Chunked { r: reader, left: 0 }) } else { Box::new(reader) };
        let content_type = header("content-type").map(str::to_string);
        return Ok((Box::new(IcyReader::new(body, metaint, on_title.clone())), content_type));
    }
    Err(io::Error::other("too many redirects"))
}

fn request(u: &Url, host: &str) -> String {
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    let query = u.query().map(|q| format!("?{q}")).unwrap_or_default();
    format!("GET {}{query} HTTP/1.0\r\nHost: {host}{port}\r\nUser-Agent: {USER_AGENT}\r\nAccept: */*\r\nIcy-MetaData: 1\r\nConnection: close\r\n\r\n", u.path())
}

/// Reads one line, but never buffers more than `max` bytes of it: a line that
/// runs past `max` without a newline is an error, so an endless line from a
/// hostile or broken server cannot grow the buffer without limit.
fn read_capped_line(r: &mut impl BufRead, line: &mut String, max: usize) -> io::Result<usize> {
    let n = r.by_ref().take(max as u64 + 1).read_line(line)?;
    if line.len() > max && !line.ends_with('\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(n)
}

/// The status and headers of a response; Shoutcast says `ICY 200 OK`.
fn read_head(r: &mut impl BufRead) -> io::Result<(u16, Vec<(String, String)>)> {
    let mut line = String::new();
    read_capped_line(r, &mut line, HEADER_LIMIT)?;
    let status = line.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()).filter(|_| line.starts_with("HTTP/") || line.starts_with("ICY")).ok_or_else(|| io::Error::other("not an http response"))?;
    let (mut headers, mut total) = (Vec::new(), line.len());
    loop {
        line.clear();
        if read_capped_line(r, &mut line, HEADER_LIMIT.saturating_sub(total))? == 0 {
            return Err(io::Error::other("the station hung up in the headers"));
        }
        total += line.len();
        if total > HEADER_LIMIT {
            return Err(io::Error::other("headers too long"));
        }
        let line = line.trim_end();
        if line.is_empty() {
            return Ok((status, headers));
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
}

/// HTTP/1.1 chunked transfer coding.
struct Chunked<R> {
    r: R,
    left: usize,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            let mut line = String::new();
            read_capped_line(&mut self.r, &mut line, CHUNK_LINE_MAX)?;
            let size = line.trim().split(';').next().unwrap_or("");
            self.left = usize::from_str_radix(size, 16).map_err(|_| io::Error::other("bad chunk size"))?;
            if self.left == 0 {
                return Ok(0);
            }
        }
        let want = out.len().min(self.left);
        let n = self.r.read(&mut out[..want])?;
        self.left -= n;
        if self.left == 0 {
            let mut crlf = [0u8; 2];
            self.r.read_exact(&mut crlf)?;
        }
        Ok(n)
    }
}

/// Removes the ICY metadata blocks from a stream: after every `metaint`
/// audio bytes comes a length byte (in 16-byte units) and that much text.
pub struct IcyReader<R, F> {
    inner: R,
    metaint: usize,
    /// audio bytes until the next metadata block
    until: usize,
    last: Option<String>,
    on_title: F,
}

impl<R: Read, F: Fn(Option<String>)> IcyReader<R, F> {
    /// `metaint` 0 means the stream carries no metadata.
    pub fn new(inner: R, metaint: usize, on_title: F) -> Self {
        IcyReader { inner, metaint, until: metaint, last: None, on_title }
    }

    fn metadata(&mut self) -> io::Result<()> {
        let mut len = [0u8; 1];
        self.inner.read_exact(&mut len)?;
        if len[0] > 0 {
            let mut block = vec![0u8; len[0] as usize * 16];
            self.inner.read_exact(&mut block)?;
            let title = stream_title(&block);
            if title.is_some() && title != self.last {
                self.last = title.clone();
                (self.on_title)(title);
            }
        }
        self.until = self.metaint;
        Ok(())
    }
}

impl<R: Read, F: Fn(Option<String>)> Read for IcyReader<R, F> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.metaint == 0 {
            return self.inner.read(out);
        }
        if self.until == 0 {
            self.metadata()?;
        }
        let want = out.len().min(self.until);
        let n = self.inner.read(&mut out[..want])?;
        self.until -= n;
        Ok(n)
    }
}

/// The `StreamTitle` of a metadata block (UTF-8, else Latin-1), if it has one.
pub fn stream_title(block: &[u8]) -> Option<String> {
    let text: String = match std::str::from_utf8(block) {
        Ok(s) => s.to_string(),
        Err(_) => block.iter().map(|&b| b as char).collect(),
    };
    let text = text.trim_end_matches('\0');
    let rest = &text[text.find("StreamTitle='")? + "StreamTitle='".len()..];
    let title = rest.find("';").map_or(rest, |end| &rest[..end]).trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// A local Icecast look-alike and a transport that dials it, for tests.
#[cfg(test)]
pub(crate) mod fake {
    use super::super::audiofile::tests::fixture;
    use super::super::id3;
    use super::*;
    use crate::deildu::http::Stream;
    use std::net::{TcpListener, TcpStream};

    /// Plain TCP to local ports: the port registered for the URL's host,
    /// else the first one.
    pub struct Local(pub Vec<(&'static str, u16)>);

    impl Local {
        pub fn on(port: u16) -> Self {
            Local(vec![("", port)])
        }
    }

    impl Transport for Local {
        fn connect(&self, host: &str, _port: u16, _tls: bool) -> io::Result<Box<dyn Stream>> {
            let port = self.0.iter().find(|(h, _)| *h == host).unwrap_or(&self.0[0]).1;
            Ok(Box::new(TcpStream::connect(("127.0.0.1", port))?))
        }
    }

    /// An Icecast look-alike. Every connection gets `head`, then `audio`
    /// over and over, with a metadata block after every `metaint` bytes
    /// (when non-zero) carrying the next of `titles`. The first connection
    /// is cut after `cut_first` body bytes. Returns the port and the
    /// requests received.
    pub fn icecast(head: &'static str, audio: Vec<u8>, metaint: usize, titles: &'static [&'static str], cut_first: Option<usize>) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for (n, conn) in listener.incoming().enumerate() {
                let Ok(mut conn) = conn else { return };
                let (audio, seen) = (audio.clone(), seen.clone());
                std::thread::spawn(move || {
                    let mut request = String::new();
                    let mut r = BufReader::new(conn.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        request.push_str(&line);
                    }
                    seen.lock().unwrap_or_else(|p| p.into_inner()).push(request);
                    let head = head.replace("{metaint}", &metaint.to_string());
                    if conn.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    let (mut sent, mut since_meta, mut block) = (0usize, 0usize, 0usize);
                    for chunk in audio.chunks(500).cycle() {
                        let mut out = Vec::new();
                        for &b in chunk {
                            if metaint > 0 && since_meta == metaint {
                                let text = if titles.is_empty() { String::new() } else { format!("StreamTitle='{}';", titles[block % titles.len()]) };
                                block += 1;
                                let units = text.len().div_ceil(16);
                                out.push(units as u8);
                                out.extend(text.as_bytes());
                                out.resize(out.len() + units * 16 - text.len(), 0);
                                since_meta = 0;
                            }
                            out.push(b);
                            since_meta += 1;
                        }
                        sent += chunk.len();
                        if conn.write_all(&out).is_err() || (n == 0 && cut_first.is_some_and(|c| sent >= c)) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                });
            }
        });
        (port, requests)
    }

    /// A Shoutcast response head; `{metaint}` is filled in per server.
    pub const ICY: &str = "ICY 200 OK\r\nicy-name: Test\r\nicy-metaint: {metaint}\r\ncontent-type: audio/mpeg\r\n\r\n";

    /// The fixture's MP3 frames as a radio sends them: no ID3 tag and no
    /// Xing/LAME header frame (whose frame count would make the decoder
    /// trim everything after the first pass).
    pub fn mp3_frames() -> Vec<u8> {
        const KBPS: [usize; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
        const HZ: [usize; 3] = [44100, 48000, 32000];
        let all = std::fs::read(fixture("tone_id3v23.mp3")).unwrap();
        let frames = &all[id3::tag_len(&all[..10])..];
        let h = &frames[..4];
        assert!(h[0] == 0xFF && h[1] & 0xFE == 0xFA, "MPEG-1 Layer III");
        let len = 144 * KBPS[(h[2] >> 4) as usize] * 1000 / HZ[((h[2] >> 2) & 3) as usize] + ((h[2] >> 1) & 1) as usize;
        assert!(frames[4..len].windows(4).any(|w| w == b"Xing" || w == b"Info"), "the first frame is the info frame");
        frames[len..].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::super::audiofile::tests::power;
    use super::fake::{icecast, Local, ICY};
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    fn handle() -> (LiveHandle, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        (LiveHandle::new(Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        })), wakes)
    }

    const FAST: Tuning = Tuning { prebuffer: 4096, backoff_first_ms: 10, backoff_max_ms: 40 };

    fn collect(d: &mut dyn AudioDecoder, frames: usize) -> Vec<i16> {
        let (mut all, mut chunk) = (Vec::new(), Vec::new());
        let start = Instant::now();
        while all.len() / 2 < frames && start.elapsed().as_secs() < 10 {
            assert!(d.next(&mut chunk).unwrap(), "a live stream does not end");
            all.extend_from_slice(&chunk);
        }
        all
    }

    #[test]
    fn titles_come_from_the_metadata_and_audio_comes_out_clean() {
        let mut stream = Vec::new();
        let mut expect = Vec::new();
        for (i, title) in ["Artist - One", "Artist - Two"].iter().enumerate() {
            stream.extend((0..8).map(|b| (b + i * 8) as u8));
            let text = format!("StreamTitle='{title}';StreamUrl='';");
            let units = text.len().div_ceil(16);
            stream.push(units as u8);
            stream.extend(text.as_bytes());
            stream.resize(stream.len() + units * 16 - text.len(), 0);
            expect.extend((0..8).map(|b| (b + i * 8) as u8));
        }
        stream.extend([16, 17, 18, 19, 20, 21, 22, 23, 0]); // an empty block ends the data
        expect.extend([16, 17, 18, 19, 20, 21, 22, 23]);
        let titles = Mutex::new(Vec::new());
        let mut r = IcyReader::new(Cursor::new(stream), 8, |t| titles.lock().unwrap_or_else(|p| p.into_inner()).push(t));
        let mut audio = Vec::new();
        r.read_to_end(&mut audio).unwrap();
        assert_eq!(audio, expect);
        assert_eq!(*titles.lock().unwrap_or_else(|p| p.into_inner()), [Some("Artist - One".to_string()), Some("Artist - Two".to_string())]);
    }

    #[test]
    fn without_metaint_the_bytes_pass_through() {
        let mut r = IcyReader::new(Cursor::new(vec![1, 2, 3, 4, 5]), 0, |_| panic!("no titles"));
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn stream_titles_parse_in_utf8_and_latin1() {
        assert_eq!(stream_title(b"StreamTitle='A - B';StreamUrl='x';\0\0"), Some("A - B".into()));
        assert_eq!(stream_title(b"StreamTitle='It's fine';"), Some("It's fine".into()));
        assert_eq!(stream_title("StreamTitle='Sigur R\u{f3}s';".as_bytes()), Some("Sigur R\u{f3}s".into()));
        assert_eq!(stream_title(b"StreamTitle='Bj\xf6rk - Hunter';"), Some("Bj\u{f6}rk - Hunter".into()), "Latin-1 fallback");
        assert_eq!(stream_title(b"StreamTitle='';"), None);
        assert_eq!(stream_title(b"StreamUrl='x';"), None);
    }

    #[test]
    fn follows_a_redirect_to_the_real_server() {
        let (port, requests) = icecast("HTTP/1.0 200 OK\r\n\r\n", vec![7; 100], 0, &[], None);
        let hop = TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_port = hop.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = hop.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = c.read(&mut buf);
            let _ = write!(c, "HTTP/1.1 302 Found\r\nLocation: http://radio.test/live?x=1\r\n\r\n");
        });
        let mut body = connect(&Local(vec![("hop.test", hop_port), ("radio.test", port)]), "http://hop.test/start", &|_| {}).unwrap();
        let mut audio = [0u8; 50];
        body.read_exact(&mut audio).unwrap();
        assert_eq!(audio, [7; 50]);
        assert!(requests.lock().unwrap_or_else(|p| p.into_inner())[0].starts_with("GET /live?x=1 HTTP/1.0"));
    }

    #[test]
    fn sends_the_icy_header_and_reads_chunked_bodies() {
        let (port, requests) = icecast("HTTP/1.1 200 OK\r\ncontent-type: audio/mpeg\r\ntransfer-encoding: chunked\r\n\r\n", Vec::new(), 0, &[], None);
        let mut body = connect(&Local::on(port), "http://radio.test/stream?k=v", &|_| {}).unwrap();
        let mut sink = [0u8; 8];
        let _ = body.read(&mut sink);
        let request = requests.lock().unwrap_or_else(|p| p.into_inner())[0].clone();
        assert!(request.starts_with("GET /stream?k=v HTTP/1.0\r\n"), "{request}");
        assert!(request.contains("Icy-MetaData: 1\r\n") && request.contains("Host: radio.test\r\n"), "{request}");
        let mut c = Chunked { r: Cursor::new(b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n".to_vec()), left: 0 };
        let mut text = String::new();
        c.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn rejects_error_statuses_and_garbage() {
        for head in ["HTTP/1.0 404 Not Found\r\n\r\n", "garbage\r\n\r\n"] {
            let (port, _) = icecast(head, vec![0; 10], 0, &[], None);
            assert!(connect(&Local::on(port), "http://radio.test/x", &|_| {}).is_err(), "{head}");
        }
    }

    #[test]
    fn a_header_line_without_a_newline_is_cut_off_early() {
        // A station that never sends a newline must not make the pump buffer the whole stream.
        let mut data = b"HTTP/1.1 200 OK\r\nX: ".to_vec();
        data.resize(1 << 20, b'a');
        let mut r = std::io::BufReader::new(Cursor::new(data));
        let err = read_head(&mut r).err().expect("an endless header line must fail");
        assert!(err.to_string().contains("too long"), "{err}");
        assert!(r.get_ref().position() < 64 * 1024, "read {} bytes", r.get_ref().position());
    }

    #[test]
    fn a_chunk_size_line_without_a_newline_is_cut_off_early() {
        let mut c = Chunked { r: std::io::BufReader::new(Cursor::new(vec![b'f'; 1 << 20])), left: 0 };
        let err = c.read(&mut [0u8; 16]).err().expect("an endless chunk-size line must fail");
        assert!(err.to_string().contains("too long"), "{err}");
        assert!(c.r.get_ref().position() < 64 * 1024, "read {} bytes", c.r.get_ref().position());
    }

    #[test]
    fn plays_a_shoutcast_stream_with_titles_through_the_mp3_decoder() {
        let (port, _) = icecast(ICY, fake::mp3_frames(), 1000, &["Artist - Song"], None);
        let (live, wakes) = handle();
        let mut d = open("http://radio.test/live", Arc::new(Local::on(port)), live.clone(), Box::new(|| false), FAST).unwrap();
        assert_eq!((d.rate(), d.channels()), (44100, 2));
        assert_eq!(d.duration_ms(), None, "a live stream has no length");
        let pcm = collect(&mut *d, 44100 * 3);
        assert!(pcm.len() / 2 >= 44100 * 3, "{} frames", pcm.len() / 2);
        assert!(power(&pcm[pcm.len() / 4..], 2, 44100, 440.0) > 20.0 * power(&pcm[pcm.len() / 4..], 2, 44100, 1000.0).max(1e-9), "the tone survives the metadata stripping");
        let st = live.status();
        assert_eq!((st.phase, st.title.as_deref(), st.error), (Phase::Live, Some("Artist - Song"), None));
        assert!(wakes.load(Ordering::SeqCst) >= 2, "the UI is woken as the status changes");
        assert!(d.seek_ms(0.0).is_err());
    }

    #[test]
    fn reconnects_when_the_connection_drops_and_keeps_decoding() {
        let (port, requests) = icecast(ICY, fake::mp3_frames(), 0, &[], Some(6000));
        let (live, _) = handle();
        let mut d = open("http://radio.test/live", Arc::new(Local::on(port)), live.clone(), Box::new(|| false), FAST).unwrap();
        let pcm = collect(&mut *d, 44100 * 2);
        assert!(pcm.len() / 2 >= 44100 * 2, "decoding went on past the dropped connection");
        assert!(requests.lock().unwrap_or_else(|p| p.into_inner()).len() >= 2, "it dialled again");
        assert_eq!(live.status().phase, Phase::Live);
        assert_eq!(live.status().error, None);
    }

    struct Counting(Arc<std::sync::atomic::AtomicUsize>, u16);

    impl Transport for Counting {
        fn connect(&self, _host: &str, _port: u16, _tls: bool) -> io::Result<Box<dyn crate::deildu::http::Stream>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(std::net::TcpStream::connect(("127.0.0.1", self.1))?))
        }
    }

    #[test]
    fn an_aborted_open_stops_the_reconnecting_pump() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (live, _) = handle();
        let connects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let abort = Arc::new(AtomicBool::new(false));
        let a = abort.clone();
        let c = connects.clone();
        let opener = std::thread::spawn(move || open("http://radio.test/live", Arc::new(Counting(c, closed)), live, Box::new(move || a.load(Ordering::SeqCst)), FAST).is_err());
        while connects.load(Ordering::SeqCst) < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        abort.store(true, Ordering::SeqCst);
        assert!(opener.join().unwrap());
        std::thread::sleep(Duration::from_millis(150));
        let seen = connects.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(connects.load(Ordering::SeqCst), seen, "the pump kept reconnecting after open gave up");
    }

    #[test]
    fn a_dead_station_is_retried_with_backoff_and_the_error_is_shown() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (live, _) = handle();
        let abort = Arc::new(AtomicBool::new(false));
        let a = abort.clone();
        let l = live.clone();
        let opener = std::thread::spawn(move || open("http://radio.test/live", Arc::new(Local::on(closed)), l, Box::new(move || a.load(Ordering::SeqCst)), FAST).is_err());
        let start = Instant::now();
        while !matches!(live.status().phase, Phase::Reconnecting(n) if n >= 3) && start.elapsed().as_secs() < 5 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let st = live.status();
        assert!(matches!(st.phase, Phase::Reconnecting(n) if n >= 3), "{st:?}");
        assert!(st.error.is_some());
        abort.store(true, Ordering::SeqCst);
        assert!(opener.join().unwrap(), "aborting ends the wait; there was never any audio");
    }

    #[test]
    fn a_stream_that_is_not_mp3_fails_to_open() {
        let (port, _) = icecast("HTTP/1.0 200 OK\r\n\r\n", b"<html>not audio, just a long enough page of text to probe</html>".repeat(200), 0, &[], None);
        let (live, _) = handle();
        let seen = Arc::new(AtomicBool::new(false));
        let s = seen.clone();
        let abort = Box::new(move || s.load(Ordering::SeqCst));
        let start = Instant::now();
        let r = open("http://radio.test/live", Arc::new(Local::on(port)), live, abort, FAST);
        assert!(r.is_err(), "{:?}", r.is_ok());
        assert!(start.elapsed().as_secs() < 5);
    }

    #[test]
    fn plays_adts_and_loas_aac_radio_whatever_the_label() {
        use super::super::aacstream::tests::{adts, lc_bits, loas, raw_frames};
        let (_, frames) = raw_frames();
        let latm: Vec<u8> = frames.iter().flat_map(|f| loas(f, Some(&lc_bits))).collect();
        let cases: [(&'static str, Vec<u8>); 3] = [
            ("HTTP/1.0 200 OK\r\ncontent-type: audio/aacp\r\n\r\n", adts(&frames)),
            ("ICY 200 OK\r\ncontent-type: audio/aacp\r\n\r\n", latm.clone()),
            ("HTTP/1.0 200 OK\r\ncontent-type: audio/mpeg\r\n\r\n", adts(&frames)),
        ];
        for (head, audio) in cases {
            let (port, _) = icecast(head, audio, 0, &[], None);
            let (live, _) = handle();
            let mut d = open("http://radio.test/live", Arc::new(Local::on(port)), live, Box::new(|| false), FAST).unwrap();
            assert_eq!((d.rate(), d.channels(), d.duration_ms()), (44100, 2, None), "{head}");
            let pcm = collect(&mut *d, 44100);
            assert!(power(&pcm[pcm.len() / 2..], 2, 44100, 440.0) > 20.0 * power(&pcm[pcm.len() / 2..], 2, 44100, 1000.0).max(1e-9), "{head}");
        }
    }

    /// Network: one real station, over the real HTTP(S) client. Run with
    /// `cargo test --lib real_station -- --ignored`.
    #[test]
    #[ignore]
    fn real_station_streams_mp3_with_icy_metadata() {
        let titles = Arc::new(Mutex::new(Vec::new()));
        let sink = titles.clone();
        let mut body = connect(&crate::deildu::http::Tls::new(), "http://ice-11.spilarinn.is/truv2", &move |t| sink.lock().unwrap_or_else(|p| p.into_inner()).push(t)).unwrap();
        let mut audio = vec![0u8; 40_000];
        body.read_exact(&mut audio).unwrap();
        assert!(audio.windows(2).any(|w| w[0] == 0xFF && w[1] & 0xE0 == 0xE0), "MP3 frame sync in the audio");
        assert!(!audio.windows(11).any(|w| w == b"StreamTitle"), "metadata blocks were stripped");
        eprintln!("titles: {:?}", titles.lock().unwrap_or_else(|p| p.into_inner()));
    }
}
