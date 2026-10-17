//! SMB client for apps (the reference's SMBClient and its engine): smb:// addresses, share and folder listings, and
//! file download from a Windows or Samba file server. The protocol engine is Samba's `smbclient`, which the system
//! packages (`packaging/arch/runtime-packages.txt`); this module is what the Files app and the Connect to Server sheet
//! need around it, and the parts that decide safety are pure and tested without a server.
//!
//! * An address is `smb://[domain;][user@]host[:port][/share[/folder/...]]`. A password in the address is refused
//!   (addresses end up in history and process lists); the password comes from the caller, usually from the keychain's
//!   share login (`keychain::share_login`), and goes to the client through the `PASSWD` environment variable, never
//!   the command line.
//! * The client is told to speak SMB 2.02 or newer (`client min protocol`), which drops SMB 1, and to encrypt the
//!   session when the server can (`client smb encrypt = desired`).
//! * Share, folder and file names are checked before they go into an `smbclient -c` command: that command splits on
//!   `;`, so `;`, quotes, wildcards, control characters and `..` are refused rather than escaped.
//!
//! Nothing here opens a socket itself and nothing leaves the machine except what the user asked the client to reach.
//! Run the calls on a background thread: each blocks for up to [`TIMEOUT`].

use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The TCP port of an SMB server when an address names none.
pub const DEFAULT_PORT: u16 = 445;
/// The client program.
pub const SMBCLIENT: &str = "smbclient";
/// The oldest protocol dialect the client may speak.
pub const MIN_PROTOCOL: &str = "SMB2_02";
/// How long a call may take before it is stopped and reported as [`Error::TimedOut`].
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// Why an address, a name or a call was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The text does not start with smb:// (compared without case).
    Scheme,
    /// The address has no host.
    EmptyHost,
    /// The host has a character a host name does not have. IPv6 literals are not handled.
    BadHost,
    /// The port is not a number from 1 to 65535.
    BadPort,
    /// A percent escape is not two hex digits, or the decoded bytes are not UTF-8.
    BadEscape,
    /// The address carries a password (`user:password@`).
    PasswordInUrl,
    /// A share, folder or file name has a character that is not allowed (see the module docs) or is `..`.
    BadName,
    /// The call needs a share or a file in the address and it has none.
    Missing(&'static str),
    /// The client program is not installed.
    NoClient,
    /// The server could not be reached.
    Unreachable,
    /// The server answered and refused the connection.
    Refused,
    /// The user name or password was not accepted.
    AuthFailed,
    /// The login was accepted and the server denied this action.
    AccessDenied,
    /// The server has no such share.
    NoSuchShare,
    /// The share has no such folder or file.
    NoSuchPath,
    /// The call took longer than [`TIMEOUT`].
    TimedOut,
    /// Any other failure, with the client's status name or first line.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Scheme => f.write_str("address must start with smb://"),
            Error::EmptyHost => f.write_str("address has no server name"),
            Error::BadHost => f.write_str("server name has a character that is not allowed"),
            Error::BadPort => f.write_str("port must be from 1 to 65535"),
            Error::BadEscape => f.write_str("bad percent escape in address"),
            Error::PasswordInUrl => f.write_str("do not put the password in the address; it is asked for separately"),
            Error::BadName => f.write_str("share, folder or file name has a character that is not allowed"),
            Error::Missing(what) => write!(f, "the address needs a {what}"),
            Error::NoClient => f.write_str("the SMB client (smbclient) is not installed"),
            Error::Unreachable => f.write_str("the server could not be reached"),
            Error::Refused => f.write_str("the server refused the connection"),
            Error::AuthFailed => f.write_str("the user name or password was not accepted"),
            Error::AccessDenied => f.write_str("the server denied access"),
            Error::NoSuchShare => f.write_str("the server has no such shared folder"),
            Error::NoSuchPath => f.write_str("no such folder or file"),
            Error::TimedOut => f.write_str("the server took too long to answer"),
            Error::Failed(why) => write!(f, "the SMB client failed: {why}"),
        }
    }
}

