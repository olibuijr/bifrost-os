//! Directory service configuration: which directory servers this machine searches, in what order, and the client
//! configuration file that follows from the list.
//!
//! A source is a named LDAP server: its URI, the search base and whether the connection is upgraded with StartTLS. The
//! list is the search policy: the first enabled source answers first. A source is accepted only when its traffic is
//! protected: an `ldaps://` URI, or an `ldap://` URI with StartTLS on. The local directory (`directory_local`) is always
//! searched and is not a source here.
//!
//! [`DirectoryConfig::render_ldap_conf`] writes the OpenLDAP client file (`/etc/openldap/ldap.conf`, from the `openldap`
//! package): `URI` lists every enabled server in order, `BASE` is the first enabled source's base, and certificates are
//! always required. The module opens no connection and writes no file; the caller (the privileged helper) installs the
//! text.

/// Most sources one configuration holds.
pub const MAX_SOURCES: usize = 16;

/// Why a source or an edit is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The name is empty, over 64 bytes or holds a control character.
    BadName,
    /// The URI is not `ldap://host[:port]` or `ldaps://host[:port]`.
    BadUri,
    /// The base is empty or holds a control character.
    BadBase,
    /// `ldap://` without StartTLS would send passwords in the clear.
    Unprotected,
    /// A source with this name exists.
    Exists,
    /// No source has this name.
    Unknown,
    /// The configuration is full.
    TooMany,
}

/// One directory server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub uri: String,
    /// The distinguished name searches start at, for example `dc=example,dc=com`.
    pub base: String,
    /// Upgrade an `ldap://` connection with StartTLS (an `ldaps://` URI is protected already).
    pub start_tls: bool,
    pub enabled: bool,
}

fn host_port_ok(rest: &str) -> bool {
    let rest = rest.trim_end_matches('/');
    let (host, port) = if rest.starts_with('[') {
        // a bracketed IPv6 address, then an optional :port
        let Some(close) = rest.find(']') else { return false };
        let tail = &rest[close + 1..];
        if !tail.is_empty() && !tail.starts_with(':') {
            return false;
        }
        (&rest[..=close], tail.strip_prefix(':'))
    } else {
        match rest.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (rest, None),
        }
    };
    let host_ok = !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
    host_ok && port.map_or(true, |p| p.parse::<u16>().is_ok_and(|n| n > 0))
}

impl Source {
    /// Checks a source against the rules in the module comment.
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() || self.name.len() > 64 || self.name.chars().any(char::is_control) {
            return Err(Error::BadName);
        }
        let secure = if let Some(rest) = self.uri.strip_prefix("ldaps://") {
            if !host_port_ok(rest) {
                return Err(Error::BadUri);
            }
            true
        } else if let Some(rest) = self.uri.strip_prefix("ldap://") {
            if !host_port_ok(rest) {
                return Err(Error::BadUri);
            }
            self.start_tls
        } else {
            return Err(Error::BadUri);
        };
        if self.base.trim().is_empty() || self.base.chars().any(char::is_control) {
            return Err(Error::BadBase);
        }
        if secure {
            Ok(())
        } else {
            Err(Error::Unprotected)
        }
    }
}

/// The ordered list of directory sources.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryConfig {
    sources: Vec<Source>,
}

impl DirectoryConfig {
    pub fn new() -> DirectoryConfig {
        DirectoryConfig::default()
    }

    /// The sources in search order.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// The enabled sources in search order.
    pub fn enabled(&self) -> Vec<&Source> {
        self.sources.iter().filter(|s| s.enabled).collect()
    }

    /// Adds a source at the end of the search order.
    pub fn add(&mut self, source: Source) -> Result<(), Error> {
        source.validate()?;
        if self.sources.len() >= MAX_SOURCES {
            return Err(Error::TooMany);
        }
        if self.sources.iter().any(|s| s.name == source.name) {
            return Err(Error::Exists);
        }
        self.sources.push(source);
        Ok(())
    }

    /// Removes a source by name.
    pub fn remove(&mut self, name: &str) -> Result<Source, Error> {
        let at = self.sources.iter().position(|s| s.name == name).ok_or(Error::Unknown)?;
        Ok(self.sources.remove(at))
    }

