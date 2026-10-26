//! Entitlements: the capabilities a package asks the system for, as a file rather than a list in the manifest.
//!
//! An entitlements file is a property list (XML, binary or the text form) with one dictionary. A key is one of
//! the names in [`KNOWN`]: a flag (`true` or `false`) for the permissions the sandbox grants (`network`, `storage`,
//! `audio`, `notifications`, `documents`, `keychain`, `accessibility`, `display-config`, `sensors`, `contacts`, and
//! the first-party capture and system entitlements `camera`, `screen`, `remote-desktop`, `log-store`, `system-monitor`, `hardware-info`), or a list of
//! reverse-DNS group names (`application-groups`, `keychain-access-groups`). An unknown key, a value of the wrong
//! type, a key that appears twice and a file larger than [`MAX_BYTES`] are refused, so a typo never becomes a
//! silent denial and a hostile file cannot grow without bound.
//!
//! [`Entitlements::check_for`] applies the rule the shell applies today: a first-party-only entitlement is honoured
//! only for a package whose id starts with `is.olibuijr.`, and `log-store`, `system-monitor` and `hardware-info` also need the `system-app` kind. The
//! result lists the manifest names the sandbox policy takes ([`Entitlements::held_names`]).
//!
//! # Signing
//!
//! [`Entitlements::sign`] writes the signed form: `BENT`, a version byte, the signature length as a u32, an ML-DSA
//! signature ([`crate::signature_pq`]), then the canonical bytes of the entitlements. The signature covers the bundle
//! id and the canonical bytes, so a signed file does not verify for another package and the signature does not depend
//! on how the plist was written. [`Entitlements::verify_signed`] checks it against a public key the caller trusts.
//! Which keys the shell trusts, and how a package gets its file signed, is not decided here: this build has no signing
//! service and no provisioned trust root.
//!
//! ```ignore
//! let ent = Entitlements::from_plist_bytes(&std::fs::read("entitlements.plist")?)?;
//! ent.check_for("is.olibuijr.notes", "app")?;
//! let signed = ent.sign("is.olibuijr.notes", &private_key)?;
//! let ent = Entitlements::verify_signed(&signed, "is.olibuijr.notes", &public_key)?;
//! ```

use crate::signature_pq::{PrivateKey, PublicKey};
use bifrost_core::plist::{self, Plist};

/// The largest entitlements file, in bytes.
pub const MAX_BYTES: usize = 64 * 1024;
/// The most keys in one file.
pub const MAX_KEYS: usize = 64;
/// The most entries in one list.
pub const MAX_LIST: usize = 64;
/// The first bytes of a signed entitlements file.
pub const SIGNED_MAGIC: &[u8; 4] = b"BENT";
const VERSION: u8 = 1;
const MAX_SIGNATURE: usize = 8192;
const SIGN_CONTEXT: &[u8] = b"bifrost-entitlements/v1";
/// Package ids that may hold the first-party-only entitlements.
pub const FIRST_PARTY_PREFIX: &str = "is.olibuijr.";
/// The package kind that may hold `log-store`.
pub const SYSTEM_APP_KIND: &str = "system-app";

/// What a key holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Flag,
    List,
}

/// A key the system knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Known {
    pub key: &'static str,
    pub shape: Shape,
    /// Only a package whose id starts with [`FIRST_PARTY_PREFIX`] may hold it.
    pub first_party_only: bool,
}

const fn flag(key: &'static str, first_party_only: bool) -> Known {
    Known { key, shape: Shape::Flag, first_party_only }
}

/// Every key an entitlements file may hold.
pub const KNOWN: &[Known] = &[
    flag("network", false),
    flag("storage", false),
    flag("audio", false),
    flag("notifications", false),
    flag("documents", false),
    flag("keychain", false),
    flag("accessibility", false),
    flag("display-config", false),
    flag("sensors", false),
    flag("contacts", false),
    flag("calendars", false),
    flag("camera", true),
    flag("screen", true),
    flag("remote-desktop", true),
    flag("log-store", true),
    flag("system-monitor", true),
    flag("hardware-info", true),
    Known { key: "application-groups", shape: Shape::List, first_party_only: false },
    Known { key: "keychain-access-groups", shape: Shape::List, first_party_only: false },
];

