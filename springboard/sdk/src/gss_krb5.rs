//! The Kerberos 5 settings of the GSS-API (`gss_krb5_ccache_name`, `gss_krb5_set_allowable_enctypes`,
//! `gsskrb5_register_acceptor_identity`): where the client keeps its credential cache, which encryption types may
//! leave the library as exported keys, and which keytab the acceptor uses for its own key.
//!
//! Names use the `TYPE:residual` form of the Kerberos credential cache and keytab names. A name with no type prefix is
//! a file name. The types this model accepts are the ones the Kerberos library documents: `FILE`, `DIR`, `KEYRING`,
//! `KCM` and `MEMORY` for credential caches, and `FILE` for keytabs. The residual is never empty.
//!
//! The encryption-type list limits the keys an export call may return. With no list set every type is allowed, as the
//! reference describes the default; an empty list allows none.
//!
//! Not here: the ticket exchange, the lucid context export and the cache copy, which need a Kerberos library. The
//! settings are only recorded; no ticket is requested or read by this module.

use std::collections::BTreeSet;
use std::fmt;

/// A credential cache type prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    File,
    Directory,
    Keyring,
    Kcm,
    Memory,
}

impl CacheKind {
    fn from_prefix(prefix: &str) -> Option<CacheKind> {
        Some(match prefix {
            "FILE" => CacheKind::File,
            "DIR" => CacheKind::Directory,
            "KEYRING" => CacheKind::Keyring,
            "KCM" => CacheKind::Kcm,
            "MEMORY" => CacheKind::Memory,
            _ => return None,
        })
    }
}

/// A parsed credential cache name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheName {
    kind: CacheKind,
    residual: String,
}

impl CacheName {
    pub fn kind(&self) -> CacheKind {
        self.kind
    }

    /// The part after the type prefix (a path, a name or a container), or the whole name when it has no prefix.
    pub fn residual(&self) -> &str {
        &self.residual
    }
}

/// Why a name or a setting is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Krb5Error {
    /// The name is empty, or its residual after the prefix is empty.
    EmptyName,
    /// The prefix is not a type this module knows, such as `XYZ:`.
    UnknownType(String),
    /// A keytab name must be a file, given with no prefix or with `FILE:`.
    UnsupportedKeytab(String),
}

impl fmt::Display for Krb5Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Krb5Error::EmptyName => f.write_str("Kerberos name is empty"),
            Krb5Error::UnknownType(t) => write!(f, "unknown Kerberos name type {}", t),
            Krb5Error::UnsupportedKeytab(t) => write!(f, "keytab type {} is not supported", t),
        }
    }
}

impl std::error::Error for Krb5Error {}

/// Splits `TYPE:residual`. A name with no colon has no type and is returned whole, with `None`.
fn split_type(text: &str) -> Result<(Option<&str>, &str), Krb5Error> {
    if text.is_empty() {
        return Err(Krb5Error::EmptyName);
    }
    match text.split_once(':') {
        Some((prefix, rest)) if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_uppercase()) => {
            if rest.is_empty() {
                Err(Krb5Error::EmptyName)
            } else {
                Ok((Some(prefix), rest))
            }
        }
        _ => Ok((None, text)),
    }
}

impl CacheName {
    /// Parses a credential cache name. A prefix that is all capitals but not a known type is refused, so a typo does not
    /// silently become a file name. A name with no colon is a file.
    pub fn parse(text: &str) -> Result<CacheName, Krb5Error> {
        match split_type(text)? {
            (None, whole) => Ok(CacheName { kind: CacheKind::File, residual: whole.to_string() }),
            (Some(prefix), rest) => {
                let kind = CacheKind::from_prefix(prefix).ok_or_else(|| Krb5Error::UnknownType(prefix.to_string()))?;
                Ok(CacheName { kind, residual: rest.to_string() })
            }
        }
    }
}

/// A parsed keytab name, which is always a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeytabName {
    path: String,
}

impl KeytabName {
    pub fn parse(text: &str) -> Result<KeytabName, Krb5Error> {
        match split_type(text)? {
            (None, whole) => Ok(KeytabName { path: whole.to_string() }),
            (Some("FILE"), rest) => Ok(KeytabName { path: rest.to_string() }),
            (Some(prefix), _) => Err(Krb5Error::UnsupportedKeytab(prefix.to_string())),
        }
    }

    /// The file path of the keytab.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// The Kerberos settings of one GSS-API user. Each setting is `None` until the application sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Krb5Settings {
    ccache: Option<CacheName>,
    /// `None` allows every encryption type. `Some` allows exactly the listed types.
    allowed_enctypes: Option<BTreeSet<u32>>,
    acceptor_keytab: Option<KeytabName>,
}

impl Krb5Settings {
    pub fn new() -> Krb5Settings {
        Krb5Settings::default()
    }

