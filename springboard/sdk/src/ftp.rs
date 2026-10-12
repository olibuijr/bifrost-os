//! FTP model for apps: ftp:// URLs, reply parsing, passive-mode addresses, command lines and directory listings.
//!
//! Everything here is pure std and opens no socket. A caller that runs its own control and data connections sends
//! [`Command::to_line`], reads the reply text and passes it to [`Reply::parse`]. A passive transfer takes its data
//! address from [`pasv_address`] or [`epsv_port`]. A directory listing read over the data connection goes through
//! [`parse_listing`], which reads the Unix long format most FTP servers send. The reference framework marks its FTP
//! calls deprecated in favour of the URL loading API, which has no FTP support here yet, so this model is for legacy
//! file servers only.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};

/// The control port of an FTP server when a URL names none (RFC 959).
pub const DEFAULT_PORT: u16 = 21;

/// Why a URL, reply, passive address or command was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The text does not start with ftp:// (the scheme is compared without case).
    Scheme,
    /// The URL has no host name before the port or path.
    EmptyHost,
    /// The port is not a number from 1 to 65535.
    BadPort,
    /// A percent escape is not two hex digits, or the decoded bytes are not UTF-8.
    BadEscape,
    /// A reply line does not start with a three-digit code.
    BadReply,
    /// A multi-line reply never reaches a closing line with the same code.
    Unterminated,
    /// A command argument carries a line break or a NUL byte, which would split the control line.
    BadArgument,
    /// The reply is not a 227 (PASV) or 229 (EPSV) answer with a usable address.
    BadPassive,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::Scheme => "URL must start with ftp://",
            Error::EmptyHost => "URL has no host name",
            Error::BadPort => "port must be from 1 to 65535",
            Error::BadEscape => "bad percent escape in URL",
            Error::BadReply => "reply line has no three-digit code",
            Error::Unterminated => "multi-line reply has no closing line",
            Error::BadArgument => "command argument contains a line break or NUL",
            Error::BadPassive => "reply is not a usable passive-mode answer",
        };
        f.write_str(text)
    }
}

/// A parsed ftp:// URL. Escapes in the user, password and path are decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpUrl {
    pub user: Option<String>,
    pub password: Option<String>,
    /// A host name or an IPv4 address. Bracketed IPv6 literals are not handled.
    pub host: String,
    pub port: u16,
    /// The decoded path, "/" when the URL has none.
    pub path: String,
}

impl FtpUrl {
    /// Parses `ftp://[user[:password]@]host[:port][/path]`.
    pub fn parse(text: &str) -> Result<FtpUrl, Error> {
        let rest = strip_prefix_ci(text, "ftp://").ok_or(Error::Scheme)?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
            None => (None, authority),
        };
        let (user, password) = match userinfo {
            Some(info) => match info.find(':') {
                Some(i) => (Some(percent_decode(&info[..i])?), Some(percent_decode(&info[i + 1..])?)),
                None => (Some(percent_decode(info)?), None),
            },
            None => (None, None),
        };
        let (host, port) = match hostport.rfind(':') {
            Some(i) => (&hostport[..i], parse_port(&hostport[i + 1..])?),
            None => (hostport, DEFAULT_PORT),
        };
        if host.is_empty() {
            return Err(Error::EmptyHost);
        }
        Ok(FtpUrl { user, password, host: host.to_string(), port, path: percent_decode(path)? })
    }

    /// True when the path names a directory (it ends with a slash).
    pub fn is_directory(&self) -> bool {
        self.path.ends_with('/')
    }
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() >= prefix.len() && text.is_char_boundary(prefix.len()) && text[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

fn parse_port(text: &str) -> Result<u16, Error> {
    match text.parse::<u16>() {
        Ok(p) if p != 0 => Ok(p),
        _ => Err(Error::BadPort),
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes `%XX` escapes. A `%` must be followed by two hex digits.
fn percent_decode(text: &str) -> Result<String, Error> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(Error::BadEscape);
            }
            let hi = hex_val(bytes[i + 1]).ok_or(Error::BadEscape)?;
            let lo = hex_val(bytes[i + 2]).ok_or(Error::BadEscape)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| Error::BadEscape)
}

