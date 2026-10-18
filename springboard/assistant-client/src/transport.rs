//! Blocking HTTPS for one Messages request: rustls over a TCP socket (the system resolver, webpki roots), HTTP/1.1 with
//! `connection: close`. Call it from a worker thread; it waits for the whole reply.

use crate::wire::{HOST, VERSION};
use crate::Error;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(120);

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls_graviola::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("graviola supports TLS 1.2 and 1.3")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// POSTs `body` (a JSON Messages request) to the endpoint with `key`. Returns the HTTP status and the body text.
pub fn post_messages(key: &str, body: &str) -> Result<(u16, String), Error> {
    let unreachable = |e: &dyn std::fmt::Display| Error::Unreachable(format!("Cannot reach {HOST}: {e}"));
    let addr = (HOST, 443).to_socket_addrs().map_err(|e| unreachable(&e))?.next().ok_or_else(|| unreachable(&"no address"))?;
    let sock = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| unreachable(&e))?;
    sock.set_read_timeout(Some(TIMEOUT)).map_err(|e| unreachable(&e))?;
    sock.set_write_timeout(Some(TIMEOUT)).map_err(|e| unreachable(&e))?;
    let name = rustls::pki_types::ServerName::try_from(HOST).map_err(|e| unreachable(&e))?;
    let conn = rustls::ClientConnection::new(tls_config(), name).map_err(|e| unreachable(&e))?;
    let mut tls = rustls::StreamOwned::new(conn, sock);

    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: {HOST}\r\nx-api-key: {key}\r\nanthropic-version: {VERSION}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(head.as_bytes()).and_then(|_| tls.write_all(body.as_bytes())).and_then(|_| tls.flush()).map_err(|e| unreachable(&e))?;

    let mut raw = Vec::new();
    // A server that closes without a TLS close_notify ends the read with an error; what arrived is still the reply.
    if let Err(e) = tls.read_to_end(&mut raw) {
        if raw.is_empty() {
            return Err(unreachable(&e));
        }
    }
    parse_response(&raw)
}

/// Splits a raw HTTP/1.1 response into its status and body (de-chunked when the server chunks it).
pub fn parse_response(raw: &[u8]) -> Result<(u16, String), Error> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| Error::Reply("malformed HTTP response".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()).ok_or_else(|| Error::Reply("malformed HTTP status".into()))?;
    let chunked = head.lines().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked"))
    });
    let body = if chunked { dechunk(&raw[split + 4..]) } else { raw[split + 4..].to_vec() };
    Ok((status, String::from_utf8_lossy(&body).to_string()))
}

/// Decodes a chunked transfer body. A truncated final chunk keeps the bytes that arrived.
pub fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = data.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&data[..nl]).ok().and_then(|s| usize::from_str_radix(s.split(';').next()?.trim(), 16).ok()).unwrap_or(0);
        data = &data[nl + 2..];
        if size == 0 {
            break;
        }
        let take = size.min(data.len());
        out.extend_from_slice(&data[..take]);
        if take < size {
            break;
        }
        data = data.get(size + 2..).unwrap_or(&[]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_response_keeps_its_status_and_body() {
        let (status, body) = parse_response(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"a\":1}").unwrap();
        assert_eq!((status, body.as_str()), (200, "{\"a\":1}"));
    }

    #[test]
    fn a_chunked_response_is_decoded() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n";
        assert_eq!(parse_response(raw).unwrap(), (200, "Wikipedia".to_string()));
    }

    #[test]
    fn a_truncated_chunk_keeps_what_arrived() {
        assert_eq!(dechunk(b"a\r\n012"), b"012".to_vec());
    }

    #[test]
    fn an_error_status_is_returned_with_its_body() {
        let (status, body) = parse_response(b"HTTP/1.1 401 Unauthorized\r\n\r\n{\"error\":{}}").unwrap();
        assert_eq!(status, 401);
        assert_eq!(body, "{\"error\":{}}");
    }

    #[test]
    fn a_response_without_a_head_is_malformed() {
        assert!(matches!(parse_response(b"nonsense"), Err(Error::Reply(_))));
    }
}
