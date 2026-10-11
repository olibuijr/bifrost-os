//! HTTPS access for the store and Software Update: the Deildu rustls client,
//! built once on first use. Always called from worker threads.

use crate::deildu::http::{Client, Tls, Transport};
use std::io;
use std::sync::{Arc, OnceLock};

pub const STORE_BASE: &str = "https://bifrost.olibuijr.com";

fn client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| Client::new(Arc::new(Tls::new()) as Arc<dyn Transport>, Default::default()).require_https())
}

/// `path` against the store origin (absolute URLs pass through).
pub fn absolute(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{STORE_BASE}/{}", path.trim_start_matches('/'))
    }
}

pub fn get(url: &str, limit: usize) -> io::Result<Vec<u8>> {
    let r = client().get(url, None, limit)?;
    if r.status != 200 {
        return Err(io::Error::other(format!("server answered {}", r.status)));
    }
    Ok(r.body)
}

/// POSTs `body` as text/plain; the status code of the answer.
pub fn post_text(url: &str, body: &[u8]) -> io::Result<u16> {
    Ok(client().post_text(url, body, 4096)?.status)
}

/// Downloads `url` in byte-range pieces (so progress is real), handing each
/// piece to `sink`. A server that ignores ranges sends one whole body.
pub fn get_chunked(url:&str,size:u64,sink:impl FnMut(&[u8])->io::Result<()>) -> io::Result<()> {
    get_streamed_from(url,size,0,sink)
}
pub fn get_streamed_from(url:&str,size:u64,start:u64,sink:impl FnMut(&[u8])->io::Result<()>) -> io::Result<()> {
    client().get_streamed(url,start,size.saturating_sub(start),sink)
}