/// A parsed smb:// address. Escapes are decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbUrl {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub domain: Option<String>,
    /// The share, when the address names one.
    pub share: Option<String>,
    /// The folder or file inside the share, `/` separated, with no leading or trailing slash. Empty for the share root.
    pub path: String,
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode(text: &str) -> Result<String, Error> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1).copied().and_then(hex).ok_or(Error::BadEscape)?;
            let lo = bytes.get(i + 2).copied().and_then(hex).ok_or(Error::BadEscape)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| Error::BadEscape)
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'$') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Whether `name` (one share, folder or file name) may go into a client command.
fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name != ".."
        && name != "."
        && !name.chars().any(|c| c.is_control() || matches!(c, '"' | '\\' | ';' | '*' | '?' | '<' | '>' | '|' | ':' | '/'))
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

impl SmbUrl {
    /// Parses `smb://[domain;][user@]host[:port][/share[/folder/...]]`.
    pub fn parse(text: &str) -> Result<SmbUrl, Error> {
        let prefix = text.get(..6).filter(|p| p.eq_ignore_ascii_case("smb://")).ok_or(Error::Scheme)?;
        let rest = &text[prefix.len()..];
        let (authority, tail) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
            None => (None, authority),
        };
        let (domain, user) = match userinfo {
            None => (None, None),
            Some(u) => {
                if u.contains(':') {
                    return Err(Error::PasswordInUrl);
                }
                let u = decode(u)?;
                match u.split_once(';') {
                    Some((d, n)) => (non_empty(d), non_empty(n)),
                    None => (None, non_empty(&u)),
                }
            }
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().ok().filter(|&p| p > 0).ok_or(Error::BadPort)?),
            None => (hostport, DEFAULT_PORT),
        };
        if host.is_empty() {
            return Err(Error::EmptyHost);
        }
        if !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) {
            return Err(Error::BadHost);
        }
        let mut segments = Vec::new();
        for raw in tail.split('/').filter(|s| !s.is_empty()) {
            let s = decode(raw)?;
            if !name_ok(&s) {
                return Err(Error::BadName);
            }
            segments.push(s);
        }
        let mut segments = segments.into_iter();
        let share = segments.next();
        let path = segments.collect::<Vec<_>>().join("/");
        Ok(SmbUrl { host: host.to_ascii_lowercase(), port, user, domain, share, path })
    }

    /// The address as text, without any password (there is none to leave out). Round-trips through [`parse`](Self::parse).
    pub fn to_text(&self) -> String {
        let mut out = String::from("smb://");
        if let Some(user) = &self.user {
            if let Some(domain) = &self.domain {
                out.push_str(&encode(domain));
                out.push(';');
            }
            out.push_str(&encode(user));
            out.push('@');
        }
        out.push_str(&self.host);
        if self.port != DEFAULT_PORT {
            out.push_str(&format!(":{}", self.port));
        }
        if let Some(share) = &self.share {
            out.push('/');
            out.push_str(&encode(share));
            for seg in self.path.split('/').filter(|s| !s.is_empty()) {
                out.push('/');
                out.push_str(&encode(seg));
            }
        }
        out
    }
}

/// How the client logs in. No user means a guest login.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Credentials {
    pub user: Option<String>,
    pub domain: Option<String>,
    pub password: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // the password never reaches a log
        f.debug_struct("Credentials").field("user", &self.user).field("domain", &self.domain).field("password", &self.password.as_ref().map(|_| "<hidden>")).finish()
    }
}

impl Credentials {
    /// Credentials for the user and password of an address, filling the user and domain from the address when the
    /// caller has none.
    pub fn for_url(url: &SmbUrl, password: Option<String>) -> Credentials {
        Credentials { user: url.user.clone(), domain: url.domain.clone(), password }
    }

    /// The environment the client reads the password from. Empty for a guest or when there is no password.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        match (&self.user, &self.password) {
            (Some(_), Some(p)) => vec![("PASSWD", p.clone())],
            _ => Vec::new(),
        }
    }
}

/// The arguments every call starts with: program, port, protocol floor, encryption and login.
fn base(url: &SmbUrl, creds: &Credentials) -> Vec<String> {
    let mut v = vec![
        SMBCLIENT.to_string(),
        "-p".to_string(),
        url.port.to_string(),
        format!("--option=client min protocol={MIN_PROTOCOL}"),
        "--option=client smb encrypt=desired".to_string(),
    ];
    match (&creds.user, &creds.password) {
        (Some(user), Some(_)) => {
            v.push("-U".to_string());
            v.push(match &creds.domain {
                Some(d) => format!("{d}\\{user}"),
                None => user.clone(),
            });
        }
        (Some(user), None) => {
            // a user with no password: never prompt, there is no terminal
            v.push("-U".to_string());
            v.push(user.clone());
            v.push("-N".to_string());
        }
        _ => v.push("-N".to_string()),
    }
    v
}