fn known(key: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.key == key)
}

/// A value in an entitlements file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Flag(bool),
    List(Vec<String>),
}

/// Why an entitlements file or signature was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file is larger than [`MAX_BYTES`] or holds more than [`MAX_KEYS`] keys or [`MAX_LIST`] list entries.
    TooBig,
    /// The plist does not parse, or its top level is not a dictionary. The text is the parser's.
    Malformed(String),
    /// A key the system does not know.
    UnknownKey(String),
    /// A key with a value of the wrong type, or a list entry that is not a reverse-DNS name.
    WrongType(String),
    /// A key that appears twice.
    Duplicate(String),
    /// A first-party-only entitlement on a package that may not hold it.
    NotPermitted(String),
    /// The signature is missing, damaged, made by another key, or made for another package.
    BadSignature,
    /// The signed form names a version this module does not read.
    UnsupportedVersion(u8),
    /// Signing failed (the random pool is unavailable).
    SigningFailed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooBig => f.write_str("the entitlements file is too large"),
            Error::Malformed(why) => write!(f, "the entitlements file is malformed: {why}"),
            Error::UnknownKey(k) => write!(f, "unknown entitlement {k}"),
            Error::WrongType(k) => write!(f, "entitlement {k} has a value of the wrong type"),
            Error::Duplicate(k) => write!(f, "entitlement {k} appears twice"),
            Error::NotPermitted(k) => write!(f, "this package may not hold the entitlement {k}"),
            Error::BadSignature => f.write_str("the entitlements signature does not match"),
            Error::UnsupportedVersion(v) => write!(f, "signed entitlements version {v} is not supported"),
            Error::SigningFailed => f.write_str("signing failed"),
        }
    }
}

/// A checked set of entitlements, sorted by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entitlements {
    entries: Vec<(String, Value)>,
}

