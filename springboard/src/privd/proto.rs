//! privd wire contract (FROZEN by the lead; tracks must not edit). Spec: docs/privd-protocol.md.
//! JSON via `bifrost_core::json` (no serde). Framing: 4-byte big-endian length + UTF-8 JSON.

use bifrost_core::json::{self, Json};

pub const PROTO_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;

pub const SOCKET_PATH: &str = "/run/bifrost/privd.sock";
pub const ROOT_SOCKET_PATH: &str = "/run/bifrost/privd-root.sock";
pub const ENV_PRIVD: &str = "SPRINGBOARD_PRIVD";
pub const ENV_DATA: &str = "SPRINGBOARD_DATA";
pub const PAM_SERVICE: &str = "bifrost-login";
pub const SESSION_GROUP: &str = "bifrost-session";
pub const MARKER_USER_SESSION: &str = "/var/lib/springboard/.user-session-v1";
pub const FLAG_NO_LOGIN: &str = "/etc/bifrost/no-login";
pub const ACCOUNT_FILE: &str = "/var/lib/springboard/account";
pub const HEARTBEAT_STATE_DIR: &str = "/run/bifrost/user/state";

// Methods. UNAUTH ones work on the lock screen; AUTH ones need Unlocked + token in params `t`.
pub const M_PING: &str = "system.ping";
pub const M_LOGIN_ACCOUNT: &str = "login.account";
pub const M_LOGIN_AUTHENTICATE: &str = "login.authenticate";
pub const M_POWER_SHUTDOWN: &str = "power.shutdown";
pub const M_POWER_RESTART: &str = "power.restart";
pub const M_POWER_SLEEP: &str = "power.sleep";
pub const M_SESSION_LOCK: &str = "session.lock";
pub const M_SESSION_LOGOUT: &str = "session.logout";
pub const M_ACCOUNT_SET_PASSWORD: &str = "account.set_password";
pub const M_UPDATE_CHECK: &str = "update.check";
pub const M_UPDATE_APPLY: &str = "update.apply";
pub const M_UPDATE_STATUS: &str = "update.status";
pub const M_STORE_INSTALL: &str = "store.install";
pub const M_STORE_UNINSTALL: &str = "store.uninstall";
pub const M_TIME_SET_TIMEZONE: &str = "time.set_timezone";
pub const M_TIME_SET_NTP: &str = "time.set_ntp";
pub const M_LOCALE_SET: &str = "locale.set";
pub const M_NET_PREFIX: &str = "net.";
pub const M_BOOT_ATTEMPT: &str = "system.boot_attempt";
pub const M_BACKUP_RESTORE_APPLY: &str = "backup.restore_apply";

/// Methods allowed before login.
pub const UNAUTH_METHODS: &[&str] = &[
    M_PING,
    M_LOGIN_ACCOUNT,
    M_LOGIN_AUTHENTICATE,
    M_POWER_SHUTDOWN,
    M_POWER_RESTART,
    M_POWER_SLEEP,
];

pub fn is_unauth(method: &str) -> bool {
    UNAUTH_METHODS.contains(&method)
}

// Events pushed on a subscription connection.
pub const EV_LOGIN_STATE: &str = "login.state";
pub const EV_UPDATE_PROGRESS: &str = "update.progress";
pub const EV_INSTALL_PROGRESS: &str = "install.progress";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    Unauthorized,
    Locked,
    BadParams,
    WrongPassword,
    LockedOut,
    NoPassword,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::Locked => "locked",
            ErrorCode::BadParams => "bad_params",
            ErrorCode::WrongPassword => "wrong_password",
            ErrorCode::LockedOut => "locked_out",
            ErrorCode::NoPassword => "no_password",
            ErrorCode::Internal => "internal",
        }
    }
    pub fn parse(s: &str) -> Option<ErrorCode> {
        Some(match s {
            "unauthorized" => ErrorCode::Unauthorized,
            "locked" => ErrorCode::Locked,
            "bad_params" => ErrorCode::BadParams,
            "wrong_password" => ErrorCode::WrongPassword,
            "locked_out" => ErrorCode::LockedOut,
            "no_password" => ErrorCode::NoPassword,
            "internal" => ErrorCode::Internal,
            _ => return None,
        })
    }
}

/// `data` carries extra error fields (`fails`, `retry_secs`) as a Json object, or Null.
/// They are serialised flat next to `code` and `message`.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub data: Json,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Error {
        Error { code, message: message.into(), data: Json::Null }
    }
    pub fn with_data(mut self, data: Json) -> Error {
        self.data = data;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    /// Json::Obj (or Null when the method takes none).
    pub params: Json,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Ok { id: u64, result: Json },
    Err { id: u64, error: Error },
}

