//! Contract tests for the URL loading completion: cookies, authentication challenges, the disk cache, WebSocket and
//! stream tasks, task metrics and background sessions, all against loopback servers in this file.

use bifrost_app::http_auth::{AuthChallenge, AuthDisposition, CredentialPersistence, URLCredential};
use bifrost_app::net_service::{NetService, NetServiceBrowser};
use bifrost_app::url::{URLRequest, URLSession};
use bifrost_app::url_background::{ConfiguredSession, DownloadDelegate, FetchType, URLSessionConfiguration, URLSessionTaskDelegate};
use bifrost_app::url_cache::{CachePolicy, URLCache};
use bifrost_app::url_stream_task::URLSessionStreamTask;
use bifrost_app::websocket::{accept_key, decode, encode, frame_for, Frame, Message, Opcode, URLSessionWebSocketTask};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

static NETWORK_HITS: AtomicUsize = AtomicUsize::new(0);

fn read_head(s: &mut TcpStream) -> Option<String> {
    let mut req = Vec::new();
    let mut b = [0u8; 1];
    while !req.ends_with(b"\r\n\r\n") {
        if s.read(&mut b).ok()? == 0 {
            return None;
        }
        req.push(b[0]);
    }
    Some(String::from_utf8_lossy(&req).into_owned())
}

fn http_server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            std::thread::spawn(move || {
                let Some(text) = read_head(&mut s) else { return };
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let reply: Vec<u8> = match path.as_str() {
                    "/login" => b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc; Path=/\r\nContent-Length: 2\r\n\r\nok".to_vec(),
                    "/whoami" => {
                        let body = if text.contains("Cookie: sid=abc") { "in" } else { "out" };
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
                    }
                    "/secret" => {
                        // base64("ann:pw") = YW5uOnB3
                        if text.contains("Authorization: Basic YW5uOnB3") {
                            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecret".to_vec()
                        } else {
                            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"lab\"\r\nContent-Length: 0\r\n\r\n".to_vec()
                        }
                    }
                    "/etag" => {
                        NETWORK_HITS.fetch_add(1, Ordering::SeqCst);
                        if text.contains("If-None-Match: \"v1\"") {
                            b"HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nCache-Control: max-age=60\r\n\r\n".to_vec()
                        } else {
                            b"HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nCache-Control: max-age=60\r\nContent-Length: 4\r\n\r\netag".to_vec()
                        }
                    }
                    "/file" => b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
                    _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
                };
                let _ = s.write_all(&reply);
            });
        }
    });
    port
}