fn share_of(url: &SmbUrl) -> Result<&str, Error> {
    url.share.as_deref().ok_or(Error::Missing("shared folder"))
}

/// The command line that lists the shares a server offers, in the client's grep-friendly format.
pub fn list_shares_argv(url: &SmbUrl, creds: &Credentials) -> Vec<String> {
    let mut v = base(url, creds);
    v.push("-g".to_string());
    v.push("-L".to_string());
    v.push(url.host.clone());
    v
}

/// The command line that lists the folder the address names (the share root when it names only a share).
pub fn list_dir_argv(url: &SmbUrl, creds: &Credentials) -> Result<Vec<String>, Error> {
    let share = share_of(url)?;
    let mut v = base(url, creds);
    v.push(format!("//{}/{}", url.host, share));
    v.push("-c".to_string());
    v.push(if url.path.is_empty() { "ls".to_string() } else { format!("cd \"{}\"; ls", url.path) });
    Ok(v)
}

/// The command line that downloads the file the address names to `local`. `local` may not hold a quote, a `;` or a
/// control character.
pub fn get_argv(url: &SmbUrl, creds: &Credentials, local: &str) -> Result<Vec<String>, Error> {
    let share = share_of(url)?;
    if url.path.is_empty() {
        return Err(Error::Missing("file"));
    }
    if local.is_empty() || local.chars().any(|c| c.is_control() || matches!(c, '"' | ';')) {
        return Err(Error::BadName);
    }
    let mut v = base(url, creds);
    v.push(format!("//{}/{}", url.host, share));
    v.push("-c".to_string());
    v.push(format!("get \"{}\" \"{}\"", url.path, local));
    Ok(v)
}

/// What a share is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareKind {
    Disk,
    Printer,
    /// The inter-process pipe share (`IPC$`) and serial devices; not browsable.
    Other,
}

/// One share of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub kind: ShareKind,
    pub comment: String,
}

impl Share {
    /// A hidden share: its name ends with `$` (administrative shares and the pipe share).
    pub fn is_hidden(&self) -> bool {
        self.name.ends_with('$')
    }
}

/// The shares in the output of `smbclient -g -L`. Lines are `Disk|name|comment`; the workgroup and server lines the
/// client adds are skipped.
pub fn parse_shares(text: &str) -> Vec<Share> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            let kind = match parts.next()? {
                "Disk" => ShareKind::Disk,
                "Printer" => ShareKind::Printer,
                "IPC" | "Comm" => ShareKind::Other,
                _ => return None,
            };
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(Share { name: name.to_string(), kind, comment: parts.next().unwrap_or("").trim().to_string() })
        })
        .collect()
}

/// One folder or file in a share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// The attribute letters the server reported (`D` folder, `A` archive, `H` hidden, `S` system, `R` read only, `N` none).
    pub attributes: String,
    /// The modified time as the client printed it, for example `Sun Mar  3 10:11:12 2024`.
    pub modified: String,
}

impl Entry {
    /// A name that starts with a dot or an entry flagged hidden.
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.') || self.attributes.contains('H')
    }
}

/// The entries in the output of an `ls` run by `smbclient`. A line is `  name  ATTRS  size  Day Mon dd hh:mm:ss yyyy`;
/// the name may hold spaces. `.` and `..`, the free-space line and anything else are skipped.
pub fn parse_listing(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.starts_with("  ") {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let n = tokens.len();
        if n < 8 {
            continue;
        }
        let (day, month, dom, time, year) = (tokens[n - 5], tokens[n - 4], tokens[n - 3], tokens[n - 2], tokens[n - 1]);
        let date_ok = day.len() == 3
            && month.len() == 3
            && day.bytes().all(|b| b.is_ascii_alphabetic())
            && month.bytes().all(|b| b.is_ascii_alphabetic())
            && !dom.is_empty()
            && dom.bytes().all(|b| b.is_ascii_digit())
            && time.contains(':')
            && year.len() == 4
            && year.bytes().all(|b| b.is_ascii_digit());
        if !date_ok {
            continue;
        }
        let Ok(size) = tokens[n - 6].parse::<u64>() else { continue };
        let attributes = tokens[n - 7];
        if !attributes.bytes().all(|b| b"DAHSRNdahsrn".contains(&b)) {
            continue;
        }
        // the name is everything before the attribute token, spaces inside it kept
        let start = attributes.as_ptr() as usize - line.as_ptr() as usize;
        let name = line[..start].trim();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        out.push(Entry {
            name: name.to_string(),
            is_dir: attributes.chars().any(|c| c == 'D' || c == 'd'),
            size,
            attributes: attributes.to_ascii_uppercase(),
            modified: tokens[n - 5..].join(" "),
        });
    }
    out
}