/// A group name: reverse-DNS labels of letters, digits and hyphens, at most 255 bytes, optionally after `group.`.
fn is_group_name(name: &str) -> bool {
    let body = name.strip_prefix("group.").unwrap_or(name);
    let labels: Vec<&str> = body.split('.').collect();
    name.len() <= 255
        && labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

impl Entitlements {
    /// Builds a set from values, applying the same checks as the file reader.
    pub fn from_entries(entries: Vec<(String, Value)>) -> Result<Entitlements, Error> {
        if entries.len() > MAX_KEYS {
            return Err(Error::TooBig);
        }
        let mut checked: Vec<(String, Value)> = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let info = known(&key).ok_or_else(|| Error::UnknownKey(key.clone()))?;
            match (&info.shape, &value) {
                (Shape::Flag, Value::Flag(_)) => {}
                (Shape::List, Value::List(items)) => {
                    if items.len() > MAX_LIST {
                        return Err(Error::TooBig);
                    }
                    if !items.iter().all(|i| is_group_name(i)) {
                        return Err(Error::WrongType(key));
                    }
                }
                _ => return Err(Error::WrongType(key)),
            }
            if checked.iter().any(|(k, _)| *k == key) {
                return Err(Error::Duplicate(key));
            }
            checked.push((key, value));
        }
        checked.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Entitlements { entries: checked })
    }

    /// Reads an entitlements plist.
    pub fn from_plist_bytes(bytes: &[u8]) -> Result<Entitlements, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::TooBig);
        }
        let Plist::Dict(items) = plist::parse_bytes(bytes).map_err(Error::Malformed)? else {
            return Err(Error::Malformed("the top level is not a dictionary".to_string()));
        };
        let mut entries = Vec::with_capacity(items.len());
        for (key, value) in items {
            let converted = match value {
                Plist::Bool(b) => Value::Flag(b),
                Plist::Array(list) => {
                    let mut names = Vec::with_capacity(list.len());
                    for item in list {
                        match item {
                            Plist::Str(s) => names.push(s),
                            _ => return Err(Error::WrongType(key)),
                        }
                    }
                    Value::List(names)
                }
                _ => return Err(Error::WrongType(key)),
            };
            entries.push((key, converted));
        }
        Entitlements::from_entries(entries)
    }

    /// Every key and value, sorted by key.
    pub fn entries(&self) -> &[(String, Value)] {
        &self.entries
    }

    /// Whether the flag `key` is present and true. A missing key, a false flag and a list key are all false.
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.entries.iter().find(|(k, _)| k == key), Some((_, Value::Flag(true))))
    }

    /// The list `key`, empty when the key is missing or is a flag.
    pub fn list(&self, key: &str) -> &[String] {
        match self.entries.iter().find(|(k, _)| k == key) {
            Some((_, Value::List(items))) => items,
            _ => &[],
        }
    }

    /// The names of the flags that are true, in key order: the list the sandbox policy takes as manifest permissions.
    pub fn held_names(&self) -> Vec<&str> {
        self.entries.iter().filter(|(_, v)| *v == Value::Flag(true)).map(|(k, _)| k.as_str()).collect()
    }

    /// Whether the package with `bundle_id` and `kind` may hold what this file asks for. Only true flags need
    /// the right: a false flag asks for nothing.
    pub fn check_for(&self, bundle_id: &str, kind: &str) -> Result<(), Error> {
        let first_party = bundle_id.starts_with(FIRST_PARTY_PREFIX);
        for (key, value) in &self.entries {
            let asks = match value {
                Value::Flag(on) => *on,
                Value::List(items) => !items.is_empty(),
            };
            if !asks {
                continue;
            }
            let info = known(key).ok_or_else(|| Error::UnknownKey(key.clone()))?;
            let needs_system = key == "log-store" || key == "system-monitor" || key == "hardware-info";
            if (info.first_party_only && !first_party) || (needs_system && kind != SYSTEM_APP_KIND) {
                return Err(Error::NotPermitted(key.clone()));
            }
        }
        Ok(())
    }

    /// The canonical bytes: a u16 key count, then for each key (sorted) a u16 name length, the name, a type byte (1 flag,
    /// 2 list) and the value: one byte for a flag, or a u16 count and u16-length-prefixed names for a list.
    pub fn canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        for (key, value) in &self.entries {
            out.extend_from_slice(&(key.len() as u16).to_le_bytes());
            out.extend_from_slice(key.as_bytes());
            match value {
                Value::Flag(on) => {
                    out.push(1);
                    out.push(*on as u8);
                }
                Value::List(items) => {
                    out.push(2);
                    out.extend_from_slice(&(items.len() as u16).to_le_bytes());
                    for item in items {
                        out.extend_from_slice(&(item.len() as u16).to_le_bytes());
                        out.extend_from_slice(item.as_bytes());
                    }
                }
            }
        }
        out
    }

    /// Reads the canonical bytes back. The result passes the same checks as a plist file; trailing bytes are refused.
    pub fn from_canonical(bytes: &[u8]) -> Result<Entitlements, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::TooBig);
        }
        let bad = || Error::Malformed("the canonical form is damaged".to_string());
        let mut pos = 0usize;
        let mut take = |n: usize| -> Result<&[u8], Error> {
            let end = pos.checked_add(n).ok_or_else(bad)?;
            let slice = bytes.get(pos..end).ok_or_else(bad)?;
            pos = end;
            Ok(slice)
        };
        let count = {
            let b = take(2)?;
            u16::from_le_bytes([b[0], b[1]]) as usize
        };
        if count > MAX_KEYS {
            return Err(Error::TooBig);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let len = {
                let b = take(2)?;
                u16::from_le_bytes([b[0], b[1]]) as usize
            };
            let key = String::from_utf8(take(len)?.to_vec()).map_err(|_| bad())?;
            let kind = take(1)?[0];
            let value = match kind {
                1 => match take(1)?[0] {
                    0 => Value::Flag(false),
                    1 => Value::Flag(true),
                    _ => return Err(bad()),
                },
                2 => {
                    let n = {
                        let b = take(2)?;
                        u16::from_le_bytes([b[0], b[1]]) as usize
                    };
                    if n > MAX_LIST {
                        return Err(Error::TooBig);
                    }
                    let mut items = Vec::with_capacity(n);
                    for _ in 0..n {
                        let l = {
                            let b = take(2)?;
                            u16::from_le_bytes([b[0], b[1]]) as usize
                        };
                        items.push(String::from_utf8(take(l)?.to_vec()).map_err(|_| bad())?);
                    }
                    Value::List(items)
                }
                _ => return Err(bad()),
            };
            entries.push((key, value));
        }
        if pos != bytes.len() {
            return Err(bad());
        }
        Entitlements::from_entries(entries)
    }

    /// The message a signature covers: the bundle id (length-prefixed) and the canonical bytes.
    fn signed_message(&self, bundle_id: &str) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(&(bundle_id.len() as u32).to_le_bytes());
        message.extend_from_slice(bundle_id.as_bytes());
        message.extend_from_slice(&self.canonical());
        message
    }

    /// Signs these entitlements for the package `bundle_id`.
    pub fn sign(&self, bundle_id: &str, key: &PrivateKey) -> Result<Vec<u8>, Error> {
        let signature = key.sign(&self.signed_message(bundle_id), SIGN_CONTEXT).map_err(|_| Error::SigningFailed)?;
        let canonical = self.canonical();
        let mut out = Vec::with_capacity(9 + signature.len() + canonical.len());
        out.extend_from_slice(SIGNED_MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&(signature.len() as u32).to_le_bytes());
        out.extend_from_slice(&signature);
        out.extend_from_slice(&canonical);
        Ok(out)
    }

    /// Checks a signed file against `key` for the package `bundle_id` and returns the entitlements in it. The
    /// signature is checked before the contents are read.
    pub fn verify_signed(signed: &[u8], bundle_id: &str, key: &PublicKey) -> Result<Entitlements, Error> {
        if signed.len() < 9 || !signed.starts_with(SIGNED_MAGIC) {
            return Err(Error::BadSignature);
        }
        if signed[4] != VERSION {
            return Err(Error::UnsupportedVersion(signed[4]));
        }
        let sig_len = u32::from_le_bytes([signed[5], signed[6], signed[7], signed[8]]) as usize;
        if sig_len > MAX_SIGNATURE {
            return Err(Error::BadSignature);
        }
        let end = 9usize.checked_add(sig_len).ok_or(Error::BadSignature)?;
        let signature = signed.get(9..end).ok_or(Error::BadSignature)?;
        let canonical = &signed[end..];
        let parsed = Entitlements::from_canonical(canonical).map_err(|_| Error::BadSignature)?;
        if key.is_valid_signature(signature, &parsed.signed_message(bundle_id), SIGN_CONTEXT) && parsed.canonical() == canonical {
            Ok(parsed)
        } else {
            Err(Error::BadSignature)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature_pq::Variant;

    const FILE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>network</key><true/>
<key>documents</key><true/>
<key>keychain</key><false/>
<key>application-groups</key><array><string>group.is.olibuijr.suite</string></array>
</dict></plist>"#;

    fn parsed() -> Entitlements {
        Entitlements::from_plist_bytes(FILE.as_bytes()).unwrap()
    }

    #[test]
    fn a_file_reads_flags_and_lists_sorted_by_key() {
        let e = parsed();
        let keys: Vec<&str> = e.entries().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["application-groups", "documents", "keychain", "network"]);
        assert!(e.flag("network") && e.flag("documents"));
        assert!(!e.flag("keychain"), "a false flag is not held");
        assert!(!e.flag("storage") && !e.flag("application-groups"));
        assert_eq!(e.list("application-groups"), ["group.is.olibuijr.suite"]);
        assert!(e.list("network").is_empty() && e.list("nothing").is_empty());
        assert_eq!(e.held_names(), ["documents", "network"]);
    }

    #[test]
    fn a_binary_plist_reads_the_same() {
        let dict = Plist::Dict(vec![("sensors".into(), Plist::Bool(true))]);
        let bytes = bifrost_core::plist_binary::write(&dict).unwrap();
        assert!(Entitlements::from_plist_bytes(&bytes).unwrap().flag("sensors"));
    }

    #[test]
    fn unknown_keys_wrong_types_and_duplicates_are_refused() {
        let wrap = |body: &str| format!("<plist version=\"1.0\"><dict>{body}</dict></plist>");
        assert_eq!(Entitlements::from_plist_bytes(wrap("<key>nework</key><true/>").as_bytes()), Err(Error::UnknownKey("nework".into())));
        assert_eq!(Entitlements::from_plist_bytes(wrap("<key>network</key><string>yes</string>").as_bytes()), Err(Error::WrongType("network".into())));
        assert_eq!(Entitlements::from_plist_bytes(wrap("<key>application-groups</key><true/>").as_bytes()), Err(Error::WrongType("application-groups".into())));
        assert_eq!(Entitlements::from_plist_bytes(wrap("<key>application-groups</key><array><integer>1</integer></array>").as_bytes()), Err(Error::WrongType("application-groups".into())));
        assert_eq!(Entitlements::from_plist_bytes(wrap("<key>application-groups</key><array><string>../etc</string></array>").as_bytes()), Err(Error::WrongType("application-groups".into())));
        assert_eq!(Entitlements::from_plist_bytes(wrap("<key>audio</key><true/><key>audio</key><false/>").as_bytes()), Err(Error::Duplicate("audio".into())));
        assert!(matches!(Entitlements::from_plist_bytes(b"<plist version=\"1.0\"><array/></plist>"), Err(Error::Malformed(_))));
        assert!(matches!(Entitlements::from_plist_bytes(b"garbage {"), Err(Error::Malformed(_))));
    }

    #[test]
    fn size_limits_are_enforced() {
        assert_eq!(Entitlements::from_plist_bytes(&vec![b' '; MAX_BYTES + 1]), Err(Error::TooBig));
        let many: Vec<String> = (0..MAX_LIST + 1).map(|i| format!("group.is.olibuijr.g{i}")).collect();
        assert_eq!(Entitlements::from_entries(vec![("application-groups".into(), Value::List(many))]), Err(Error::TooBig));
        let keys: Vec<(String, Value)> = (0..MAX_KEYS + 1).map(|_| ("network".to_string(), Value::Flag(true))).collect();
        assert_eq!(Entitlements::from_entries(keys), Err(Error::TooBig));
    }

    #[test]
    fn first_party_entitlements_need_a_first_party_package() {
        let e = Entitlements::from_entries(vec![("camera".into(), Value::Flag(true)), ("network".into(), Value::Flag(true))]).unwrap();
        assert_eq!(e.check_for("is.olibuijr.photobooth.desktop", "app"), Ok(()));
        assert_eq!(e.check_for("com.example.cam", "app"), Err(Error::NotPermitted("camera".into())));
        let off = Entitlements::from_entries(vec![("camera".into(), Value::Flag(false))]).unwrap();
        assert_eq!(off.check_for("com.example.cam", "app"), Ok(()), "a false flag asks for nothing");
    }

    #[test]
    fn the_log_store_entitlement_also_needs_the_system_app_kind() {
        let e = Entitlements::from_entries(vec![("log-store".into(), Value::Flag(true))]).unwrap();
        assert_eq!(e.check_for("is.olibuijr.console.desktop", SYSTEM_APP_KIND), Ok(()));
        assert_eq!(e.check_for("is.olibuijr.console.desktop", "app"), Err(Error::NotPermitted("log-store".into())));
        assert_eq!(e.check_for("com.example.logs", SYSTEM_APP_KIND), Err(Error::NotPermitted("log-store".into())));
    }

    #[test]
    fn the_canonical_form_round_trips_and_ignores_file_order() {
        let e = parsed();
        assert_eq!(Entitlements::from_canonical(&e.canonical()).unwrap(), e);
        let shuffled = "<plist version=\"1.0\"><dict><key>network</key><true/><key>keychain</key><false/><key>application-groups</key><array><string>group.is.olibuijr.suite</string></array><key>documents</key><true/></dict></plist>";
        assert_eq!(Entitlements::from_plist_bytes(shuffled.as_bytes()).unwrap().canonical(), e.canonical());
        let mut extra = e.canonical();
        extra.push(0);
        assert!(Entitlements::from_canonical(&extra).is_err());
        for n in 0..e.canonical().len() {
            assert!(Entitlements::from_canonical(&e.canonical()[..n]).is_err(), "cut at {n}");
        }
    }

    #[test]
    fn a_signed_file_verifies_only_for_its_key_and_its_package() {
        let key = PrivateKey::from_seed(Variant::MlDsa65, &[1u8; 32]);
        let other = PrivateKey::from_seed(Variant::MlDsa65, &[2u8; 32]);
        let signed = parsed().sign("is.olibuijr.notes", &key).unwrap();
        assert_eq!(&signed[..4], SIGNED_MAGIC);
        assert_eq!(Entitlements::verify_signed(&signed, "is.olibuijr.notes", &key.public_key()).unwrap(), parsed());
        assert_eq!(Entitlements::verify_signed(&signed, "is.olibuijr.other", &key.public_key()), Err(Error::BadSignature), "bound to the package id");
        assert_eq!(Entitlements::verify_signed(&signed, "is.olibuijr.notes", &other.public_key()), Err(Error::BadSignature));
    }

    #[test]
    fn a_changed_signed_file_is_refused_at_every_byte() {
        let key = PrivateKey::from_seed(Variant::MlDsa65, &[1u8; 32]);
        let signed = Entitlements::from_entries(vec![("network".into(), Value::Flag(false))]).unwrap().sign("is.olibuijr.a", &key).unwrap();
        let public = key.public_key();
        // flipping the network flag from false to true must not verify
        let last = signed.len() - 1;
        let mut flipped = signed.clone();
        flipped[last] = 1;
        assert_eq!(Entitlements::verify_signed(&flipped, "is.olibuijr.a", &public), Err(Error::BadSignature));
        for i in (0..signed.len()).step_by(97).chain([0, 4, 5, last]) {
            let mut bad = signed.clone();
            bad[i] ^= 0x01;
            assert!(Entitlements::verify_signed(&bad, "is.olibuijr.a", &public).is_err(), "byte {i}");
        }
        for n in [0usize, 4, 8, 9, 100, last] {
            assert!(Entitlements::verify_signed(&signed[..n], "is.olibuijr.a", &public).is_err(), "cut at {n}");
        }
        let mut huge = signed;
        huge[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Entitlements::verify_signed(&huge, "is.olibuijr.a", &public), Err(Error::BadSignature));
    }

    #[test]
    fn a_signed_version_from_the_future_is_named() {
        let key = PrivateKey::from_seed(Variant::MlDsa65, &[1u8; 32]);
        let mut signed = parsed().sign("is.olibuijr.a", &key).unwrap();
        signed[4] = 9;
        assert_eq!(Entitlements::verify_signed(&signed, "is.olibuijr.a", &key.public_key()), Err(Error::UnsupportedVersion(9)));
    }

    #[test]
    fn every_known_key_is_listed_once_and_the_first_party_ones_are_named() {
        let mut keys: Vec<&str> = KNOWN.iter().map(|k| k.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), KNOWN.len());
        let first_party: Vec<&str> = KNOWN.iter().filter(|k| k.first_party_only).map(|k| k.key).collect();
        assert_eq!(first_party, ["camera", "screen", "remote-desktop", "log-store", "system-monitor", "hardware-info"]);
    }
}
