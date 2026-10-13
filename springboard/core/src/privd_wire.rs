//! The client side of the privileged helper's socket (`bifrost-privd`): one request, one reply. The frame is a 4-byte
//! big-endian length and a UTF-8 JSON body, at most `MAX_FRAME` bytes; the request is `{"id","method","params"}` and
//! the reply `{"id","ok","result"}` or `{"id","ok":false,"error":{"code","message"}}`. The server side and the
//! contract are in `springboard/src/privd`. The shell's `privd_client` and the apps both call this.

use crate::json::{self, Json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub const MAX_FRAME: usize = 64 * 1024;

/// A failed call: a privileged error code (`unauthorized`, `locked`, `bad_params`, `internal`, ...), or
/// `unavailable` when the helper cannot be reached or answers nonsense.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

fn unavailable(e: impl std::fmt::Display) -> WireError {
    WireError { code: "unavailable".into(), message: format!("Login service unavailable: {e}") }
}

/// Sends one request to the helper at `sock` and returns its result. `token` (the unlocked session's) is added as
/// `t` when given; the root socket needs none. `timeout` bounds each read and write, not the whole call.
pub fn call(sock: &Path, token: Option<&str>, method: &str, mut fields: Vec<(String, Json)>, timeout: Duration) -> Result<Json, WireError> {
    if let Some(t) = token {
        fields.push(("t".into(), Json::Str(t.to_string())));
    }
    let req = Json::Obj(vec![("id".into(), Json::Num(1.0)), ("method".into(), Json::Str(method.to_string())), ("params".into(), Json::Obj(fields))]);
    let body = req.encode();
    if body.len() > MAX_FRAME {
        return Err(WireError { code: "bad_params".into(), message: "request too large".into() });
    }
    let mut c = UnixStream::connect(sock).map_err(unavailable)?;
    c.set_read_timeout(Some(timeout)).and_then(|_| c.set_write_timeout(Some(timeout))).map_err(unavailable)?;
    c.write_all(&(body.len() as u32).to_be_bytes()).and_then(|_| c.write_all(body.as_bytes())).map_err(unavailable)?;
    let mut len = [0u8; 4];
    c.read_exact(&mut len).map_err(unavailable)?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(unavailable("reply too large"));
    }
    let mut buf = vec![0u8; n];
    c.read_exact(&mut buf).map_err(unavailable)?;
    let text = String::from_utf8(buf).map_err(unavailable)?;
    let reply = json::parse(&text).map_err(unavailable)?;
    if reply.get("ok") == Some(&Json::Bool(true)) {
        return Ok(reply.get("result").cloned().unwrap_or(Json::Null));
    }
    let e = reply.get("error");
    Err(WireError {
        code: e.and_then(|e| e.str_field("code")).unwrap_or_else(|| "internal".into()),
        message: e.and_then(|e| e.str_field("message")).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// Serves one connection: reads the request, checks it, and writes `reply`.
    fn serve_once(path: std::path::PathBuf, reply: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut len = [0u8; 4];
            s.read_exact(&mut len).unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            s.read_exact(&mut body).unwrap();
            s.write_all(&(reply.len() as u32).to_be_bytes()).unwrap();
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8(body).unwrap()
        })
    }

    fn temp_socket(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("privd-wire-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("s.sock")
    }

    #[test]
    fn a_result_comes_back_and_the_token_travels_as_t() {
        let path = temp_socket("ok");
        let server = serve_once(path.clone(), "{\"id\":1,\"ok\":true,\"result\":{\"steps\":[\"sgdisk\"]}}");
        let got = call(&path, Some("tok"), "disk.verify_volume", vec![("device".into(), Json::Str("sdb1".into()))], Duration::from_secs(5)).unwrap();
        let sent = server.join().unwrap();
        assert!(sent.contains("\"method\":\"disk.verify_volume\"") && sent.contains("\"t\":\"tok\""), "{sent}");
        assert_eq!(got.get("steps").and_then(Json::as_array).map(|a| a.len()), Some(1));
    }

    #[test]
    fn an_error_reply_keeps_its_code_and_message() {
        let path = temp_socket("err");
        let server = serve_once(path.clone(), "{\"id\":1,\"ok\":false,\"error\":{\"code\":\"bad_params\",\"message\":\"Unmount it first.\"}}");
        let e = call(&path, None, "disk.erase_disk", vec![], Duration::from_secs(5)).unwrap_err();
        server.join().unwrap();
        assert_eq!(e, WireError { code: "bad_params".into(), message: "Unmount it first.".into() });
    }

    #[test]
    fn a_missing_helper_is_unavailable() {
        let path = temp_socket("none");
        let e = call(&path, None, "system.ping", vec![], Duration::from_secs(1)).unwrap_err();
        assert_eq!(e.code, "unavailable");
    }
}
