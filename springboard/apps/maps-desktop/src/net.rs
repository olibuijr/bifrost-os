//! The network the app reads through: one GET with the identifying user agent the tile and geocoder usage policies ask
//! for. The window uses `Live`; tests and the pinned testing kit use fakes, so no test reaches a server.

use springboard::deildu::http::{Client, Tls, Transport};
use std::io;
use std::sync::{Arc, OnceLock};

/// Largest reply the app reads (a routing reply with a full geometry is the biggest).
pub const REPLY_LIMIT: usize = 4 * 1024 * 1024;

pub trait Net: Send + Sync {
    /// GET `url` with `user_agent`; the body of a 200 reply.
    fn get(&self, url: &str, user_agent: &str, limit: usize) -> io::Result<Vec<u8>>;
}

/// HTTPS and HTTP through the shell's own client.
#[derive(Default)]
pub struct Live {
    client: OnceLock<Client>,
}

impl Net for Live {
    fn get(&self, url: &str, user_agent: &str, limit: usize) -> io::Result<Vec<u8>> {
        let client = self.client.get_or_init(|| Client::new(Arc::new(Tls::new()) as Arc<dyn Transport>, Default::default()));
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

#[cfg(test)]
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
