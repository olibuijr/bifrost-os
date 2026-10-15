//! Calls from an app to a background service that the supervisor keeps (`bifrostd`, docs/services.md).
//!
//! A service is a `kind = "service"` package. Its socket is `<root>/run/<id>.sock`: the supervisor binds it, and starts
//! the service when a connection waits on it (socket activation). So connecting is the launch: the first `ping` may take
//! a cold start, which is why the default timeout is longer than a single round trip. The app's sandbox binds only the
//! service sockets it may reach, so `ServiceError::NotInstalled` also means the app was not granted that service.
//!
//! Frames are the service wire format of `bifrost_proto::svc`: a little-endian length, then the payload.

use bifrost_proto::svc::{self, common};
use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The shell's data folder, where the supervisor keeps `run/` when `BIFROST_SHELL_DIR` is not set.
pub const DEFAULT_ROOT: &str = "/var/lib/springboard";
/// The time a read or write may wait. It covers the start of a service that is not running yet.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// A service identifier, such as `is.olibuijr.audio`. It is also the name of the service's socket file, so it holds
/// only the characters the supervisor accepts in a manifest id: ASCII letters, digits, `.`, `-` and `_`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServiceId(String);

impl ServiceId {
    pub fn new(id: &str) -> Result<ServiceId, ServiceError> {
        let safe = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if safe {
            Ok(ServiceId(id.to_string()))
        } else {
            Err(ServiceError::InvalidId(id.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The socket a service listens on under `root`.
pub fn socket_path(root: &Path, id: &ServiceId) -> PathBuf {
    root.join("run").join(format!("{}.sock", id.as_str()))
}

/// What a service reports about itself in its reply to a ping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceInfo {
    pub id: String,
    pub version: String,
}

/// Why a call to a service failed. Each variant names one condition an app can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// The text is not a valid service identifier, so no socket is opened.
    InvalidId(String),
    /// There is no socket for the service: it is not installed, the supervisor is not running, or the sandbox does not
    /// bind the socket.
    NotInstalled,
    /// The socket file exists but nothing accepts on it (the supervisor has not bound it yet, or it is stale).
    NotListening,
    /// A read or write waited longer than the timeout.
    Timeout,
    /// The service closed the connection before it replied (it crashed or exited during the call).
    Disconnected,
    /// The reply to a ping came from a different service than the one the client connected to.
    WrongService { expected: String, found: String },
    /// The request is longer than the frame limit, so it was not sent.
    TooLarge { len: usize, max: usize },
    /// The service sent a frame that does not decode, or a frame longer than the limit.
    Protocol(String),
    /// Any other input or output failure.
    Io(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::InvalidId(id) => write!(f, "{id:?} is not a service identifier"),
            ServiceError::NotInstalled => write!(f, "the service has no socket"),
            ServiceError::NotListening => write!(f, "the service socket accepts no connections"),
            ServiceError::Timeout => write!(f, "the service did not answer in time"),
            ServiceError::Disconnected => write!(f, "the service closed the connection"),
            ServiceError::WrongService { expected, found } => write!(f, "expected {expected}, the socket answered as {found}"),
            ServiceError::TooLarge { len, max } => write!(f, "a {len} byte request is over the {max} byte frame limit"),
            ServiceError::Protocol(why) => write!(f, "the service sent a bad frame: {why}"),
            ServiceError::Io(why) => write!(f, "the service connection failed: {why}"),
        }
    }
}

impl std::error::Error for ServiceError {}

fn map_io(err: io::Error) -> ServiceError {
    match err.kind() {
        io::ErrorKind::NotFound => ServiceError::NotInstalled,
        io::ErrorKind::ConnectionRefused => ServiceError::NotListening,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ServiceError::Timeout,
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset => ServiceError::Disconnected,
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => ServiceError::Protocol(err.to_string()),
        _ => ServiceError::Io(err.to_string()),
    }
}

/// An open connection to one service.
pub struct ServiceClient {
    id: ServiceId,
    stream: UnixStream,
}

impl ServiceClient {
    /// Connects to `id`'s socket under `root`, with `timeout` on every read and write.
    pub fn connect(root: &Path, id: &ServiceId, timeout: Duration) -> Result<ServiceClient, ServiceError> {
        let stream = UnixStream::connect(socket_path(root, id)).map_err(map_io)?;
        stream.set_read_timeout(Some(timeout)).map_err(map_io)?;
        stream.set_write_timeout(Some(timeout)).map_err(map_io)?;
        Ok(ServiceClient { id: id.clone(), stream })
    }

    /// The service's identifier and version. The service must answer as the id the client connected to.
    pub fn ping(&mut self) -> Result<ServiceInfo, ServiceError> {
        let reply = self.call(&common::ping())?;
        let (id, version) = common::parse_pong(&reply).map_err(|e| ServiceError::Protocol(format!("{e:?}")))?;
        if id != self.id.as_str() {
            return Err(ServiceError::WrongService { expected: self.id.as_str().to_string(), found: id });
        }
        Ok(ServiceInfo { id, version })
    }

    /// Sends one request frame and returns the reply frame. The payload is the service's own message encoding.
    pub fn call(&mut self, payload: &[u8]) -> Result<Vec<u8>, ServiceError> {
        if payload.len() > svc::MAX_FRAME {
            return Err(ServiceError::TooLarge { len: payload.len(), max: svc::MAX_FRAME });
        }
        svc::write_frame(&mut self.stream, payload).map_err(map_io)?;
        svc::read_frame(&mut self.stream).map_err(map_io)
    }
}