/// The error a client's output stands for, from its `NT_STATUS_*` name. `None` when it names no status.
pub fn classify(output: &str) -> Option<Error> {
    let at = output.find("NT_STATUS_")?;
    let status: String = output[at..].chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
    Some(match status.as_str() {
        "NT_STATUS_LOGON_FAILURE" | "NT_STATUS_WRONG_PASSWORD" | "NT_STATUS_ACCOUNT_DISABLED" | "NT_STATUS_ACCOUNT_LOCKED_OUT" | "NT_STATUS_PASSWORD_EXPIRED" | "NT_STATUS_NO_SUCH_USER" => Error::AuthFailed,
        "NT_STATUS_ACCESS_DENIED" => Error::AccessDenied,
        "NT_STATUS_BAD_NETWORK_NAME" => Error::NoSuchShare,
        "NT_STATUS_OBJECT_NAME_NOT_FOUND" | "NT_STATUS_OBJECT_PATH_NOT_FOUND" | "NT_STATUS_NO_SUCH_FILE" => Error::NoSuchPath,
        "NT_STATUS_CONNECTION_REFUSED" => Error::Refused,
        "NT_STATUS_HOST_UNREACHABLE" | "NT_STATUS_NETWORK_UNREACHABLE" | "NT_STATUS_UNSUCCESSFUL" | "NT_STATUS_CONNECTION_DISCONNECTED" | "NT_STATUS_CONNECTION_RESET" => Error::Unreachable,
        "NT_STATUS_IO_TIMEOUT" => Error::TimedOut,
        _ => Error::Failed(status),
    })
}

/// Runs `argv` with `env` added, waits at most `timeout`, and returns its standard output. A non-zero exit becomes
/// the error its output stands for ([`classify`]) or [`Error::Failed`] with its first line.
pub fn run(argv: &[String], env: &[(&str, String)], timeout: Duration) -> Result<String, Error> {
    let (program, args) = argv.split_first().ok_or(Error::Failed("no command".to_string()))?;
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        command.env(k, v);
    }
    let mut child = command.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::NoClient } else { Error::Failed(e.to_string()) })?;
    let mut out = child.stdout.take().ok_or_else(|| Error::Failed("no output pipe".to_string()))?;
    let mut err = child.stderr.take().ok_or_else(|| Error::Failed("no error pipe".to_string()))?;
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::TimedOut);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(Error::Failed(e.to_string())),
        }
    };
    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if status.success() {
        return Ok(stdout);
    }
    let combined = format!("{stdout}\n{stderr}");
    Err(classify(&combined).unwrap_or_else(|| Error::Failed(combined.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("exit status").to_string())))
}

/// The shares the server in `url` offers, without the hidden ones.
pub fn list_shares(url: &SmbUrl, creds: &Credentials) -> Result<Vec<Share>, Error> {
    let text = run(&list_shares_argv(url, creds), &creds.env(), TIMEOUT)?;
    Ok(parse_shares(&text).into_iter().filter(|s| !s.is_hidden()).collect())
}

/// The entries of the folder `url` names.
pub fn list_dir(url: &SmbUrl, creds: &Credentials) -> Result<Vec<Entry>, Error> {
    let text = run(&list_dir_argv(url, creds)?, &creds.env(), TIMEOUT)?;
    if let Some(e) = classify(&text) {
        return Err(e);
    }
    Ok(parse_listing(&text))
}