/// The class of a reply, from its first digit (RFC 959 section 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// 1yz: the request is accepted and another reply follows.
    Preliminary,
    /// 2yz: the request completed.
    Completion,
    /// 3yz: the request needs more information.
    Intermediate,
    /// 4yz: the request failed for now; it can be retried.
    TransientNegative,
    /// 5yz: the request failed and should not be retried unchanged.
    PermanentNegative,
}

/// A server reply: its three-digit code and the text of each line, without the code and separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    pub lines: Vec<String>,
}

fn parse_code(line: &str) -> Result<u16, Error> {
    let b = line.as_bytes();
    if b.len() < 3 || !b[..3].iter().all(|c| c.is_ascii_digit()) {
        return Err(Error::BadReply);
    }
    Ok(((b[0] - b'0') as u16) * 100 + ((b[1] - b'0') as u16) * 10 + (b[2] - b'0') as u16)
}

/// The text after `code` and its separator. A line shorter than the separator gives an empty string.
fn text_after(line: &str) -> String {
    line.get(4..).unwrap_or("").to_string()
}

impl Reply {
    /// Parses the text of one reply. A line `220-text` opens a multi-line reply that ends with `220 text`.
    /// Empty lines are dropped. Both CRLF and LF line ends are accepted.
    pub fn parse(text: &str) -> Result<Reply, Error> {
        let all: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()).collect();
        let first = *all.first().ok_or(Error::BadReply)?;
        let code = parse_code(first)?;
        if !matches!(first.as_bytes().get(3), None | Some(&b' ') | Some(&b'-')) {
            return Err(Error::BadReply);
        }
        if first.as_bytes().get(3) != Some(&b'-') {
            return Ok(Reply { code, lines: vec![text_after(first)] });
        }
        let mut lines = vec![text_after(first)];
        for line in &all[1..] {
            let b = line.as_bytes();
            if parse_code(line).ok() == Some(code) && (b.len() == 3 || b[3] == b' ') {
                lines.push(text_after(line));
                return Ok(Reply { code, lines });
            }
            lines.push(line.to_string());
        }
        Err(Error::Unterminated)
    }

    /// The class of the reply, or `None` for a first digit outside 1 to 5.
    pub fn class(&self) -> Option<Class> {
        match self.code / 100 {
            1 => Some(Class::Preliminary),
            2 => Some(Class::Completion),
            3 => Some(Class::Intermediate),
            4 => Some(Class::TransientNegative),
            5 => Some(Class::PermanentNegative),
            _ => None,
        }
    }
}

/// The data address in a 227 reply, `Entering Passive Mode (h1,h2,h3,h4,p1,p2)`. The port is p1 * 256 + p2.
pub fn pasv_address(reply: &Reply) -> Result<SocketAddrV4, Error> {
    if reply.code != 227 {
        return Err(Error::BadPassive);
    }
    let text = reply.lines.join(" ");
    let open = text.find('(').ok_or(Error::BadPassive)?;
    let close = text[open..].find(')').ok_or(Error::BadPassive)? + open;
    let parts: Result<Vec<u16>, Error> = text[open + 1..close]
        .split(',')
        .map(|p| p.trim().parse::<u16>().map_err(|_| Error::BadPassive))
        .collect();
    let nums = parts?;
    if nums.len() != 6 || nums.iter().any(|&n| n > 255) {
        return Err(Error::BadPassive);
    }
    let ip = Ipv4Addr::new(nums[0] as u8, nums[1] as u8, nums[2] as u8, nums[3] as u8);
    let port = nums[4] * 256 + nums[5];
    if port == 0 {
        return Err(Error::BadPassive);
    }
    Ok(SocketAddrV4::new(ip, port))
}