fn session(config: URLSessionConfiguration, delegate: Option<Arc<dyn URLSessionTaskDelegate>>) -> ConfiguredSession {
    ConfiguredSession::new(config, URLSession::shared(), delegate)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("bifrost-url-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn cookies_set_by_one_reply_are_sent_on_the_next_request_unless_the_policy_refuses() {
    let port = http_server();
    let s = session(URLSessionConfiguration::ephemeral(), None);
    let base = format!("http://127.0.0.1:{port}");
    s.data_at(&URLRequest::new(&format!("{base}/login")), 100).unwrap();
    let (_, body) = s.data_at(&URLRequest::new(&format!("{base}/whoami")), 101).unwrap();
    assert_eq!(body, b"in");

    let refusing = session(URLSessionConfiguration::ephemeral(), None);
    refusing.configuration.http_cookie_storage.as_ref().unwrap().lock().unwrap().cookie_accept_policy = bifrost_app::http_cookies::AcceptPolicy::Never;
    refusing.data_at(&URLRequest::new(&format!("{base}/login")), 100).unwrap();
    let (_, body) = refusing.data_at(&URLRequest::new(&format!("{base}/whoami")), 101).unwrap();
    assert_eq!(body, b"out");
}

struct Ann;
impl URLSessionTaskDelegate for Ann {
    fn did_receive(&self, challenge: &AuthChallenge) -> AuthDisposition {
        assert_eq!(challenge.space.realm.as_deref(), Some("lab"));
        AuthDisposition::UseCredential(URLCredential::new("ann", "pw", CredentialPersistence::ForSession))
    }
}

#[test]
fn a_401_goes_to_the_delegate_and_the_credential_is_kept_per_protection_space() {
    let port = http_server();
    let s = session(URLSessionConfiguration::ephemeral(), Some(Arc::new(Ann)));
    let (resp, body) = s.data_at(&URLRequest::new(&format!("http://127.0.0.1:{port}/secret")), 0).unwrap();
    assert_eq!((resp.status_code, body.as_slice()), (200, &b"secret"[..]));
    assert_eq!(s.configuration.url_credential_storage.as_ref().unwrap().lock().unwrap().len(), 1);
    assert_eq!(s.last_metrics().unwrap().transactions.len(), 2);

    // no delegate and no stored credential: the 401 is returned
    let plain = session(URLSessionConfiguration::ephemeral(), None);
    let (resp, _) = plain.data_at(&URLRequest::new(&format!("http://127.0.0.1:{port}/secret")), 0).unwrap();
    assert_eq!(resp.status_code, 401);
}

#[test]
fn the_disk_cache_revalidates_with_the_etag_and_serves_fresh_entries_without_the_network() {
    let port = http_server();
    let dir = scratch("cache");
    let cache = Arc::new(URLCache::new(1 << 10, 1 << 20, &dir));
    let s = session(URLSessionConfiguration::default_configuration(Some(cache.clone())), None);
    let url = format!("http://127.0.0.1:{port}/etag");
    let before = NETWORK_HITS.load(Ordering::SeqCst);
    assert_eq!(s.data_at(&URLRequest::new(&url), 0).unwrap().1, b"etag");
    // fresh: no network
    assert_eq!(s.data_at(&URLRequest::new(&url), 30).unwrap().1, b"etag");
    assert_eq!(s.last_metrics().unwrap().transactions[0].resource_fetch_type, FetchType::LocalCache);
    // stale: conditional request answered with 304, body from the cache
    assert_eq!(s.data_at(&URLRequest::new(&url), 90).unwrap().1, b"etag");
    assert_eq!(NETWORK_HITS.load(Ordering::SeqCst) - before, 2);
    // a new session over the same directory still has it
    let again = URLCache::new(0, 1 << 20, &dir);
    assert!(again.cached_response(&url).is_some());
    let mut offline = URLSessionConfiguration::default_configuration(Some(Arc::new(again)));
    offline.request_cache_policy = CachePolicy::ReturnCacheDataDontLoad;
    assert_eq!(session(offline, None).data_at(&URLRequest::new(&url), 5000).unwrap().1, b"etag");
}

struct Finished(Option<u16>);

impl DownloadDelegate for Finished {
    fn did_write_data(&mut self, _: u64, _: u64, _: u64, _: Option<u64>) {}
    fn did_finish_downloading_to(&mut self, _: u64, _: &std::path::Path) {}
    fn did_complete(&mut self, _: u64, result: Result<bifrost_app::url::HTTPURLResponse, bifrost_app::url::URLError>) {
        self.0 = result.ok().map(|r| r.status_code);
    }
}

#[test]
fn a_background_session_resumes_its_queue_after_it_is_rebuilt() {
    let port = http_server();
    let dir = scratch("bg");
    let dest = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{port}/file");
    let config = URLSessionConfiguration::background("com.example.sync");
    assert_eq!(config.identifier(), Some("com.example.sync"));
    assert!(URLSessionConfiguration::ephemeral().background_session(URLSession::shared(), &dir).is_none());
    {
        let first = config.background_session(URLSession::shared(), &dir).unwrap();
        first.download_task(&url, &dest);
    }
    let rebuilt = config.background_session(URLSession::shared(), &dir).unwrap();
    assert_eq!(rebuilt.pending_tasks().len(), 1);
    let mut delegate = Finished(None);
    rebuilt.run(rebuilt.pending_tasks()[0].id, &mut delegate);
    assert_eq!(delegate.0, Some(200));
    assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    assert!(config.background_session(URLSession::shared(), &dir).unwrap().pending_tasks().is_empty());
}

fn ws_server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let head = read_head(&mut s).unwrap();
        let key = head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).unwrap().to_string();
        let reply = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key));
        s.write_all(reply.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = s.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Ok(Some((frame, used))) = decode(&buf) {
                buf.drain(..used);
                let out = match frame.opcode {
                    Opcode::Text => Some(Message::Text(format!("echo {}", String::from_utf8_lossy(&frame.payload)))),
                    Opcode::Binary => Some(Message::Binary(frame.payload.iter().rev().copied().collect())),
                    Opcode::Ping => Some(Message::Pong(frame.payload.clone())),
                    Opcode::Close => {
                        let _ = s.write_all(&encode(&Frame { fin: true, opcode: Opcode::Close, payload: frame.payload }, None));
                        return;
                    }
                    _ => None,
                };
                if let Some(m) = out {
                    s.write_all(&encode(&frame_for(&m), None)).unwrap();
                }
            }
        }
    });
    port
}

#[test]
fn a_websocket_task_sends_text_binary_and_ping_and_closes_with_a_code() {
    let port = ws_server();
    let t = Duration::from_secs(5);
    let mut ws = URLSessionWebSocketTask::connect("127.0.0.1", port, "/chat", &[], t).unwrap();
    ws.send(&Message::Text("hi".into())).unwrap();
    assert_eq!(ws.receive(t).unwrap(), Message::Text("echo hi".into()));
    ws.send(&Message::Binary(vec![1, 2, 3])).unwrap();
    assert_eq!(ws.receive(t).unwrap(), Message::Binary(vec![3, 2, 1]));
    ws.send_ping(t).unwrap();
    ws.cancel(1000, "bye").unwrap();
    assert!(ws.is_closed());
    assert_eq!(ws.send(&Message::Text("late".into())), Err(bifrost_app::websocket::TaskError::Closed));
}

#[test]
fn a_stream_task_reads_and_writes_lines() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.write_all(b"220 ready\r\n").unwrap();
        let mut b = [0u8; 64];
        let n = s.read(&mut b).unwrap();
        let mut echo = b"250 ".to_vec();
        echo.extend_from_slice(&b[..n]);
        s.write_all(&echo).unwrap();
    });
    let t = Duration::from_secs(5);
    let mut st = URLSessionStreamTask::connect("127.0.0.1", port, t).unwrap();
    assert_eq!(st.read_line(t).unwrap().as_deref(), Some("220 ready"));
    st.write_line("HELO lab", t).unwrap();
    assert_eq!(st.read_line(t).unwrap().as_deref(), Some("250 HELO lab"));
    assert_eq!(st.read_line(t).unwrap(), None);
}

#[test]
fn bonjour_services_publish_and_a_browser_finds_them() {
    let mut svc = NetService::new("Lab", "_http._tcp", 8080).unwrap();
    svc.publish();
    let mut b = NetServiceBrowser::new();
    b.search_for_services("_http._tcp").unwrap();
    b.service_appeared(svc, false);
    assert_eq!(b.found()[0].full_name(), "Lab._http._tcp.local.");
}
