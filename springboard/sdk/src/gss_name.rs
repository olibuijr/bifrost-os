//! GSS-API name handling (`GSSCreateName`, `gss_import_name`, `gss_display_name`, `gss_compare_name`,
//! `gss_canonicalize_name`, `GSSNameCreateDisplayString`): names that identify a principal such as a user or a
//! network service, with the operations an application needs to read a name, match it and store it.
//!
//! Two name types are modelled. A user name is an opaque string. A host-based service name has the form
//! `service@host`, or only `service` when no host is given. The host part is compared without regard to ASCII
//! case, so the canonical form lowers it, and two names are equal when their canonical forms match. The service
//! part stays case-sensitive. The name type identifiers are the object identifiers RFC 2744 section 4 defines; the
//! reference page names the functions and the types but does not print the identifiers, so they follow that RFC,
//! as `gss_status` does for its field widths.
//!
//! Not here: the mechanism that maps an internal name to a mechanism name (`gss_export_name` token form), and the
//! anonymous name type. The release calls are replaced by drop.

use crate::gss_oid::Oid;
use crate::gss_status::RoutineError;
use std::fmt;

/// The name types this model reads, as `GSS_C_NT_USER_NAME` and `GSS_C_NT_HOSTBASED_SERVICE` (RFC 2744 section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameType {
    UserName,
    HostBasedService,
}

impl NameType {
    /// The object identifier that names this type.
    pub fn oid(self) -> Oid {
        let text = match self {
            NameType::UserName => "1.2.840.113554.1.2.1.1",
            NameType::HostBasedService => "1.2.840.113554.1.2.1.4",
        };
        // The constants above are valid identifiers, so the parse cannot fail.
        Oid::parse(text).expect("name type identifiers are valid")
    }

    /// The name type that an identifier names, or `None` for an identifier this model does not read.
    pub fn from_oid(oid: &Oid) -> Option<NameType> {
        [NameType::UserName, NameType::HostBasedService]
            .into_iter()
            .find(|t| t.oid() == *oid)
    }
}

/// Why a name cannot be imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    /// The bytes are empty, not UTF-8, or a host-based name has an empty service or host, or a second `@`.
    BadName,
    /// The name type identifier is not one this model reads.
    BadNameType,
}

impl NameError {
    /// The GSS routine error this failure reports (`GSS_S_BAD_NAME` or `GSS_S_BAD_NAMETYPE`).
    pub fn routine_error(self) -> RoutineError {
        match self {
            NameError::BadName => RoutineError::BadName,
            NameError::BadNameType => RoutineError::BadNameType,
        }
    }
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NameError::BadName => "name is empty or malformed for its name type",
            NameError::BadNameType => "name type is not supported",
        })
    }
}

impl std::error::Error for NameError {}

/// An imported name. For a user name `text` is the whole name and `host` is `None`. For a host-based service name
/// `text` is the service and `host` is the host when one was given.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Name {
    kind: NameType,
    text: String,
    host: Option<String>,
}

impl Name {
    /// `gss_import_name` with a known name type. Empty input is refused for both types.
    pub fn import(bytes: &[u8], kind: NameType) -> Result<Name, NameError> {
        let text = std::str::from_utf8(bytes).map_err(|_| NameError::BadName)?;
        if text.is_empty() {
            return Err(NameError::BadName);
        }
        match kind {
            NameType::UserName => Ok(Name { kind, text: text.to_string(), host: None }),
            NameType::HostBasedService => {
                let (service, host) = match text.split_once('@') {
                    Some((service, host)) => (service, Some(host)),
                    None => (text, None),
                };
                if service.is_empty() || host.map_or(false, |h| h.is_empty() || h.contains('@')) {
                    return Err(NameError::BadName);
                }
                Ok(Name {
                    kind,
                    text: service.to_string(),
                    host: host.map(|h| h.to_string()),
                })
            }
        }
    }

    /// `gss_import_name` with a name type identifier. An identifier this model does not read is refused.
    pub fn import_typed(bytes: &[u8], name_type: &Oid) -> Result<Name, NameError> {
        let kind = NameType::from_oid(name_type).ok_or(NameError::BadNameType)?;
        Name::import(bytes, kind)
    }