impl Request {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"id\":{},\"method\":{},\"params\":{}}}",
            self.id,
            json::quote(&self.method),
            write(&self.params)
        )
    }
    pub fn parse(text: &str) -> Result<Request, String> {
        let j = json::parse(text)?;
        let id = j.get("id").and_then(|v| v.as_u64()).ok_or("missing id")?;
        let method = j.get("method").and_then(|v| v.as_str()).ok_or("missing method")?.to_string();
        let params = j.get("params").cloned().unwrap_or(Json::Null);
        Ok(Request { id, method, params })
    }
}

impl Reply {
    pub fn to_json(&self) -> String {
        match self {
            Reply::Ok { id, result } => format!("{{\"id\":{},\"ok\":true,\"result\":{}}}", id, write(result)),
            Reply::Err { id, error } => {
                let mut extra = String::new();
                if let Json::Obj(fields) = &error.data {
                    for (k, v) in fields {
                        extra.push_str(&format!(",{}:{}", json::quote(k), write(v)));
                    }
                }
                format!(
                    "{{\"id\":{},\"ok\":false,\"error\":{{\"code\":{},\"message\":{}{}}}}}",
                    id,
                    json::quote(error.code.as_str()),
                    json::quote(&error.message),
                    extra
                )
            }
        }
    }
    pub fn parse(text: &str) -> Result<Reply, String> {
        let j = json::parse(text)?;
        let id = j.get("id").and_then(|v| v.as_u64()).ok_or("missing id")?;
        if matches!(j.get("ok"), Some(Json::Bool(true))) {
            return Ok(Reply::Ok { id, result: j.get("result").cloned().unwrap_or(Json::Null) });
        }
        let e = j.get("error").ok_or("missing error")?;
        let code = e
            .get("code")
            .and_then(|v| v.as_str())
            .and_then(ErrorCode::parse)
            .unwrap_or(ErrorCode::Internal);
        let message = e.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let data = match e {
            Json::Obj(f) => Json::Obj(f.iter().filter(|(k, _)| k != "code" && k != "message").cloned().collect()),
            _ => Json::Null,
        };
        Ok(Reply::Err { id, error: Error { code, message, data } })
    }
}

/// Build a Json object from pairs.
pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn jstr(v: &str) -> Json {
    Json::Str(v.to_string())
}

/// Serialise a Json value.
pub fn write(j: &Json) -> String {
    match j {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 9e15 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        Json::Str(t) => json::quote(t),
        Json::Arr(a) => format!("[{}]", a.iter().map(write).collect::<Vec<_>>().join(",")),
        Json::Obj(o) => format!(
            "{{{}}}",
            o.iter().map(|(k, v)| format!("{}:{}", json::quote(k), write(v))).collect::<Vec<_>>().join(",")
        ),
    }
}

/// Encode one frame (length prefix + body). Err if over MAX_FRAME.
pub fn frame(body: &str) -> Result<Vec<u8>, String> {
    if body.len() > MAX_FRAME {
        return Err("frame too large".into());
    }
    let mut v = (body.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(body.as_bytes());
    Ok(v)
}

/// Read one frame from a reader. Ok(None) on clean EOF.
pub fn read_frame(r: &mut impl std::io::Read) -> std::io::Result<Option<String>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "utf8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let r = Request { id: 7, method: M_LOGIN_AUTHENTICATE.into(), params: obj(vec![("password", jstr("a\"b"))]) };
        assert_eq!(Request::parse(&r.to_json()).unwrap(), r);
    }

    #[test]
    fn error_reply_carries_data() {
        let e = Error::new(ErrorCode::WrongPassword, "no").with_data(obj(vec![("fails", Json::Num(2.0))]));
        let t = Reply::Err { id: 1, error: e.clone() }.to_json();
        assert_eq!(Reply::parse(&t).unwrap(), Reply::Err { id: 1, error: e });
    }

    #[test]
    fn frames_round_trip() {
        let f = frame("{\"a\":1}").unwrap();
        let mut c = std::io::Cursor::new(f);
        assert_eq!(read_frame(&mut c).unwrap().as_deref(), Some("{\"a\":1}"));
        assert_eq!(read_frame(&mut c).unwrap(), None);
    }

    #[test]
    fn unauth_list() {
        assert!(is_unauth(M_LOGIN_AUTHENTICATE));
        assert!(!is_unauth(M_UPDATE_APPLY));
    }
}