/// Downloads the file `url` names to `local`.
pub fn get(url: &SmbUrl, creds: &Credentials, local: &str) -> Result<(), Error> {
    let text = run(&get_argv(url, creds, local)?, &creds.env(), TIMEOUT)?;
    match classify(&text) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> SmbUrl {
        SmbUrl::parse(s).unwrap()
    }

    #[test]
    fn an_address_is_split_into_its_parts() {
        let u = url("SMB://WORK;ann@Files.local:4455/Photos/2024/summer%20trip");
        assert_eq!(u.host, "files.local");
        assert_eq!(u.port, 4455);
        assert_eq!(u.user.as_deref(), Some("ann"));
        assert_eq!(u.domain.as_deref(), Some("WORK"));
        assert_eq!(u.share.as_deref(), Some("Photos"));
        assert_eq!(u.path, "2024/summer trip");
    }

    #[test]
    fn a_bare_server_has_no_share_and_the_default_port() {
        let u = url("smb://nas");
        assert_eq!((u.port, u.share, u.path.as_str(), u.user), (DEFAULT_PORT, None, "", None));
    }

    #[test]
    fn a_password_in_the_address_is_refused() {
        assert_eq!(SmbUrl::parse("smb://ann:secret@nas/Docs"), Err(Error::PasswordInUrl));
    }

    #[test]
    fn bad_addresses_are_refused() {
        assert_eq!(SmbUrl::parse("ftp://nas"), Err(Error::Scheme));
        assert_eq!(SmbUrl::parse("smb://"), Err(Error::EmptyHost));
        assert_eq!(SmbUrl::parse("smb://na s/x"), Err(Error::BadHost));
        assert_eq!(SmbUrl::parse("smb://nas:0/x"), Err(Error::BadPort));
        assert_eq!(SmbUrl::parse("smb://nas:99999/x"), Err(Error::BadPort));
        assert_eq!(SmbUrl::parse("smb://nas/a%zz"), Err(Error::BadEscape));
    }

    #[test]
    fn names_that_could_split_a_client_command_are_refused() {
        for bad in ["smb://nas/share/..%2F", "smb://nas/share/a%3Bb", "smb://nas/share/%22x", "smb://nas/share/..", "smb://nas/share/a%2Ab", "smb://nas/share/a%0Ab", "smb://nas/sh%3Bare"] {
            assert_eq!(SmbUrl::parse(bad), Err(Error::BadName), "{bad}");
        }
    }

    #[test]
    fn an_address_round_trips() {
        for text in ["smb://nas", "smb://nas/Docs", "smb://WORK;ann@nas:4455/Docs/a%20b/c.txt", "smb://ann@nas/Docs"] {
            assert_eq!(url(&url(text).to_text()), url(text), "{text}");
        }
        assert_eq!(url("smb://ann@nas:445/Docs").to_text(), "smb://ann@nas/Docs");
    }

    #[test]
    fn the_password_goes_in_the_environment_and_never_in_the_arguments() {
        let u = url("smb://WORK;ann@nas/Docs/Reports");
        let creds = Credentials::for_url(&u, Some("hunter2".to_string()));
        assert_eq!(creds.env(), vec![("PASSWD", "hunter2".to_string())]);
        let argv = list_dir_argv(&u, &creds).unwrap();
        assert!(argv.iter().all(|a| !a.contains("hunter2")));
        assert!(argv.windows(2).any(|w| w[0] == "-U" && w[1] == "WORK\\ann"));
        assert_eq!(argv.last().unwrap(), "cd \"Reports\"; ls");
        assert!(!format!("{creds:?}").contains("hunter2"));
    }

    #[test]
    fn the_client_is_held_to_smb2_and_never_prompts() {
        let u = url("smb://nas");
        let guest = list_shares_argv(&u, &Credentials::default());
        assert!(guest.contains(&format!("--option=client min protocol={MIN_PROTOCOL}")));
        assert!(guest.contains(&"--option=client smb encrypt=desired".to_string()));
        assert!(guest.contains(&"-N".to_string()), "a guest login sends no password");
        assert_eq!(&guest[guest.len() - 3..], ["-g", "-L", "nas"]);
        let no_password = base(&u, &Credentials { user: Some("ann".into()), ..Credentials::default() });
        assert!(no_password.contains(&"-N".to_string()));
        assert!(Credentials { user: Some("ann".into()), ..Credentials::default() }.env().is_empty());
    }

    #[test]
    fn listing_and_download_need_a_share_and_a_file() {
        let creds = Credentials::default();
        assert_eq!(list_dir_argv(&url("smb://nas"), &creds), Err(Error::Missing("shared folder")));
        assert_eq!(get_argv(&url("smb://nas/Docs"), &creds, "/tmp/x"), Err(Error::Missing("file")));
        assert_eq!(get_argv(&url("smb://nas/Docs/a.txt"), &creds, "/tmp/x\";rm"), Err(Error::BadName));
        let argv = get_argv(&url("smb://nas/Docs/a b/c.txt"), &creds, "/tmp/c.txt").unwrap();
        assert_eq!(argv.last().unwrap(), "get \"a b/c.txt\" \"/tmp/c.txt\"");
        assert_eq!(list_dir_argv(&url("smb://nas/Docs"), &creds).unwrap().last().unwrap(), "ls");
    }

    #[test]
    fn shares_are_read_from_the_grep_friendly_listing() {
        let text = "Disk|Docs|Team documents\nDisk|Photos|\nPrinter|Office|Front desk\nIPC|IPC$|IPC Service\nDisk|C$|Default share\nReconnecting with SMB1 for workgroup listing.\nWorkgroup|WORKGROUP|NAS\n";
        let shares = parse_shares(text);
        assert_eq!(shares.len(), 5);
        assert_eq!(shares[0], Share { name: "Docs".into(), kind: ShareKind::Disk, comment: "Team documents".into() });
        assert_eq!(shares[2].kind, ShareKind::Printer);
        assert!(shares[3].is_hidden() && shares[4].is_hidden());
        assert!(!shares[0].is_hidden());
    }

    #[test]
    fn a_folder_listing_is_read_with_spaces_in_names() {
        let text = "  .                                   D        0  Sun Mar  3 10:11:12 2024\n  ..                                  D        0  Sun Mar  3 10:11:12 2024\n  Summer trip 2024                    D        0  Mon Jul  1 09:00:00 2024\n  report  final.pdf                   A   482113  Tue Jul  2 14:30:05 2024\n  .hidden                            AH       12  Tue Jul  2 14:30:05 2024\n\n\t\t97910528 blocks of size 1024. 52436512 blocks available\n";
        let entries = parse_listing(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "Summer trip 2024");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "report  final.pdf");
        assert_eq!(entries[1].size, 482113);
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].modified, "Tue Jul 2 14:30:05 2024");
        assert!(entries[2].is_hidden());
    }

    #[test]
    fn lines_that_are_not_entries_are_skipped() {
        assert!(parse_listing("NT_STATUS_ACCESS_DENIED listing \\*\n  nonsense line\n").is_empty());
        assert!(parse_listing("").is_empty());
    }

    #[test]
    fn client_statuses_become_errors() {
        assert_eq!(classify("session setup failed: NT_STATUS_LOGON_FAILURE"), Some(Error::AuthFailed));
        assert_eq!(classify("tree connect failed: NT_STATUS_BAD_NETWORK_NAME"), Some(Error::NoSuchShare));
        assert_eq!(classify("NT_STATUS_ACCESS_DENIED listing \\*"), Some(Error::AccessDenied));
        assert_eq!(classify("cd \\x: NT_STATUS_OBJECT_NAME_NOT_FOUND"), Some(Error::NoSuchPath));
        assert_eq!(classify("Connection to nas failed (Error NT_STATUS_HOST_UNREACHABLE)"), Some(Error::Unreachable));
        assert_eq!(classify("NT_STATUS_CONNECTION_REFUSED"), Some(Error::Refused));
        assert_eq!(classify("NT_STATUS_IO_TIMEOUT"), Some(Error::TimedOut));
        assert_eq!(classify("NT_STATUS_DISK_FULL)"), Some(Error::Failed("NT_STATUS_DISK_FULL".into())));
        assert_eq!(classify("all fine"), None);
    }

    #[test]
    fn running_a_program_returns_its_output_or_why_it_failed() {
        let out = run(&["echo".to_string(), "hi".to_string()], &[], Duration::from_secs(5)).unwrap();
        assert_eq!(out, "hi\n");
        assert!(matches!(run(&["false".to_string()], &[], Duration::from_secs(5)), Err(Error::Failed(_))));
        assert_eq!(run(&["/nonexistent/smbclient-for-tests".to_string()], &[], Duration::from_secs(5)), Err(Error::NoClient));
        assert_eq!(run(&[], &[], Duration::from_secs(5)), Err(Error::Failed("no command".to_string())));
    }

    #[test]
    fn a_call_that_takes_too_long_is_stopped() {
        assert_eq!(run(&["sleep".to_string(), "5".to_string()], &[], Duration::from_millis(100)), Err(Error::TimedOut));
    }

    #[test]
    fn the_environment_reaches_the_program() {
        let out = run(&["printenv".to_string(), "PASSWD".to_string()], &[("PASSWD", "s3cret".to_string())], Duration::from_secs(5)).unwrap();
        assert_eq!(out.trim(), "s3cret");
    }
}