    /// The name type of this name.
    pub fn name_type(&self) -> NameType {
        self.kind
    }

    /// The host part, when the name has one.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// `GSSNameCreateDisplayString` and the text half of `gss_display_name`: `service@host`, or the user name.
    pub fn display(&self) -> String {
        match &self.host {
            Some(host) => format!("{}@{}", self.text, host),
            None => self.text.clone(),
        }
    }

    /// `gss_canonicalize_name`: the same name with its host lowered to ASCII. The service and user name are kept.
    pub fn canonicalize(&self) -> Name {
        Name {
            kind: self.kind,
            text: self.text.clone(),
            host: self.host.as_ref().map(|h| h.to_ascii_lowercase()),
        }
    }

    /// `gss_compare_name`: true when both names have the same type and the same canonical form.
    pub fn compare(&self, other: &Name) -> bool {
        self.canonicalize() == other.canonicalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_name_displays_as_it_was_imported() {
        let n = Name::import(b"olibuijr", NameType::UserName).unwrap();
        assert_eq!(n.display(), "olibuijr");
        assert_eq!(n.host(), None);
        assert_eq!(n.name_type(), NameType::UserName);
    }

    #[test]
    fn a_host_based_name_displays_as_service_at_host() {
        let n = Name::import(b"ftp@files.example.org", NameType::HostBasedService).unwrap();
        assert_eq!(n.display(), "ftp@files.example.org");
        assert_eq!(n.host(), Some("files.example.org"));
    }

    #[test]
    fn a_service_without_a_host_is_valid() {
        let n = Name::import(b"ftp", NameType::HostBasedService).unwrap();
        assert_eq!(n.host(), None);
        assert_eq!(n.display(), "ftp");
    }

    #[test]
    fn host_case_is_ignored_and_service_case_is_not() {
        let a = Name::import(b"ftp@Files.Example.org", NameType::HostBasedService).unwrap();
        let b = Name::import(b"ftp@files.example.org", NameType::HostBasedService).unwrap();
        let c = Name::import(b"FTP@files.example.org", NameType::HostBasedService).unwrap();
        assert!(a.compare(&b));
        assert!(!a.compare(&c));
        assert_eq!(a.canonicalize().host(), Some("files.example.org"));
        // Canonicalizing keeps the service text as given.
        assert_eq!(a.canonicalize().display(), "ftp@files.example.org");
    }

    #[test]
    fn user_names_compare_exactly_and_types_must_match() {
        let u = Name::import(b"bob", NameType::UserName).unwrap();
        let v = Name::import(b"Bob", NameType::UserName).unwrap();
        let s = Name::import(b"bob", NameType::HostBasedService).unwrap();
        assert!(!u.compare(&v));
        assert!(!u.compare(&s));
        assert!(u.compare(&u.clone()));
    }

    #[test]
    fn malformed_names_are_refused() {
        assert_eq!(Name::import(b"", NameType::UserName), Err(NameError::BadName));
        assert_eq!(Name::import(b"@host", NameType::HostBasedService), Err(NameError::BadName));
        assert_eq!(Name::import(b"ftp@", NameType::HostBasedService), Err(NameError::BadName));
        assert_eq!(Name::import(b"a@b@c", NameType::HostBasedService), Err(NameError::BadName));
        assert_eq!(Name::import(&[0xFF, 0xFE], NameType::UserName), Err(NameError::BadName));
    }

    #[test]
    fn an_unknown_name_type_identifier_is_refused() {
        let unknown = Oid::parse("1.2.3").unwrap();
        assert_eq!(Name::import_typed(b"x", &unknown), Err(NameError::BadNameType));
        let known = NameType::HostBasedService.oid();
        assert_eq!(known.to_dotted(), "1.2.840.113554.1.2.1.4");
        assert!(Name::import_typed(b"ftp@h", &known).is_ok());
        assert_eq!(NameError::BadNameType.routine_error(), RoutineError::BadNameType);
        assert_eq!(NameError::BadName.routine_error(), RoutineError::BadName);
    }
}
