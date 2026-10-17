//! LDAP directory nodes filled from a server through the OpenLDAP client, `ldapsearch` (the `openldap` package).
//! A search runs `ldapsearch -LLL -x -H <uri> -b <base> -s sub <filter> <attributes...>`, reads the LDIF it prints,
//! and turns each entry into a [`DirRecord`]. [`load`] puts those records into a [`Session`] node, so the
//! directory query model in `bifrost_ui::directory_query` runs on them like any other directory.
//!
//! Simple (anonymous) binds only; the tool reads its own configuration (`/etc/openldap/ldap.conf`) for TLS. The
//! command runs without a shell, and a filter value from user input must go through [`escape_filter_value`].
//! LDIF values that are base64-encoded (`attr:: ...`) are skipped, not decoded.

use std::fmt;
use std::process::Command;

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
            LdapError::Spawn(reason) => write!(f, "could not run ldapsearch: {reason}"),
            LdapError::Failed(text) => f.write_str(text),
            LdapError::Node(e) => write!(f, "directory node: {e:?}"),
        }
    }
}

impl std::error::Error for LdapError {}

/// Where to search: a server URI and the base DN the search starts at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    uri: String,
    base_dn: String,
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
        Ok(Server { uri: uri.into(), base_dn: base_dn.into() })
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn base_dn(&self) -> &str {
        &self.base_dn
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

/// The `ldapsearch` command for a subtree search. Attributes are requested by name; none means every user attribute.
pub fn search_command(server: &Server, filter: &str, attributes: &[&str]) -> Result<Command, LdapError> {
    if filter.is_empty() || !filter.starts_with('(') {
        return Err(LdapError::BadFilter);
    }
    let mut cmd = Command::new("ldapsearch");
    cmd.args(["-LLL", "-x", "-H", server.uri(), "-b", server.base_dn(), "-s", "sub", filter]);
    cmd.args(attributes);
    Ok(cmd)
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

/// Reads LDIF as `ldapsearch -LLL` prints it into records. Entries without a plain `dn:` line are skipped, and so
/// are base64 attributes. A line that starts with one space continues the line before it.
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
        let Some(dn) = dn_line.strip_prefix("dn: ") else { continue };
        let Some(name) = record_name(dn) else { continue };

        let mut attrs: Vec<(String, String)> = Vec::new();
        for line in &block[1..] {
            // `attr:: base64` has a second colon; skip it, the model holds plain text only
            let Some((attr, value)) = line.split_once(": ") else { continue };
            if attr.is_empty() || attr.contains(':') {
                continue;
            }
            attrs.push((attr.to_string(), value.to_string()));
        }

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

/// Runs a subtree search and parses the entries.
pub fn search(server: &Server, filter: &str, attributes: &[&str]) -> Result<Vec<DirRecord>, LdapError> {
    let out = search_command(server, filter, attributes)?
        .output()
        .map_err(|e| LdapError::Spawn(e.to_string()))?;
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
cn: skipped\n";

    #[test]
    fn entries_become_records_named_by_their_rdn() {
        let records = parse_ldif(LDIF);
        assert_eq!(records.len(), 2, "the base64 DN entry is skipped");
        assert_eq!(records[0].name(), "alice");
        assert_eq!(records[0].record_type(), "inetOrgPerson", "top is not a record type");
        assert_eq!(records[1].name(), "bob");
        assert_eq!(records[1].record_type(), "entry", "no object class but top gives the generic type");
    }

    #[test]
    fn attributes_keep_every_value_and_unfolded_lines() {
        let alice = &parse_ldif(LDIF)[0];
        assert_eq!(alice.values("cn"), vec!["Alice Smith"]);
        assert_eq!(alice.values("mail"), vec!["alice@example.com", "a.smith@example.com"]);
        assert_eq!(alice.values("description"), vec!["a long value thatcontinues here"], "a folded line joins without its space");
    }

    #[test]
    fn base64_attributes_are_skipped_and_name_is_not_an_attribute() {
        let bob = &parse_ldif(LDIF)[1];
        assert!(bob.values("cn").is_empty());
        assert!(bob.values("name").iter().all(|v| v == "bob"), "name reads the record name");
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
        let cmd = search_command(&server, "(uid=alice)", &["cn", "mail"]).unwrap();
        assert_eq!(cmd.get_program(), "ldapsearch");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            vec!["-LLL", "-x", "-H", "ldaps://ldap.example.com", "-b", "dc=example,dc=com", "-s", "sub", "(uid=alice)", "cn", "mail"]
        );
        assert_eq!(search_command(&server, "uid=alice", &[]).err(), Some(LdapError::BadFilter));
    }

    #[test]
    fn loading_fills_a_node_and_skips_duplicates() {
        let mut session = Session::new();
        session.add_node("/LDAP").unwrap();
        let records = parse_ldif(LDIF);
        assert_eq!(load(&mut session, "/LDAP", records.clone()), Ok(2));
        assert_eq!(load(&mut session, "/LDAP", records), Ok(0), "same type and name is already there");
        assert_eq!(session.node("/LDAP").unwrap().record_count(), 2);
        assert!(matches!(load(&mut session, "/Missing", Vec::new()), Err(LdapError::Node(_))));
    }
}
