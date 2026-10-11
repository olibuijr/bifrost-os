//! Contract test for `bifrost_app::url` (URLSession parity) against a local TcpListener HTTP server.

use bifrost_app::dispatch::drain_main;
use bifrost_app::url::{URLCache, URLError, URLRequest, URLSession};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

static HITS: AtomicUsize = AtomicUsize::new(0);

fn serve() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut req = Vec::new();
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let text = String::from_utf8_lossy(&req).to_string();
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let reply: Vec<u8> = match path.as_str() {
                    "/cached" => {
                        HITS.fetch_add(1, Ordering::SeqCst);
                        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\nabc".to_vec()
                    }
                    "/etag" => {
                        if text.contains("If-None-Match: \"v1\"") {
                            b"HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\n\r\n".to_vec()
                        } else {
                            b"HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: 4\r\n\r\netag".to_vec()
                        }
                    }
                    "/redir" => b"HTTP/1.1 302 Found\r\nLocation: /chunked\r\nContent-Length: 0\r\n\r\n".to_vec(),
                    "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".to_vec(),
                    "/big" => {
                        let mut v = b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n".to_vec();
                        v.extend(std::iter::repeat(7u8).take(100_000));
                        v
                    }
                    _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
                };
                let _ = s.write_all(&reply);
            });
        }
    });
    port
}

/// Runs the main queue (this test thread) until the channel yields.
fn wait<T>(rx: &mpsc::Receiver<T>) -> T {
    for _ in 0..1000 {
        drain_main();
        if let Ok(v) = rx.recv_timeout(Duration::from_millis(10)) {
            return v;
        }
    }
    panic!("timed out");
}

#[test]
fn url_session_contract() {
    let port = serve();
    let base = format!("http://127.0.0.1:{port}");
    let s = URLSession::shared();
    drain_main(); // this thread becomes "main"
    let main_id = std::thread::current().id();

    // redirect + chunked; completion on the main thread
    let (tx, rx) = mpsc::channel();
    s.data_task(URLRequest::new(&format!("{base}/redir")), move |r| tx.send((r, std::thread::current().id())).unwrap());
    let (r, id) = wait(&rx);
    let (resp, body) = r.unwrap();
    assert_eq!(id, main_id);
    assert_eq!((resp.status_code, body.as_slice()), (200, &b"hello world"[..]));

    // 404 is a response, not an error
    let (tx, rx) = mpsc::channel();
    s.data_task(URLRequest::new(&format!("{base}/nope")), move |r| tx.send(r).unwrap());
    assert_eq!(wait(&rx).unwrap().0.status_code, 404);

    // max-age: the second request is served from the cache
    for _ in 0..2 {
        let (tx, rx) = mpsc::channel();
        s.data_task(URLRequest::new(&format!("{base}/cached")), move |r| tx.send(r).unwrap());
        assert_eq!(wait(&rx).unwrap().1, b"abc");
    }
    assert_eq!(HITS.load(Ordering::SeqCst), 1);

    // ETag: no max-age means stale at once, so the second request revalidates and a 304 serves the cached body
    for _ in 0..2 {
        let (tx, rx) = mpsc::channel();
        s.data_task(URLRequest::new(&format!("{base}/etag")), move |r| tx.send(r).unwrap());
        let (resp, body) = wait(&rx).unwrap();
        assert_eq!((resp.status_code, body.as_slice()), (200, &b"etag"[..]));
    }
    let uncached = URLSession::with_transport(Arc::new(springboard::deildu::http::Tls::new()), None);
    assert!(uncached.url_cache().is_none());
    assert!(s.url_cache().is_some_and(|c: &Arc<URLCache>| !c.is_empty()));

    // bad scheme
    let (tx, rx) = mpsc::channel();
    s.data_task(URLRequest::new("ftp://x"), move |r| tx.send(r).unwrap());
    assert!(matches!(wait(&rx), Err(URLError::BadURL(_))));

    // download with progress
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("big.bin");
    let seen = Arc::new(AtomicUsize::new(0));
    let seen2 = seen.clone();
    let (tx, rx) = mpsc::channel();
    s.download_task(
        URLRequest::new(&format!("{base}/big")),
        dest.clone(),
        move |got, total| {
            assert_eq!(total, Some(100_000));
            seen2.store(got as usize, Ordering::SeqCst);
        },
        move |r| tx.send(r).unwrap(),
    );
    assert_eq!(wait(&rx).unwrap().status_code, 200);
    drain_main();
    assert_eq!(std::fs::metadata(&dest).unwrap().len(), 100_000);
    assert_eq!(seen.load(Ordering::SeqCst), 100_000);
}
