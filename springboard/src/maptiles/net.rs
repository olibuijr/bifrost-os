//! The network the app reads through: one GET with the identifying user agent the tile and geocoder usage policies ask
//! for. The window uses `Live`; tests and the pinned testing kit use fakes, so no test reaches a server.

use crate::deildu::http::{Client, Tls, Transport};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

/// Largest reply the app reads (a routing reply with a full geometry is the biggest).
pub const REPLY_LIMIT: usize = 4 * 1024 * 1024;

pub trait Net: Send + Sync {
    /// GET `url` with `user_agent`; the body of a 200 reply.
    fn get(&self, url: &str, user_agent: &str, limit: usize) -> io::Result<Vec<u8>>;
}

/// The most clients (one per scheme and host) kept; past it they are all dropped and made again.
const MAX_CLIENTS: usize = 32;

/// HTTPS and HTTP through the shell's own client. Each host gets a client of its own, so the cookies one service sets are
/// never sent to another, and a request to an `https` address is made by a client that refuses a redirect to plain
/// `http` (the replies of a tile, search or route server cannot be swapped by a network attacker that way). An address
/// the person configured as `http` is still read over `http`.
#[derive(Default)]
pub struct Live {
    transport: OnceLock<Arc<dyn Transport>>,
    clients: Mutex<HashMap<(bool, String), Arc<Client>>>,
}

/// What decides which client serves `url`: whether it is `https`, and its host. An address with another scheme or no
/// host is refused.
fn client_key(url: &str) -> io::Result<(bool, String)> {
    let u = url::Url::parse(url).map_err(|_| io::Error::other("bad url"))?;
    let secure = match u.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(io::Error::other("unsupported scheme")),
    };
    let host = u.host_str().filter(|h| !h.is_empty()).ok_or_else(|| io::Error::other("missing host"))?;
    Ok((secure, host.to_ascii_lowercase()))
}

impl Live {
    /// A client over `transport` instead of the real network.
    pub fn with_transport(transport: Arc<dyn Transport>) -> Live {
        let live = Live::default();
        let _ = live.transport.set(transport);
        live
    }

    fn client_for(&self, url: &str) -> io::Result<Arc<Client>> {
        let key = client_key(url)?;
        let transport = self.transport.get_or_init(|| Arc::new(Tls::new()) as Arc<dyn Transport>).clone();
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if clients.len() >= MAX_CLIENTS && !clients.contains_key(&key) {
            clients.clear();
        }
        let secure = key.0;
        Ok(clients
            .entry(key)
            .or_insert_with(|| {
                let client = Client::new(transport, Default::default());
                Arc::new(if secure { client.require_https() } else { client })
            })
            .clone())
    }
}

impl Net for Live {
    fn get(&self, url: &str, user_agent: &str, limit: usize) -> io::Result<Vec<u8>> {
        let client = self.client_for(url)?;
        let r = client.get_with_headers(url, &[("User-Agent", user_agent)], limit)?;
        if r.status != 200 {
            return Err(io::Error::other(format!("status {}", r.status)));
        }
        Ok(r.body)
    }
}

/// No network: every request fails, so a snapshot never shows live data.
pub struct Offline;

impl Net for Offline {
    fn get(&self, _url: &str, _user_agent: &str, _limit: usize) -> io::Result<Vec<u8>> {
        Err(io::Error::other("offline"))
    }
}

#[cfg(any(test, feature = "testkit"))]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Answers by URL prefix and records every request as (url, user agent).
    #[derive(Default)]
    pub struct Fake {
        pub routes: Mutex<Vec<(String, Vec<u8>)>>,
        pub seen: Mutex<Vec<(String, String)>>,
    }

    impl Fake {
        pub fn with(prefix: &str, body: &[u8]) -> Arc<Fake> {
            let f = Fake::default();
            f.routes.lock().unwrap().push((prefix.to_string(), body.to_vec()));
            Arc::new(f)
        }
    }

    impl Net for Fake {
        fn get(&self, url: &str, user_agent: &str, _limit: usize) -> io::Result<Vec<u8>> {
            self.seen.lock().unwrap().push((url.to_string(), user_agent.to_string()));
            self.routes
                .lock()
                .unwrap()
                .iter()
                .find(|(p, _)| url.starts_with(p.as_str()))
                .map(|(_, b)| b.clone())
                .ok_or_else(|| io::Error::other("no route"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deildu::http::Stream;
    use std::io::{Cursor, Read, Write};

    /// A transport that answers every request with `reply` and records each connection as (host, tls).
    struct Canned {
        reply: Vec<u8>,
        connects: Mutex<Vec<(String, bool)>>,
    }

    struct Conn(Cursor<Vec<u8>>);

    impl Read for Conn {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Conn {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Transport for Canned {
        fn connect(&self, host: &str, _port: u16, tls: bool) -> io::Result<Box<dyn Stream>> {
            self.connects.lock().unwrap().push((host.to_string(), tls));
            Ok(Box::new(Conn(Cursor::new(self.reply.clone()))))
        }
    }

    fn canned(reply: &str) -> Arc<Canned> {
        Arc::new(Canned { reply: reply.as_bytes().to_vec(), connects: Mutex::new(Vec::new()) })
    }

    #[test]
    fn a_client_is_chosen_by_scheme_and_host() {
        assert_eq!(client_key("https://Tile.Example.org/1/2/3.png").unwrap(), (true, "tile.example.org".to_string()));
        assert_eq!(client_key("http://localhost:8080/x").unwrap(), (false, "localhost".to_string()));
        assert!(client_key("ftp://example.org/x").is_err());
        assert!(client_key("not a url").is_err());
    }

    #[test]
    fn a_redirect_from_https_to_http_is_refused() {
        let transport = canned("HTTP/1.1 302 Found\r\nLocation: http://other.example/t.png\r\nContent-Length: 0\r\n\r\n");
        let live = Live::with_transport(transport.clone());
        let err = live.get("https://tile.example/1/2/3.png", "test", 1000).unwrap_err();
        assert!(err.to_string().contains("HTTPS required"), "{err}");
        let connects = transport.connects.lock().unwrap();
        assert_eq!(*connects, vec![("tile.example".to_string(), true)], "no plain connection was made");
    }

    #[test]
    fn an_http_address_the_person_configured_still_loads() {
        let transport = canned("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
        let live = Live::with_transport(transport.clone());
        assert_eq!(live.get("http://maps.lan/1/2/3.png", "test", 1000).unwrap(), b"hi");
    }

    #[test]
    fn every_host_has_its_own_client_and_so_its_own_cookies() {
        let live = Live::with_transport(canned("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
        let a = live.client_for("https://a.example/x").unwrap();
        let b = live.client_for("https://b.example/x").unwrap();
        let a_again = live.client_for("https://a.example/y").unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&a, &a_again));
        let plain = live.client_for("http://a.example/x").unwrap();
        assert!(!Arc::ptr_eq(&a, &plain), "the https-only client is not the plain one");
    }

    #[test]
    fn the_clients_are_bounded() {
        let live = Live::with_transport(canned(""));
        for n in 0..(MAX_CLIENTS + 10) {
            live.client_for(&format!("https://h{n}.example/x")).unwrap();
        }
        assert!(live.clients.lock().unwrap().len() <= MAX_CLIENTS);
    }
}