    /// `gss_krb5_ccache_name`: sets the credential cache, and returns the one it replaces.
    pub fn set_ccache_name(&mut self, text: &str) -> Result<Option<CacheName>, Krb5Error> {
        let name = CacheName::parse(text)?;
        Ok(self.ccache.replace(name))
    }

    pub fn ccache(&self) -> Option<&CacheName> {
        self.ccache.as_ref()
    }

    /// `gss_krb5_set_allowable_enctypes`: only the listed encryption types may be exported. Repeats collapse.
    pub fn set_allowable_enctypes(&mut self, enctypes: &[u32]) {
        self.allowed_enctypes = Some(enctypes.iter().copied().collect());
    }

    /// True when an exported key of this encryption type is allowed.
    pub fn enctype_allowed(&self, enctype: u32) -> bool {
        match &self.allowed_enctypes {
            None => true,
            Some(set) => set.contains(&enctype),
        }
    }

    /// The keys from `keys` that an export may return, in their given order.
    pub fn filter_exportable(&self, keys: &[u32]) -> Vec<u32> {
        keys.iter().copied().filter(|k| self.enctype_allowed(*k)).collect()
    }

    /// `gsskrb5_register_acceptor_identity`: the keytab the acceptor uses. Returns the one it replaces.
    pub fn register_acceptor_identity(&mut self, text: &str) -> Result<Option<KeytabName>, Krb5Error> {
        let keytab = KeytabName::parse(text)?;
        Ok(self.acceptor_keytab.replace(keytab))
    }

    pub fn acceptor_keytab(&self) -> Option<&KeytabName> {
        self.acceptor_keytab.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_split_on_a_known_type_prefix() {
        let c = CacheName::parse("FILE:/tmp/krb5cc_501").unwrap();
        assert_eq!(c.kind(), CacheKind::File);
        assert_eq!(c.residual(), "/tmp/krb5cc_501");
        assert_eq!(CacheName::parse("KEYRING:persistent:0").unwrap().kind(), CacheKind::Keyring);
        assert_eq!(CacheName::parse("DIR:/run/krb5/cc").unwrap().residual(), "/run/krb5/cc");
        assert_eq!(CacheName::parse("KCM:501").unwrap().kind(), CacheKind::Kcm);
    }

    #[test]
    fn a_name_without_a_prefix_is_a_file_and_a_bad_prefix_is_refused() {
        let plain = CacheName::parse("/tmp/cache").unwrap();
        assert_eq!(plain.kind(), CacheKind::File);
        assert_eq!(plain.residual(), "/tmp/cache");
        assert_eq!(CacheName::parse("XYZ:/tmp/cache"), Err(Krb5Error::UnknownType("XYZ".to_string())));
        assert_eq!(CacheName::parse("FILE:"), Err(Krb5Error::EmptyName));
        assert_eq!(CacheName::parse(""), Err(Krb5Error::EmptyName));
    }

    #[test]
    fn keytab_names_accept_only_files() {
        assert_eq!(KeytabName::parse("FILE:/etc/krb5.keytab").unwrap().path(), "/etc/krb5.keytab");
        assert_eq!(KeytabName::parse("/etc/krb5.keytab").unwrap().path(), "/etc/krb5.keytab");
        assert_eq!(KeytabName::parse("MEMORY:x"), Err(Krb5Error::UnsupportedKeytab("MEMORY".to_string())));
    }

    #[test]
    fn enctype_list_is_unrestricted_until_set_and_then_exact() {
        let mut s = Krb5Settings::new();
        assert!(s.enctype_allowed(18));
        assert_eq!(s.filter_exportable(&[18, 17, 23]), vec![18, 17, 23]);
        s.set_allowable_enctypes(&[18, 18, 23]);
        assert!(s.enctype_allowed(18));
        assert!(!s.enctype_allowed(17));
        assert_eq!(s.filter_exportable(&[17, 23, 18]), vec![23, 18]);
        s.set_allowable_enctypes(&[]);
        assert!(s.filter_exportable(&[18]).is_empty());
    }

    #[test]
    fn setting_a_cache_or_keytab_returns_the_previous_value() {
        let mut s = Krb5Settings::new();
        assert_eq!(s.set_ccache_name("FILE:/tmp/a").unwrap(), None);
        let old = s.set_ccache_name("MEMORY:b").unwrap().unwrap();
        assert_eq!(old.kind(), CacheKind::File);
        assert_eq!(s.ccache().unwrap().kind(), CacheKind::Memory);
        assert!(s.set_ccache_name("XYZ:c").is_err());
        assert_eq!(s.ccache().unwrap().kind(), CacheKind::Memory, "a refused name changes nothing");
        assert_eq!(s.register_acceptor_identity("/etc/one.keytab").unwrap(), None);
        assert_eq!(s.register_acceptor_identity("FILE:/etc/two.keytab").unwrap().unwrap().path(), "/etc/one.keytab");
        assert_eq!(s.acceptor_keytab().unwrap().path(), "/etc/two.keytab");
    }
}
