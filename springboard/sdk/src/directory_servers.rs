//! The list of directory servers an administrator has set up, and the editing of that list: add, remove, rename,
//! turn on or off, change the search order, test a connection, and write the OpenLDAP client defaults.
//!
//! A [`Book`] is the model behind a directory settings pane. Each [`Entry`] names a server (`uri`, search base and an
//! optional bind DN); the order of the entries is the search order, and only enabled entries are used. The list is
//! saved as text ([`Book::save`], by default at [`DEFAULT_PATH`]). **Passwords are never stored here**: the caller
//! passes the password for a bound server when it connects ([`Book::server`], [`Book::probe`]), usually read from the
//! keychain. [`Book::ldap_conf`] writes the `URI` and `BASE` lines that `ldapsearch` and other OpenLDAP clients read
//! from `/etc/openldap/ldap.conf`. Servers and searches run through [`crate::ldap_directory`].

use std::fmt;
use std::path::Path;

use crate::ldap_directory::{self, LdapError, Server};

/// Where the list is kept on a machine.
pub const DEFAULT_PATH: &str = "/var/lib/springboard/directory-servers.txt";
/// Most servers in the list.
pub const MAX_ENTRIES: usize = 32;

/// Why an edit was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The name is empty, holds a tab or line break, or is longer than 64 bytes.
    BadName,
    /// Another entry already has the name.
    Duplicate,
    /// No entry has the name.
    NotFound,
    /// The list already holds [`MAX_ENTRIES`] entries.
    Full,
    /// The address, base or bind DN is not acceptable.
    Ldap(LdapError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadName => f.write_str("a server name is 1 to 64 characters on one line"),
            Error::Duplicate => f.write_str("a server with that name already exists"),
            Error::NotFound => f.write_str("no server has that name"),
            Error::Full => f.write_str("the server list is full"),
            Error::Ldap(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<LdapError> for Error {
    fn from(e: LdapError) -> Error {
        Error::Ldap(e)
    }
}

/// One directory server of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub uri: String,
    pub base_dn: String,
    /// The DN to bind as; `None` searches anonymously.
    pub bind_dn: Option<String>,
    pub enabled: bool,
}

impl Entry {
    /// A new enabled entry with an anonymous bind. Checks the name, address and base.
    pub fn new(name: &str, uri: &str, base_dn: &str) -> Result<Entry, Error> {
        check_name(name)?;
        let server = Server::new(uri, base_dn)?;
        Ok(Entry { name: name.into(), uri: server.uri().into(), base_dn: server.base_dn().into(), bind_dn: None, enabled: true })
    }

    /// The same entry that binds as `dn`.
    pub fn with_bind_dn(mut self, dn: &str) -> Result<Entry, Error> {
        let dn = dn.trim();
        if dn.is_empty() || dn.contains(['\t', '\n', '\r', '\0']) {
            return Err(Error::Ldap(LdapError::BadCredentials));
        }
        self.bind_dn = Some(dn.into());
        Ok(self)
    }
}

fn check_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.len() > 64 || name.contains(['\t', '\n', '\r', '\0']) {
        return Err(Error::BadName);
    }
    Ok(())
}

/// The ordered list of servers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    entries: Vec<Entry>,
}

impl Book {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    fn index(&self, name: &str) -> Result<usize, Error> {
        self.entries.iter().position(|e| e.name == name).ok_or(Error::NotFound)
    }

    /// Adds an entry at the end of the search order.
    pub fn add(&mut self, entry: Entry) -> Result<(), Error> {
        check_name(&entry.name)?;
        Server::new(&entry.uri, &entry.base_dn)?;
        if self.get(&entry.name).is_some() {
            return Err(Error::Duplicate);
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::Full);
        }
        self.entries.push(entry);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<Entry, Error> {
        let i = self.index(name)?;
        Ok(self.entries.remove(i))
    }