/// The data port in a 229 reply, `Entering Extended Passive Mode (|||port|)`. The delimiter is any one character,
/// repeated three times before the port and once after it (RFC 2428 section 3).
pub fn epsv_port(reply: &Reply) -> Result<u16, Error> {
    if reply.code != 229 {
        return Err(Error::BadPassive);
    }
    let text = reply.lines.join(" ");
    let open = text.find('(').ok_or(Error::BadPassive)?;
    let body = text[open + 1..].trim_end_matches(')');
    let d = body.chars().next().ok_or(Error::BadPassive)?;
    let parts: Vec<&str> = body.split(d).collect();
    if parts.len() != 5 || !parts[0].is_empty() || !parts[1].is_empty() || !parts[2].is_empty() || !parts[4].is_empty() {
        return Err(Error::BadPassive);
    }
    match parts[3].parse::<u16>() {
        Ok(p) if p != 0 => Ok(p),
        _ => Err(Error::BadPassive),
    }
}

/// One control command. The argument is sent as given after a single space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command<'a> {
    User(&'a str),
    Pass(&'a str),
    /// TYPE A, ASCII transfers.
    TypeAscii,
    /// TYPE I, binary transfers.
    TypeImage,
    Passive,
    ExtendedPassive,
    Cwd(&'a str),
    ParentDir,
    /// LIST for a path. An empty path lists the current directory.
    List(&'a str),
    Retr(&'a str),
    Stor(&'a str),
    Dele(&'a str),
    Quit,
}

impl Command<'_> {
    /// The control line, with its CRLF. An argument with CR, LF or NUL is refused, because it would start a second
    /// command on the control connection.
    pub fn to_line(&self) -> Result<String, Error> {
        let (verb, arg): (&str, Option<&str>) = match *self {
            Command::User(a) => ("USER", Some(a)),
            Command::Pass(a) => ("PASS", Some(a)),
            Command::TypeAscii => ("TYPE", Some("A")),
            Command::TypeImage => ("TYPE", Some("I")),
            Command::Passive => ("PASV", None),
            Command::ExtendedPassive => ("EPSV", None),
            Command::Cwd(a) => ("CWD", Some(a)),
            Command::ParentDir => ("CDUP", None),
            Command::List(a) => ("LIST", Some(a)),
            Command::Retr(a) => ("RETR", Some(a)),
            Command::Stor(a) => ("STOR", Some(a)),
            Command::Dele(a) => ("DELE", Some(a)),
            Command::Quit => ("QUIT", None),
        };
        let mut line = verb.to_string();
        if let Some(a) = arg {
            if a.bytes().any(|c| c == b'\r' || c == b'\n' || c == 0) {
                return Err(Error::BadArgument);
            }
            if !a.is_empty() {
                line.push(' ');
                line.push_str(a);
            }
        }
        line.push_str("\r\n");
        Ok(line)
    }
}

/// The kind of a listed item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    /// A symbolic link and the target after ` -> ` (empty when the server sent none).
    Symlink(String),
    /// A device, pipe or socket.
    Other,
}

/// The modification time of a listed item. The server gives either a time of day, for recent items, or a year.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// 1 for January to 12 for December.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    pub year: Option<u16>,
    /// Hour and minute, 0 to 23 and 0 to 59.
    pub time: Option<(u8, u8)>,
}

/// One item of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// Permission bits in octal layout, owner in bits 6 to 8, group in bits 3 to 5 and other in bits 0 to 2.
    pub mode: u16,
    pub owner: String,
    pub group: String,
    pub modified: Stamp,
}

/// Parses a directory listing in the Unix long format, one item per line. Lines that do not match the format, such
/// as the `total` line, are skipped, and so are the `.` and `..` entries. The name is the rest of the line, so it may
/// contain spaces.
pub fn parse_listing(text: &str) -> Vec<Entry> {
    text.lines().filter_map(parse_line).filter(|e| e.name != "." && e.name != "..").collect()
}

/// Splits off the first `n` whitespace-separated fields and returns them with the rest of the line.
fn split_fields(line: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut fields = Vec::with_capacity(n);
    let mut rest = line.trim_start();
    for _ in 0..n {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        fields.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some((fields, rest))
}

fn parse_mode(bits: &str) -> Option<u16> {
    if bits.len() != 9 {
        return None;
    }
    let chars: Vec<char> = bits.chars().collect();
    let mut mode = 0u16;
    for (triplet, group) in chars.chunks(3).enumerate() {
        let r = group[0] == 'r';
        let w = group[1] == 'w';
        let x = matches!(group[2], 'x' | 's' | 't');
        let value = (if r { 4 } else { 0 }) + (if w { 2 } else { 0 }) + (if x { 1 } else { 0 });
        mode |= (value as u16) << (6 - 3 * triplet as u16);
    }
    Some(mode)
}

fn month_number(name: &str) -> Option<u8> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    MONTHS.iter().position(|m| *m == name).map(|i| i as u8 + 1)
}

fn parse_line(line: &str) -> Option<Entry> {
    let (fields, rest) = split_fields(line, 8)?;
    let perms = fields[0];
    if perms.len() != 10 || rest.is_empty() {
        return None;
    }
    let kind_byte = perms.as_bytes()[0];
    let mode = parse_mode(&perms[1..])?;
    let size: u64 = fields[4].parse().ok()?;
    let month = month_number(fields[5])?;
    let day: u8 = fields[6].parse().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let (year, time) = if fields[7].contains(':') {
        let (h, m) = fields[7].split_once(':')?;
        let h: u8 = h.parse().ok()?;
        let m: u8 = m.parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        (None, Some((h, m)))
    } else {
        (Some(fields[7].parse::<u16>().ok()?), None)
    };
    let (name, kind) = match kind_byte {
        b'd' => (rest.to_string(), Kind::Directory),
        b'-' => (rest.to_string(), Kind::File),
        b'l' => match rest.split_once(" -> ") {
            Some((n, target)) => (n.to_string(), Kind::Symlink(target.to_string())),
            None => (rest.to_string(), Kind::Symlink(String::new())),
        },
        b'c' | b'b' | b'p' | b's' => (rest.to_string(), Kind::Other),
        _ => return None,
    };
    Some(Entry {
        name,
        kind,
        size,
        mode,
        owner: fields[2].to_string(),
        group: fields[3].to_string(),
        modified: Stamp { month, day, year, time },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_with_user_password_port_and_escaped_path() {
        let u = FtpUrl::parse("ftp://anna:s%40cret@files.example.is:2121/pub/My%20Docs/").unwrap();
        assert_eq!(u.user.as_deref(), Some("anna"));
        assert_eq!(u.password.as_deref(), Some("s@cret"));
        assert_eq!(u.host, "files.example.is");
        assert_eq!(u.port, 2121);
        assert_eq!(u.path, "/pub/My Docs/");
        assert!(u.is_directory());
    }

    #[test]
    fn url_defaults_port_and_root_path_and_scheme_is_case_free() {
        let u = FtpUrl::parse("FTP://files.example.is").unwrap();
        assert_eq!(u.user, None);
        assert_eq!(u.port, DEFAULT_PORT);
        assert_eq!(u.path, "/");
        assert_eq!(FtpUrl::parse("http://files.example.is"), Err(Error::Scheme));
        assert_eq!(FtpUrl::parse("ftp:///pub"), Err(Error::EmptyHost));
        assert_eq!(FtpUrl::parse("ftp://h:0/"), Err(Error::BadPort));
        assert_eq!(FtpUrl::parse("ftp://h/a%2"), Err(Error::BadEscape));
    }

    #[test]
    fn single_line_and_multi_line_replies() {
        let r = Reply::parse("220 Ready\r\n").unwrap();
        assert_eq!(r.code, 220);
        assert_eq!(r.lines, vec!["Ready".to_string()]);
        assert_eq!(r.class(), Some(Class::Completion));

        let m = Reply::parse("220-Welcome\r\n220 Ready\r\n").unwrap();
        assert_eq!(m.code, 220);
        assert_eq!(m.lines, vec!["Welcome".to_string(), "Ready".to_string()]);

        assert_eq!(Reply::parse("150-one\r\n151 two\r\n"), Err(Error::Unterminated));
        assert_eq!(Reply::parse("ok\r\n"), Err(Error::BadReply));
        assert_eq!(Reply::parse("150 x").unwrap().class(), Some(Class::Preliminary));
        assert_eq!(Reply::parse("331 x").unwrap().class(), Some(Class::Intermediate));
        assert_eq!(Reply::parse("530 x").unwrap().class(), Some(Class::PermanentNegative));
    }

    #[test]
    fn pasv_reply_gives_host_and_port() {
        // Port 195 * 256 + 80 = 49920 + 80 = 50000.
        let r = Reply::parse("227 Entering Passive Mode (193,4,10,2,195,80).\r\n").unwrap();
        assert_eq!(pasv_address(&r), Ok(SocketAddrV4::new(Ipv4Addr::new(193, 4, 10, 2), 50000)));
        let wrong = Reply::parse("200 ok (1,2,3,4,5,6)\r\n").unwrap();
        assert_eq!(pasv_address(&wrong), Err(Error::BadPassive));
        let short = Reply::parse("227 (1,2,3,4,5)\r\n").unwrap();
        assert_eq!(pasv_address(&short), Err(Error::BadPassive));
    }

    #[test]
    fn epsv_reply_gives_the_port() {
        let r = Reply::parse("229 Entering Extended Passive Mode (|||6446|)\r\n").unwrap();
        assert_eq!(epsv_port(&r), Ok(6446));
        let bad = Reply::parse("229 Entering Extended Passive Mode (|||0|)\r\n").unwrap();
        assert_eq!(epsv_port(&bad), Err(Error::BadPassive));
    }

    #[test]
    fn command_lines_and_argument_check() {
        assert_eq!(Command::User("anna").to_line().unwrap(), "USER anna\r\n");
        assert_eq!(Command::TypeImage.to_line().unwrap(), "TYPE I\r\n");
        assert_eq!(Command::Passive.to_line().unwrap(), "PASV\r\n");
        assert_eq!(Command::List("").to_line().unwrap(), "LIST\r\n");
        assert_eq!(Command::Cwd("a\r\nQUIT").to_line(), Err(Error::BadArgument));
        assert_eq!(Command::Retr("a\0b").to_line(), Err(Error::BadArgument));
    }

    #[test]
    fn unix_listing_lines_become_entries() {
        let text = "total 8\r\n\
drwxr-xr-x    2 ftp      ftp          4096 Jan 12 10:30 My Docs\r\n\
-rw-r--r--    1 ftp      ftp          1234 Mar  3  2025 notes.txt\r\n\
lrwxrwxrwx    1 ftp      ftp             7 Jan  1 00:00 latest -> 2025\r\n\
drwxr-xr-x    2 ftp      ftp          4096 Jan  1 00:00 .\r\n";
        let entries = parse_listing(text);
        assert_eq!(entries.len(), 3);

        let dir = &entries[0];
        assert_eq!(dir.name, "My Docs");
        assert_eq!(dir.kind, Kind::Directory);
        assert_eq!(dir.size, 4096);
        assert_eq!(dir.mode, 0o755);
        assert_eq!(dir.owner, "ftp");
        assert_eq!(dir.modified, Stamp { month: 1, day: 12, year: None, time: Some((10, 30)) });

        let file = &entries[1];
        assert_eq!(file.kind, Kind::File);
        assert_eq!(file.size, 1234);
        assert_eq!(file.mode, 0o644);
        assert_eq!(file.modified, Stamp { month: 3, day: 3, year: Some(2025), time: None });

        let link = &entries[2];
        assert_eq!(link.name, "latest");
        assert_eq!(link.kind, Kind::Symlink("2025".to_string()));
        assert_eq!(link.mode, 0o777);
    }
}
