//! LDAP directory nodes filled from a server through the OpenLDAP client, `ldapsearch` (the `openldap` package).
//! A search runs `ldapsearch -LLL -x -H <uri> -b <base> -s sub [-D <bind dn> -y <password file>] <filter> <attributes...>`,
//! reads the LDIF it prints, and turns each entry into a [`DirRecord`]. [`load`] puts those records into a [`Session`]
//! node, so the directory query model in `bifrost_ui::directory_query` runs on them like any other directory.
//!
//! Binds are simple (name and password). The password never goes on the command line: it is written to a private file
//! (mode 0600, in `$XDG_RUNTIME_DIR` or the temporary directory) that `ldapsearch -y` reads, and the file is removed
//! when the search ends. Without a bind the search is anonymous. The tool reads its own configuration
//! (`/etc/openldap/ldap.conf`) for TLS. The command runs without a shell.
//! LDIF attributes and DNs written as base64 (`attr:: ...`) are decoded; a value that is not UTF-8 text is skipped.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use bifrost_ui::directory_node::{Error as NodeError, Session};
use bifrost_ui::directory_query::{DirRecord, NAME_ATTRIBUTE};

/// Why a search or a load failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LdapError {
    /// The URI does not start with `ldap://` or `ldaps://`, or holds whitespace.
    BadUri,
    /// The search base is empty or holds a line break.
    BadBaseDn,
    /// The filter is empty or does not start with `(`.
    BadFilter,
    /// The bind DN or password is empty or holds a line break, or a bind has no password file.
    BadCredentials,
    /// The password file could not be written or removed.
    Io(String),
    /// `ldapsearch` could not be started (the `openldap` package is missing).
    Spawn(String),
    /// `ldapsearch` ran and failed; the text is its standard error.
    Failed(String),
    /// The target node does not exist in the session.
    Node(NodeError),
}

impl fmt::Display for LdapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LdapError::BadUri => f.write_str("LDAP URI must start with ldap:// or ldaps:// and hold no spaces"),
            LdapError::BadBaseDn => f.write_str("LDAP search base is empty"),
            LdapError::BadFilter => f.write_str("LDAP filter must start with ("),
            LdapError::BadCredentials => f.write_str("LDAP bind DN and password must be non-empty single lines"),
            LdapError::Io(reason) => write!(f, "LDAP password file: {reason}"),
            LdapError::Spawn(reason) => write!(f, "could not run ldapsearch: {reason}"),
            LdapError::Failed(text) => f.write_str(text),
            LdapError::Node(e) => write!(f, "directory node: {e:?}"),
        }
    }
}

impl std::error::Error for LdapError {}

/// A simple bind: the DN and its password. The password is not shown by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Bind {
    dn: String,
    password: String,
}

impl fmt::Debug for Bind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bind").field("dn", &self.dn).field("password", &"<redacted>").finish()
    }
}

/// Where to search: a server URI, the base DN the search starts at, and an optional simple bind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    uri: String,
    base_dn: String,
    bind: Option<Bind>,
}

impl Server {
    pub fn new(uri: &str, base_dn: &str) -> Result<Server, LdapError> {
        let scheme_ok = uri.starts_with("ldap://") || uri.starts_with("ldaps://");
        if !scheme_ok || uri.chars().any(char::is_whitespace) {
            return Err(LdapError::BadUri);
        }
        let base_dn = base_dn.trim();
        if base_dn.is_empty() || base_dn.contains(['\n', '\r', '\0']) {
            return Err(LdapError::BadBaseDn);
        }
        Ok(Server { uri: uri.into(), base_dn: base_dn.into(), bind: None })
    }

    /// The same server with a simple bind as `dn` and `password`. Searches then authenticate; without it they are
    /// anonymous.
    pub fn with_bind(self, dn: &str, password: &str) -> Result<Server, LdapError> {
        let dn = dn.trim();
        let single_line = |s: &str| !s.is_empty() && !s.contains(['\n', '\r', '\0']);
        if !single_line(dn) || !single_line(password) {
            return Err(LdapError::BadCredentials);
        }
        Ok(Server { bind: Some(Bind { dn: dn.into(), password: password.into() }), ..self })
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn base_dn(&self) -> &str {
        &self.base_dn
    }

    /// The DN of the simple bind, when the server has one.
    pub fn bind_dn(&self) -> Option<&str> {
        self.bind.as_ref().map(|b| b.dn.as_str())
    }
}

/// Escapes a value for use inside an LDAP filter (RFC 4515): `\`, `*`, `(`, `)` and NUL become `\` and two hex digits.
pub fn escape_filter_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\5c"),
            '*' => out.push_str("\\2a"),
            '(' => out.push_str("\\28"),
            ')' => out.push_str("\\29"),
            '\0' => out.push_str("\\00"),
            c => out.push(c),
        }
    }
    out
}