    pub fn rename(&mut self, name: &str, new_name: &str) -> Result<(), Error> {
        check_name(new_name)?;
        let i = self.index(name)?;
        if name != new_name && self.get(new_name).is_some() {
            return Err(Error::Duplicate);
        }
        self.entries[i].name = new_name.into();
        Ok(())
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), Error> {
        let i = self.index(name)?;
        self.entries[i].enabled = enabled;
        Ok(())
    }

    /// Moves an entry to `position` in the search order (clamped to the end of the list).
    pub fn move_to(&mut self, name: &str, position: usize) -> Result<(), Error> {
        let i = self.index(name)?;
        let entry = self.entries.remove(i);
        self.entries.insert(position.min(self.entries.len()), entry);
        Ok(())
    }

    /// The enabled entries in search order.
    pub fn enabled(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.enabled)
    }

    /// The LDAP server of an entry, bound with `password` when the entry has a bind DN. An entry with a bind DN and no
    /// password searches anonymously rather than binding with an empty password.
    pub fn server(&self, name: &str, password: Option<&str>) -> Result<Server, Error> {
        let e = self.get(name).ok_or(Error::NotFound)?;
        let server = Server::new(&e.uri, &e.base_dn)?;
        match (&e.bind_dn, password) {
            (Some(dn), Some(pw)) => Ok(server.with_bind(dn, pw)?),
            _ => Ok(server),
        }
    }

    /// Tests a connection: searches the base for its own entry and returns how many entries came back.
    pub fn probe(&self, name: &str, password: Option<&str>) -> Result<usize, Error> {
        let server = self.server(name, password)?;
        let found = ldap_directory::search(&server, "(objectClass=*)", &["dn"])?;
        Ok(found.len())
    }

    /// The OpenLDAP client defaults for the enabled servers: one `URI` line with every address in order and a `BASE`
    /// line from the first. Empty when no entry is enabled.
    pub fn ldap_conf(&self) -> String {
        let mut it = self.enabled();
        let Some(first) = it.next() else { return String::new() };
        let mut uris = vec![first.uri.as_str()];
        uris.extend(it.map(|e| e.uri.as_str()));
        format!("URI {}\nBASE {}\n", uris.join(" "), first.base_dn)
    }

    /// The list as text, one `server<TAB>name<TAB>on|off<TAB>uri<TAB>base<TAB>bind dn` line per entry.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&format!(
                "server\t{}\t{}\t{}\t{}\t{}\n",
                e.name,
                if e.enabled { "on" } else { "off" },
                e.uri,
                e.base_dn,
                e.bind_dn.as_deref().unwrap_or("")
            ));
        }
        out
    }

    /// Reads [`Book::to_text`] output. A line that does not describe a valid, new entry is skipped.
    pub fn parse(text: &str) -> Book {
        let mut book = Book::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 6 || f[0] != "server" || !matches!(f[2], "on" | "off") {
                continue;
            }
            let Ok(mut entry) = Entry::new(f[1], f[3], f[4]) else { continue };
            entry.enabled = f[2] == "on";
            if !f[5].is_empty() {
                match entry.with_bind_dn(f[5]) {
                    Ok(e) => entry = e,
                    Err(_) => continue,
                }
            }
            let _ = book.add(entry);
        }
        book
    }

    pub fn load(path: &Path) -> Book {
        std::fs::read_to_string(path).map(|t| Book::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        bifrost_core::fsutil::write_atomic(path, self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry::new(name, &format!("ldaps://{name}.example.com"), "dc=example,dc=com").unwrap()
    }

    fn book() -> Book {
        let mut b = Book::default();
        for n in ["alpha", "beta", "gamma"] {
            b.add(entry(n)).unwrap();
        }
        b
    }

    #[test]
    fn entries_are_checked_when_made_and_added() {
        assert_eq!(Entry::new("", "ldap://h", "dc=x"), Err(Error::BadName));
        assert_eq!(Entry::new("a\tb", "ldap://h", "dc=x"), Err(Error::BadName));
        assert_eq!(Entry::new("a", "http://h", "dc=x"), Err(Error::Ldap(LdapError::BadUri)));
        assert_eq!(Entry::new("a", "ldap://h", " "), Err(Error::Ldap(LdapError::BadBaseDn)));
        assert!(entry("a").with_bind_dn("cn=admin,dc=example,dc=com").is_ok());
        assert_eq!(entry("a").with_bind_dn(""), Err(Error::Ldap(LdapError::BadCredentials)));
        let mut b = book();
        assert_eq!(b.add(entry("alpha")), Err(Error::Duplicate));
        let mut bad = entry("zeta");
        bad.uri = "nonsense".into();
        assert_eq!(b.add(bad), Err(Error::Ldap(LdapError::BadUri)));
    }

    #[test]
    fn the_list_has_a_size_limit() {
        let mut b = Book::default();
        for i in 0..MAX_ENTRIES {
            b.add(entry(&format!("s{i}"))).unwrap();
        }
        assert_eq!(b.add(entry("extra")), Err(Error::Full));
    }

    #[test]
    fn edits_change_names_state_and_order() {
        let mut b = book();
        b.rename("beta", "bravo").unwrap();
        assert_eq!(b.rename("alpha", "gamma"), Err(Error::Duplicate));
        assert_eq!(b.rename("nope", "x"), Err(Error::NotFound));
        b.move_to("gamma", 0).unwrap();
        let names: Vec<&str> = b.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["gamma", "alpha", "bravo"]);
        b.move_to("gamma", 99).unwrap();
        assert_eq!(b.entries()[2].name, "gamma");
        b.set_enabled("alpha", false).unwrap();
        assert_eq!(b.enabled().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["bravo", "gamma"]);
        assert_eq!(b.remove("bravo").unwrap().name, "bravo");
        assert_eq!(b.remove("bravo"), Err(Error::NotFound));
    }

    #[test]
    fn the_client_defaults_list_the_enabled_servers_in_order() {
        let mut b = book();
        b.set_enabled("alpha", false).unwrap();
        assert_eq!(b.ldap_conf(), "URI ldaps://beta.example.com ldaps://gamma.example.com\nBASE dc=example,dc=com\n");
        b.set_enabled("beta", false).unwrap();
        b.set_enabled("gamma", false).unwrap();
        assert_eq!(b.ldap_conf(), "");
    }

    #[test]
    fn a_bound_server_needs_its_password_and_the_list_keeps_none() {
        let mut b = Book::default();
        b.add(entry("corp").with_bind_dn("cn=reader,dc=example,dc=com").unwrap()).unwrap();
        let anon = b.server("corp", None).unwrap();
        assert_eq!(anon.bind_dn(), None, "no password, no bind");
        let bound = b.server("corp", Some("s3cret")).unwrap();
        assert_eq!(bound.bind_dn(), Some("cn=reader,dc=example,dc=com"));
        assert!(!b.to_text().contains("s3cret"));
        assert_eq!(b.server("none", None), Err(Error::NotFound));
    }

    #[test]
    fn the_text_round_trips_and_skips_bad_lines() {
        let mut b = book();
        b.add(entry("corp").with_bind_dn("cn=reader,dc=example,dc=com").unwrap()).unwrap();
        b.set_enabled("beta", false).unwrap();
        assert_eq!(Book::parse(&b.to_text()), b);
        let text = "junk\nserver\tok\ton\tldap://h\tdc=x\t\nserver\tbad\ton\thttp://h\tdc=x\t\nserver\tok\ton\tldap://h2\tdc=y\t\nserver\tx\tmaybe\tldap://h\tdc=x\t\n";
        let parsed = Book::parse(text);
        assert_eq!(parsed.entries().len(), 1, "a bad line and a repeated name are skipped");
        assert_eq!(parsed.entries()[0].uri, "ldap://h");
    }

    #[test]
    fn the_list_is_saved_and_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("etc/servers.txt");
        let b = book();
        b.save(&path).unwrap();
        assert_eq!(Book::load(&path), b);
        assert_eq!(Book::load(&dir.path().join("missing")), Book::default());
    }
}