    /// Switches a source on or off without losing its place in the order.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), Error> {
        self.sources.iter_mut().find(|s| s.name == name).ok_or(Error::Unknown)?.enabled = enabled;
        Ok(())
    }

    /// Moves a source to `index` in the search order (clamped to the end).
    pub fn move_to(&mut self, name: &str, index: usize) -> Result<(), Error> {
        let from = self.sources.iter().position(|s| s.name == name).ok_or(Error::Unknown)?;
        let s = self.sources.remove(from);
        self.sources.insert(index.min(self.sources.len()), s);
        Ok(())
    }

    /// The OpenLDAP client file for this configuration. With no enabled source it only keeps certificate checking on.
    pub fn render_ldap_conf(&self) -> String {
        let mut out = String::from("# Written by Directory settings. Edit the sources there.\n");
        let on = self.enabled();
        if let Some(first) = on.first() {
            let uris: Vec<&str> = on.iter().map(|s| s.uri.as_str()).collect();
            out.push_str(&format!("URI {}\nBASE {}\n", uris.join(" "), first.base.trim()));
        }
        out.push_str("TLS_REQCERT demand\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(name: &str, uri: &str, tls: bool) -> Source {
        Source { name: name.into(), uri: uri.into(), base: "dc=example,dc=com".into(), start_tls: tls, enabled: true }
    }

    #[test]
    fn only_protected_sources_with_a_good_address_are_accepted() {
        assert_eq!(src("a", "ldaps://dir.example.com", false).validate(), Ok(()));
        assert_eq!(src("a", "ldaps://dir.example.com:636", false).validate(), Ok(()));
        assert_eq!(src("a", "ldap://dir.example.com", true).validate(), Ok(()));
        assert_eq!(src("a", "ldap://dir.example.com", false).validate(), Err(Error::Unprotected));
        assert_eq!(src("a", "http://dir.example.com", true).validate(), Err(Error::BadUri));
        assert_eq!(src("a", "ldaps://[2001:db8::1]:636", false).validate(), Ok(()));
        assert_eq!(src("a", "ldaps://[2001:db8::1]x", false).validate(), Err(Error::BadUri));
        assert_eq!(src("a", "ldaps://", false).validate(), Err(Error::BadUri));
        assert_eq!(src("a", "ldaps://dir.example.com:99999", false).validate(), Err(Error::BadUri));
        assert_eq!(src("a", "ldaps://dir example", false).validate(), Err(Error::BadUri));
        assert_eq!(src("", "ldaps://d.example", false).validate(), Err(Error::BadName));
        let mut s = src("a", "ldaps://d.example", false);
        s.base = "  ".into();
        assert_eq!(s.validate(), Err(Error::BadBase));
    }

    #[test]
    fn sources_keep_their_order_names_and_switches() {
        let mut c = DirectoryConfig::new();
        c.add(src("corp", "ldaps://corp.example", false)).unwrap();
        c.add(src("lab", "ldap://lab.example", true)).unwrap();
        assert_eq!(c.add(src("lab", "ldaps://x.example", false)), Err(Error::Exists));
        c.move_to("lab", 0).unwrap();
        assert_eq!(c.sources().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["lab", "corp"]);
        c.set_enabled("lab", false).unwrap();
        assert_eq!(c.enabled().len(), 1);
        assert_eq!(c.set_enabled("nope", true), Err(Error::Unknown));
        assert_eq!(c.remove("lab").unwrap().name, "lab");
        assert_eq!(c.remove("lab"), Err(Error::Unknown));
        for i in 0..MAX_SOURCES {
            let _ = c.add(src(&format!("s{i}"), "ldaps://s.example", false));
        }
        assert_eq!(c.add(src("last", "ldaps://s.example", false)), Err(Error::TooMany));
    }

    #[test]
    fn the_client_file_lists_enabled_servers_in_order_with_the_first_base() {
        let mut c = DirectoryConfig::new();
        assert_eq!(c.render_ldap_conf(), "# Written by Directory settings. Edit the sources there.\nTLS_REQCERT demand\n");
        c.add(src("corp", "ldaps://corp.example", false)).unwrap();
        let mut lab = src("lab", "ldap://lab.example", true);
        lab.base = "dc=lab".into();
        c.add(lab).unwrap();
        let text = c.render_ldap_conf();
        assert!(text.contains("URI ldaps://corp.example ldap://lab.example\n"));
        assert!(text.contains("BASE dc=example,dc=com\n"));
        assert!(text.ends_with("TLS_REQCERT demand\n"));
        c.set_enabled("corp", false).unwrap();
        let text = c.render_ldap_conf();
        assert!(text.contains("URI ldap://lab.example\n") && text.contains("BASE dc=lab\n"));
    }
}