/// The `ldapsearch` command for a subtree search. A bound server needs `password_file`, which holds the password (see
/// [`search`]); the password itself is never an argument. Attributes are requested by name; none means every user
/// attribute.
pub fn search_command(server: &Server, filter: &str, attributes: &[&str], password_file: Option<&Path>) -> Result<Command, LdapError> {
    if filter.is_empty() || !filter.starts_with('(') {
        return Err(LdapError::BadFilter);
    }
    let mut cmd = Command::new("ldapsearch");
    cmd.args(["-LLL", "-x", "-H", server.uri(), "-b", server.base_dn(), "-s", "sub"]);
    if let Some(bind) = &server.bind {
        let file = password_file.ok_or(LdapError::BadCredentials)?;
        cmd.args(["-D", bind.dn.as_str(), "-y"]).arg(file);
    }
    cmd.arg(filter);
    cmd.args(attributes);
    Ok(cmd)
}

/// Writes the password to a new private file and returns its path. The file is created exclusively with mode 0600.
fn write_password_file(password: &str) -> io::Result<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("bifrost-ldap-{}-{nanos}.pw", std::process::id()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    file.write_all(password.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// The record type of an entry: its first object class other than `top`, or `entry` when it has none.
fn record_type(object_classes: &[String]) -> String {
    object_classes.iter().find(|c| !c.eq_ignore_ascii_case("top")).cloned().unwrap_or_else(|| "entry".into())
}

/// The record name of an entry: the value of the first RDN of its DN (`uid=alice,ou=people,...` gives `alice`).
fn record_name(dn: &str) -> Option<String> {
    let first = dn.split(',').next()?;
    let (_, value) = first.split_once('=')?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Decodes standard base64 (ignoring white space and padding). None on a character outside the alphabet.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// One LDIF line as `(attribute, value)`. `attr: value` is plain text; `attr:: base64` is decoded, and a value that is
/// not UTF-8 gives None. Attribute names hold no colon, so the first colon ends the name.
fn ldif_pair(line: &str) -> Option<(&str, String)> {
    let (attr, rest) = line.split_once(':')?;
    if attr.is_empty() || attr.contains(char::is_whitespace) {
        return None;
    }
    if let Some(encoded) = rest.strip_prefix(':') {
        let bytes = base64_decode(encoded.trim_start())?;
        return String::from_utf8(bytes).ok().map(|text| (attr, text));
    }
    Some((attr, rest.strip_prefix(' ').unwrap_or(rest).to_string()))
}

/// Reads LDIF as `ldapsearch -LLL` prints it into records. An entry needs a `dn:` line that yields a name; the other
/// lines that do not parse are skipped. A line that starts with one space continues the line before it.
pub fn parse_ldif(text: &str) -> Vec<DirRecord> {
    // unfold continuation lines first
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }

    let mut records = Vec::new();
    for block in lines.split(|l| l.trim().is_empty()) {
        let Some(dn_line) = block.first() else { continue };
        let Some(("dn", dn)) = ldif_pair(dn_line) else { continue };
        let Some(name) = record_name(&dn) else { continue };

        let attrs: Vec<(String, String)> = block[1..]
            .iter()
            .filter_map(|line| ldif_pair(line))
            .map(|(attr, value)| (attr.to_string(), value))
            .collect();

        let classes: Vec<String> = attrs.iter().filter(|(a, _)| a.eq_ignore_ascii_case("objectClass")).map(|(_, v)| v.clone()).collect();
        let mut record = DirRecord::new(&record_type(&classes), &name);
        let mut names: Vec<String> = attrs.iter().map(|(a, _)| a.clone()).collect();
        names.sort();
        names.dedup();
        for attr in names {
            if attr == NAME_ATTRIBUTE {
                continue; // the record's name is read from the DN; it cannot be set as an attribute
            }
            let values: Vec<String> = attrs.iter().filter(|(a, _)| *a == attr).map(|(_, v)| v.clone()).collect();
            let _ = record.set_values(&attr, values);
        }
        records.push(record);
    }
    records
}

/// Runs a subtree search and parses the entries. A bound server writes its password to a private file for the length of
/// the search and removes it afterwards, whatever the outcome.
pub fn search(server: &Server, filter: &str, attributes: &[&str]) -> Result<Vec<DirRecord>, LdapError> {
    let password_file = match &server.bind {
        Some(bind) => Some(write_password_file(&bind.password).map_err(|e| LdapError::Io(e.to_string()))?),
        None => None,
    };
    let output = search_command(server, filter, attributes, password_file.as_deref())
        .and_then(|mut cmd| cmd.output().map_err(|e| LdapError::Spawn(e.to_string())));
    if let Some(path) = &password_file {
        let _ = fs::remove_file(path);
    }
    let out = output?;
    if !out.status.success() {
        return Err(LdapError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(parse_ldif(&String::from_utf8_lossy(&out.stdout)))
}

/// Adds the records to the node at `path` and returns how many were added. A record already in the node (same type
/// and name) is skipped, not replaced.
pub fn load(session: &mut Session, path: &str, records: Vec<DirRecord>) -> Result<usize, LdapError> {
    let node = session.node_mut(path).map_err(LdapError::Node)?;
    let mut added = 0;
    for record in records {
        if node.add_record(record).is_ok() {
            added += 1;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LDIF: &str = "dn: uid=alice,ou=people,dc=example,dc=com\n\
objectClass: top\n\
objectClass: inetOrgPerson\n\
cn: Alice Smith\n\
mail: alice@example.com\n\
mail: a.smith@example.com\n\
description: a long value that\n \
 continues here\n\
\n\
dn: uid=bob,ou=people,dc=example,dc=com\n\
objectClass: top\n\
cn:: Ym9i\n\
\n\
dn:: Y249YmFzZTY0\n\
cn: from a base64 dn\n";

    #[test]
    fn entries_become_records_named_by_their_rdn() {
        let records = parse_ldif(LDIF);
        assert_eq!(records.len(), 3, "the base64 DN entry is decoded too");
        assert_eq!(records[0].name(), "alice");
        assert_eq!(records[0].record_type(), "inetOrgPerson", "top is not a record type");
        assert_eq!(records[1].name(), "bob");
        assert_eq!(records[1].record_type(), "entry", "no object class but top gives the generic type");
        assert_eq!(records[2].name(), "base64");
    }

    #[test]
    fn attributes_keep_every_value_and_unfolded_lines() {
        let alice = &parse_ldif(LDIF)[0];
        assert_eq!(alice.values("cn"), vec!["Alice Smith"]);
        assert_eq!(alice.values("mail"), vec!["alice@example.com", "a.smith@example.com"]);
        assert_eq!(alice.values("description"), vec!["a long value thatcontinues here"], "a folded line joins without its space");
    }

    #[test]
    fn base64_attributes_are_decoded() {
        let bob = &parse_ldif(LDIF)[1];
        assert_eq!(bob.values("cn"), vec!["bob"]);
        assert!(bob.values("name").iter().all(|v| v == "bob"), "name reads the record name");
    }

    #[test]
    fn base64_values_that_are_not_text_or_not_base64_are_skipped() {
        let text = "dn: uid=carol,dc=example\nphoto:: /w==\njpegPhoto:: @@@@\nmail: carol@example.com\n";
        let carol = &parse_ldif(text)[0];
        assert!(carol.values("photo").is_empty(), "0xFF is not UTF-8");
        assert!(carol.values("jpegPhoto").is_empty(), "@ is not base64");
        assert_eq!(carol.values("mail"), vec!["carol@example.com"]);
    }

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64_decode("Ym9i"), Some(b"bob".to_vec()));
        assert_eq!(base64_decode("Ym8="), Some(b"bo".to_vec()));
        assert_eq!(base64_decode("Ym8"), Some(b"bo".to_vec()));
        assert_eq!(base64_decode("Y249 YmFz\nZTY0"), Some(b"cn=base64".to_vec()), "white space is ignored");
        assert_eq!(base64_decode("Y!"), None);
    }

    #[test]
    fn filter_values_are_escaped() {
        assert_eq!(escape_filter_value("a*b(c)\\d"), "a\\2ab\\28c\\29\\5cd");
        assert_eq!(escape_filter_value("plain"), "plain");
    }

    #[test]
    fn server_checks_its_uri_and_base() {
        assert!(Server::new("ldap://ldap.example.com", "dc=example,dc=com").is_ok());
        assert!(Server::new("ldaps://ldap.example.com:636", "dc=example,dc=com").is_ok());
        assert_eq!(Server::new("http://x", "dc=example"), Err(LdapError::BadUri));
        assert_eq!(Server::new("ldap://a b", "dc=example"), Err(LdapError::BadUri));
        assert_eq!(Server::new("ldap://x", "  "), Err(LdapError::BadBaseDn));
    }

    #[test]
    fn search_command_is_built_without_a_shell() {
        let server = Server::new("ldaps://ldap.example.com", "dc=example,dc=com").unwrap();
        let cmd = search_command(&server, "(uid=alice)", &["cn", "mail"], None).unwrap();
        assert_eq!(cmd.get_program(), "ldapsearch");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            vec!["-LLL", "-x", "-H", "ldaps://ldap.example.com", "-b", "dc=example,dc=com", "-s", "sub", "(uid=alice)", "cn", "mail"]
        );
        assert_eq!(search_command(&server, "uid=alice", &[], None).err(), Some(LdapError::BadFilter));
    }

    #[test]
    fn a_bind_passes_its_dn_and_password_file_never_the_password() {
        let server = Server::new("ldaps://ldap.example.com", "dc=example,dc=com")
            .unwrap()
            .with_bind("uid=svc,ou=services,dc=example,dc=com", "s3cret-pass")
            .unwrap();
        assert_eq!(server.bind_dn(), Some("uid=svc,ou=services,dc=example,dc=com"));
        let file = Path::new("/run/user/1000/bifrost-ldap-test.pw");
        let cmd = search_command(&server, "(uid=alice)", &["cn"], Some(file)).unwrap();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let pos = args.iter().position(|a| a == "-D").expect("-D present");
        assert_eq!(args[pos + 1], "uid=svc,ou=services,dc=example,dc=com");
        assert_eq!(args[pos + 2], "-y");
        assert_eq!(args[pos + 3], "/run/user/1000/bifrost-ldap-test.pw");
        assert_eq!(args.iter().position(|a| a == "(uid=alice)").unwrap(), pos + 4, "options come before the filter");
        assert!(!args.iter().any(|a| a.contains("s3cret")), "the password is never an argument");
        assert_eq!(search_command(&server, "(uid=alice)", &[], None).err(), Some(LdapError::BadCredentials));
    }

    #[test]
    fn bind_credentials_are_checked_and_hidden_in_debug_output() {
        let base = || Server::new("ldap://ldap.example.com", "dc=example").unwrap();
        assert_eq!(base().with_bind("", "pw").err(), Some(LdapError::BadCredentials));
        assert_eq!(base().with_bind("uid=x", "").err(), Some(LdapError::BadCredentials));
        assert_eq!(base().with_bind("uid=x", "a\nb").err(), Some(LdapError::BadCredentials));
        let server = base().with_bind("uid=x,dc=example", "hunter2").unwrap();
        assert!(!format!("{server:?}").contains("hunter2"));
        assert!(format!("{server:?}").contains("<redacted>"));
    }

    #[test]
    fn the_password_file_is_private_and_holds_the_password() {
        let path = write_password_file("hunter2").unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hunter2\n");
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn loading_fills_a_node_and_skips_duplicates() {
        let mut session = Session::new();
        session.add_node("/LDAP").unwrap();
        let records = parse_ldif(LDIF);
        assert_eq!(load(&mut session, "/LDAP", records.clone()), Ok(3));
        assert_eq!(load(&mut session, "/LDAP", records), Ok(0), "same type and name is already there");
        assert_eq!(session.node("/LDAP").unwrap().record_count(), 3);
        assert!(matches!(load(&mut session, "/Missing", Vec::new()), Err(LdapError::Node(_))));
    }
}
