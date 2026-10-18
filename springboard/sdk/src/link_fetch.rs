//! The request behind link metadata: `LPMetadataProvider.startFetchingMetadata(for:)`.
//!
//! `link_metadata` parses a page and keeps the state of one fetch but never opens a connection. [`fetch`] does the
//! request with a `URLSession`: a GET with the fetch's timeout, reading only as much of the page as holds its `<head>`
//! (the metadata is there; the read stops at `</head>` or after 512 KiB), and reports the outcome through the same
//! `LinkFetch` state, so the errors are the `LinkError` values an app already handles. It is blocking; an app calls it from
//! a background thread, as with `URLSession::bytes`. The only host contacted is the one in the address the app passes.

use crate::link_metadata::{FetchOptions, FetchState, LinkError, LinkFetch, LinkMetadata};
use crate::url::{URLError, URLRequest, URLSession};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// The most of a page the fetch reads.
pub const MAX_BODY: usize = 512 * 1024;

fn map_error(error: &URLError) -> LinkError {
    match error {
        URLError::TimedOut => LinkError::Timeout,
        URLError::Cancelled => LinkError::Cancelled,
        URLError::BadURL(_) => LinkError::InvalidUrl,
        _ => LinkError::Network,
    }
}

fn head_ended(body: &[u8]) -> bool {
    body.windows(7).any(|w| w.eq_ignore_ascii_case(b"</head>"))
}

/// Fetches the metadata of `url`: its title, icon and picture. A 3xx redirect is followed by the session; a status
/// outside 200 to 299 is `Network`, a body that is not HTML is `NotHtml`, and a slow page is `Timeout`.
pub fn fetch(session: &URLSession, url: &str, options: FetchOptions) -> Result<LinkMetadata, LinkError> {
    let mut link = LinkFetch::start(url, options)?;
    let mut request = URLRequest::new(link.url());
    request.timeout = Duration::from_millis(link.timeout_ms());
    request.set_header("Accept", "text/html,application/xhtml+xml");
    let cancel = AtomicBool::new(false);
    let mut status = 0u16;
    let mut content_type = String::new();
    let mut body: Vec<u8> = Vec::new();
    let mut stopped = false;
    let result = session.bytes(
        &request,
        &cancel,
        &mut |head| {
            status = head.status_code;
            content_type = head.value("Content-Type").unwrap_or("").to_string();
        },
        &mut |chunk| {
            body.extend_from_slice(chunk);
            if body.len() >= MAX_BODY || head_ended(&body) {
                stopped = true;
                return false;
            }
            true
        },
    );
    match result {
        Ok(_) => {}
        // the read stopped on purpose once the head was in
        Err(URLError::Cancelled) if stopped => {}
        Err(e) => {
            let error = map_error(&e);
            link.fail(error);
            return Err(error);
        }
    }
    if !(200..300).contains(&status) {
        link.fail(LinkError::Network);
    } else {
        link.finish_html(&String::from_utf8_lossy(&body), &content_type);
    }
    match link.state() {
        FetchState::Done(metadata) => Ok(metadata.clone()),
        FetchState::Failed(error) => Err(*error),
        FetchState::Pending => Err(LinkError::Network),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves one answer to every connection until the test ends.
    fn serve(reply: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let reply = reply.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 2048];
                    let mut seen = Vec::new();
                    while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => seen.extend_from_slice(&buf[..n]),
                        }
                    }
                    let _ = stream.write_all(&reply);
                });
            }
        });
        port
    }

    fn page(body: &str, content_type: &str) -> Vec<u8> {
        format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn a_page_gives_its_title_icon_and_picture() {
        let html = r#"<html><head><title>Plain</title>
            <meta property="og:title" content="Fréttir">
            <meta property="og:image" content="/pic.jpg">
            <link rel="icon" href="/i.png"></head><body>text</body></html>"#;
        let port = serve(page(html, "text/html; charset=utf-8"));
        let url = format!("http://127.0.0.1:{port}/a");
        let m = fetch(&URLSession::shared(), &url, FetchOptions::default()).unwrap();
        assert_eq!(m.url, url);
        assert_eq!(m.title.as_deref(), Some("Fréttir"));
        assert_eq!(m.image, Some(format!("http://127.0.0.1:{port}/pic.jpg")));
        assert_eq!(m.icon, Some(format!("http://127.0.0.1:{port}/i.png")));
    }

    #[test]
    fn a_page_that_is_not_html_is_refused() {
        let port = serve(page("{\"a\":1}", "application/json"));
        let r = fetch(&URLSession::shared(), &format!("http://127.0.0.1:{port}/"), FetchOptions::default());
        assert_eq!(r, Err(LinkError::NotHtml));
    }

    #[test]
    fn an_error_status_is_a_network_failure() {
        let port = serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec());
        let r = fetch(&URLSession::shared(), &format!("http://127.0.0.1:{port}/"), FetchOptions::default());
        assert_eq!(r, Err(LinkError::Network));
    }

    #[test]
    fn an_unreachable_host_is_a_network_failure() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let r = fetch(&URLSession::shared(), &format!("http://127.0.0.1:{port}/"), FetchOptions::default());
        assert_eq!(r, Err(LinkError::Network));
    }

    #[test]
    fn an_address_that_is_not_a_web_address_never_reaches_the_network() {
        let r = fetch(&URLSession::shared(), "javascript:alert(1)", FetchOptions::default());
        assert_eq!(r, Err(LinkError::InvalidUrl));
        assert_eq!(fetch(&URLSession::shared(), "ftp://example.com/", FetchOptions::default()), Err(LinkError::InvalidUrl));
    }

    #[test]
    fn the_read_stops_at_the_end_of_the_head() {
        assert!(head_ended(b"<head><title>x</title></HEAD><body>"));
        assert!(!head_ended(b"<head><title>x</title>"));
        assert_eq!(map_error(&URLError::TimedOut), LinkError::Timeout);
        assert_eq!(map_error(&URLError::Cancelled), LinkError::Cancelled);
        assert_eq!(map_error(&URLError::CannotConnect("x".into())), LinkError::Network);
    }
}
