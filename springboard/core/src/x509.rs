//! X.509 certificates (RFC 5280): parse the DER of a certificate into its fields, and assemble one (std only).
//!
//! `parse` reads the certificate with the strict reader in `crate::der` and returns the serial number, the issuer and
//! subject names, the validity window in Unix seconds, the public key info, the extensions and the bytes a signature
//! covers (`tbs`) with the signature itself. It refuses a certificate whose two signature algorithm fields differ,
//! whose validity runs backwards, whose extensions repeat, or that is not DER. Typed readers cover the extensions the
//! trust decision needs: basic constraints, key usage and the subject alternative names.
//!
//! Not here: checking a signature. The TLS stack (rustls) checks signatures and chains for connections; the chain
//! rules apps can run over parsed fields are `bifrost_app::trust`, which builds from a certificate parsed here.
//! `encode` assembles a certificate around a signature the caller computed with a signing key, which is how a local
//! authority or a test fixture is made.

use crate::der::{self, DerError, Tlv};
use crate::oid::Oid;
use crate::sha256;
use std::fmt;

/// Why a certificate was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum X509Error {
    Der(DerError),
    /// The version field is not v1, v2 or v3.
    BadVersion,
    /// The signature algorithm in the signed body differs from the one beside the signature.
    SignatureAlgorithmMismatch,
    /// The validity window ends before it starts.
    BadValidity,
    /// The same extension appears twice.
    DuplicateExtension(String),
    /// Extensions in a certificate that is not v3.
    ExtensionsBeforeV3,
    /// An extension's value does not have the shape its type defines.
    BadExtension(&'static str),
    /// The signature bit string has unused bits.
    BadSignature,
    /// An attribute name that is neither a known short name nor a dotted identifier.
    UnknownAttribute(String),
}

impl fmt::Display for X509Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            X509Error::Der(e) => write!(f, "not valid DER: {e}"),
            X509Error::BadVersion => f.write_str("unknown certificate version"),
            X509Error::SignatureAlgorithmMismatch => f.write_str("the two signature algorithms differ"),
            X509Error::BadValidity => f.write_str("the validity window ends before it starts"),
            X509Error::DuplicateExtension(oid) => write!(f, "extension {oid} appears twice"),
            X509Error::ExtensionsBeforeV3 => f.write_str("extensions need a version 3 certificate"),
            X509Error::BadExtension(which) => write!(f, "malformed {which} extension"),
            X509Error::BadSignature => f.write_str("the signature bit string has unused bits"),
            X509Error::UnknownAttribute(name) => write!(f, "unknown name attribute {name}"),
        }
    }
}

impl std::error::Error for X509Error {}

impl From<DerError> for X509Error {
    fn from(e: DerError) -> Self {
        X509Error::Der(e)
    }
}

/// One attribute of a distinguished name, such as a common name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub oid: Oid,
    pub value: String,
}

/// A distinguished name: relative names in encoded order (most general first), each of one or more attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub rdns: Vec<Vec<Attribute>>,
    /// The DER of the name, which is what "issued by" compares.
    pub raw: Vec<u8>,
}

/// The short names of the common attribute types.
const SHORT_NAMES: [(&str, &[u64]); 10] = [
    ("CN", &[2, 5, 4, 3]),
    ("SERIALNUMBER", &[2, 5, 4, 5]),
    ("C", &[2, 5, 4, 6]),
    ("L", &[2, 5, 4, 7]),
    ("ST", &[2, 5, 4, 8]),
    ("O", &[2, 5, 4, 10]),
    ("OU", &[2, 5, 4, 11]),
    ("DC", &[0, 9, 2342, 19200300, 100, 1, 25]),
    ("EMAIL", &[1, 2, 840, 113549, 1, 9, 1]),
    ("UID", &[0, 9, 2342, 19200300, 100, 1, 1]),
];

fn attribute_oid(name: &str) -> Result<Oid, X509Error> {
    if let Some((_, arcs)) = SHORT_NAMES.iter().find(|(short, _)| short.eq_ignore_ascii_case(name)) {
        return Oid::from_arcs(arcs.to_vec()).map_err(|e| X509Error::Der(DerError::Oid(e)));
    }
    Oid::parse(name).map_err(|_| X509Error::UnknownAttribute(name.to_string()))
}

fn escape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for (i, c) in v.chars().enumerate() {
        if matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';') || (i == 0 && (c == '#' || c == ' ')) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Name {
    fn parse(tlv: &Tlv<'_>) -> Result<Name, X509Error> {
        let mut rdns = Vec::new();
        let mut list = tlv.reader()?;
        while !list.is_empty() {
            let set = list.expect(der::SET)?;
            let mut atvs = set.reader()?;
            let mut group = Vec::new();
            while !atvs.is_empty() {
                let mut atv = atvs.expect(der::SEQUENCE)?.reader()?;
                let oid = atv.expect(der::OBJECT_IDENTIFIER)?.as_oid()?;
                let value = atv.read()?;
                atv.finish()?;
                group.push(Attribute { oid, value: attribute_text(&value) });
            }
            if group.is_empty() {
                return Err(DerError::Truncated.into());
            }
            rdns.push(group);
        }
        Ok(Name { rdns, raw: tlv.raw.to_vec() })
    }

    /// The first value of the attribute with dotted identifier `arcs`.
    pub fn first(&self, arcs: &[u64]) -> Option<&str> {
        self.rdns.iter().flatten().find(|a| a.oid.arcs() == arcs).map(|a| a.value.as_str())
    }

    pub fn common_name(&self) -> Option<&str> {
        self.first(&[2, 5, 4, 3])
    }

    pub fn is_empty(&self) -> bool {
        self.rdns.is_empty()
    }
}

/// The text of an attribute value: a string type as text, a BMPString decoded, anything else as `#` and hex.
fn attribute_text(v: &Tlv<'_>) -> String {
    if let Ok(s) = v.as_string() {
        return s.to_string();
    }
    if v.tag == 0x1e && v.content.len() % 2 == 0 {
        let units: Vec<u16> = v.content.chunks(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
        if let Ok(s) = String::from_utf16(&units) {
            return s;
        }
    }
    format!("#{}", sha256::hex(v.raw))
}

/// RFC 4514 form: the most specific relative name first, attributes joined by `+`, relative names by `,`.
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for group in self.rdns.iter().rev() {
            if !first {
                f.write_str(",")?;
            }
            first = false;
            for (i, a) in group.iter().enumerate() {
                if i > 0 {
                    f.write_str("+")?;
                }
                let short = SHORT_NAMES.iter().find(|(_, arcs)| *arcs == a.oid.arcs()).map(|(s, _)| (*s).to_string());
                write!(f, "{}={}", short.unwrap_or_else(|| a.oid.to_string()), escape_value(&a.value))?;
            }
        }
        Ok(())
    }
}

/// One extension: its type, whether a reader that does not know it must refuse the certificate, and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub oid: Oid,
    pub critical: bool,
    /// The extnValue octets (itself DER).
    pub value: Vec<u8>,
}

/// One entry of a subject alternative name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneralName {
    Dns(String),
    Email(String),
    Uri(String),
    /// Four or sixteen address bytes.
    Ip(Vec<u8>),
    /// Any other kind, by its context tag number.
    Other(u8),
}

const OID_KEY_USAGE: [u64; 4] = [2, 5, 29, 15];
const OID_SUBJECT_ALT_NAME: [u64; 4] = [2, 5, 29, 17];
const OID_BASIC_CONSTRAINTS: [u64; 4] = [2, 5, 29, 19];

/// A parsed certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    /// 1, 2 or 3.
    pub version: u8,
    /// The serial number's content octets.
    pub serial: Vec<u8>,
    pub signature_algorithm: Oid,
    pub issuer: Name,
    pub subject: Name,
    pub not_before: u64,
    pub not_after: u64,
    /// The DER of the SubjectPublicKeyInfo.
    pub spki: Vec<u8>,
    pub extensions: Vec<Extension>,
    /// The signed body, tag and length included.
    pub tbs: Vec<u8>,
    pub signature: Vec<u8>,
    /// The whole certificate.
    pub der: Vec<u8>,
}

/// A signature algorithm identifier: its object identifier and the raw encoding (to compare two of them).
fn algorithm(tlv: &Tlv<'_>) -> Result<(Oid, Vec<u8>), X509Error> {
    let mut r = tlv.reader()?;
    let oid = r.expect(der::OBJECT_IDENTIFIER)?.as_oid()?;
    if !r.is_empty() {
        r.read()?;
    }
    r.finish()?;
    Ok((oid, tlv.raw.to_vec()))
}

pub fn parse(data: &[u8]) -> Result<Certificate, X509Error> {
    let top = der::parse_one(data)?;
    if top.tag != der::SEQUENCE {
        return Err(DerError::UnexpectedTag { expected: der::SEQUENCE, found: top.tag }.into());
    }
    let mut cert = top.reader()?;
    let tbs_tlv = cert.expect(der::SEQUENCE)?;
    let (outer_oid, outer_alg) = algorithm(&cert.expect(der::SEQUENCE)?)?;
    let (unused, signature) = cert.expect(der::BIT_STRING)?.as_bit_string()?;
    if unused != 0 {
        return Err(X509Error::BadSignature);
    }
    cert.finish()?;

    let mut tbs = tbs_tlv.reader()?;
    let version = match tbs.optional(der::context_tag(0, true))? {
        Some(v) => {
            let mut inner = v.reader()?;
            let n = inner.expect(der::INTEGER)?.as_u64()?;
            inner.finish()?;
            if n > 2 {
                return Err(X509Error::BadVersion);
            }
            n as u8 + 1
        }
        None => 1,
    };
    let serial = tbs.expect(der::INTEGER)?;
    if serial.content.is_empty() {
        return Err(DerError::Truncated.into());
    }
    let (_, inner_alg) = algorithm(&tbs.expect(der::SEQUENCE)?)?;
    if inner_alg != outer_alg {
        return Err(X509Error::SignatureAlgorithmMismatch);
    }
    let issuer = Name::parse(&tbs.expect(der::SEQUENCE)?)?;
    let mut validity = tbs.expect(der::SEQUENCE)?.reader()?;
    let not_before = validity.read()?.as_time()?;
    let not_after = validity.read()?.as_time()?;
    validity.finish()?;
    if not_before > not_after {
        return Err(X509Error::BadValidity);
    }
    let subject = Name::parse(&tbs.expect(der::SEQUENCE)?)?;
    let spki = tbs.expect(der::SEQUENCE)?.raw.to_vec();
    tbs.optional(der::context_tag(1, false))?;
    tbs.optional(der::context_tag(2, false))?;

    let mut extensions: Vec<Extension> = Vec::new();
    if let Some(wrapper) = tbs.optional(der::context_tag(3, true))? {
        if version != 3 {
            return Err(X509Error::ExtensionsBeforeV3);
        }
        let mut outer = wrapper.reader()?;
        let mut list = outer.sequence()?;
        outer.finish()?;
        while !list.is_empty() {
            let mut x = list.sequence()?;
            let oid = x.expect(der::OBJECT_IDENTIFIER)?.as_oid()?;
            let critical = match x.optional(der::BOOLEAN)? {
                Some(b) => b.as_bool()?,
                None => false,
            };
            let value = x.expect(der::OCTET_STRING)?.as_octets()?.to_vec();
            x.finish()?;
            if extensions.iter().any(|e| e.oid == oid) {
                return Err(X509Error::DuplicateExtension(oid.to_string()));
            }
            extensions.push(Extension { oid, critical, value });
        }
    }
    tbs.finish()?;

    Ok(Certificate {
        version,
        serial: serial.content.to_vec(),
        signature_algorithm: outer_oid,
        issuer,
        subject,
        not_before,
        not_after,
        spki,
        extensions,
        tbs: tbs_tlv.raw.to_vec(),
        signature: signature.to_vec(),
        der: data.to_vec(),
    })
}

impl Certificate {
    /// The extension with identifier `arcs`.
    pub fn extension(&self, arcs: &[u64]) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.oid.arcs() == arcs)
    }

    /// The basic constraints: `(is a certificate authority, longest chain below it)`, or None when absent.
    pub fn basic_constraints(&self) -> Result<Option<(bool, Option<u64>)>, X509Error> {
        let Some(ext) = self.extension(&OID_BASIC_CONSTRAINTS) else { return Ok(None) };
        let bad = |_| X509Error::BadExtension("basic constraints");
        let top = der::parse_one(&ext.value).map_err(bad)?;
        let mut seq = der::Reader::new(top.raw).sequence().map_err(bad)?;
        let ca = match seq.optional(der::BOOLEAN).map_err(bad)? {
            Some(b) => b.as_bool().map_err(bad)?,
            None => false,
        };
        let path_len = match seq.optional(der::INTEGER).map_err(bad)? {
            Some(n) => Some(n.as_u64().map_err(bad)?),
            None => None,
        };
        seq.finish().map_err(bad)?;
        Ok(Some((ca, path_len)))
    }

    /// The key usage bits as bytes (bit 0 is the top bit of the first byte), or None when absent.
    pub fn key_usage(&self) -> Result<Option<Vec<u8>>, X509Error> {
        let Some(ext) = self.extension(&OID_KEY_USAGE) else { return Ok(None) };
        let bad = |_| X509Error::BadExtension("key usage");
        let tlv = der::parse_one(&ext.value).map_err(bad)?;
        let (_, bytes) = tlv.as_bit_string().map_err(bad)?;
        Ok(Some(bytes.to_vec()))
    }

    /// True when the certificate may sign other certificates: marked as an authority, and if it states key usage,
    /// with the certificate-signing bit (bit 5) set.
    pub fn can_sign_certificates(&self) -> Result<bool, X509Error> {
        let ca = self.basic_constraints()?.is_some_and(|(ca, _)| ca);
        let usage_ok = match self.key_usage()? {
            Some(bits) => bits.first().is_some_and(|b| b & 0x04 != 0),
            None => true,
        };
        Ok(ca && usage_ok)
    }

    /// The subject alternative names, in order.
    pub fn subject_alt_names(&self) -> Result<Vec<GeneralName>, X509Error> {
        let Some(ext) = self.extension(&OID_SUBJECT_ALT_NAME) else { return Ok(Vec::new()) };
        let bad = |_| X509Error::BadExtension("subject alternative name");
        let top = der::parse_one(&ext.value).map_err(bad)?;
        let mut list = top.reader().map_err(bad)?;
        if top.tag != der::SEQUENCE {
            return Err(bad(DerError::UnexpectedTag { expected: der::SEQUENCE, found: top.tag }));
        }
        let mut names = Vec::new();
        while !list.is_empty() {
            let n = list.read().map_err(bad)?;
            let text = || std::str::from_utf8(n.content).ok().filter(|s| s.is_ascii()).map(str::to_string);
            names.push(match n.tag {
                0x81 => GeneralName::Email(text().ok_or_else(|| bad(DerError::InvalidString))?),
                0x82 => GeneralName::Dns(text().ok_or_else(|| bad(DerError::InvalidString))?),
                0x86 => GeneralName::Uri(text().ok_or_else(|| bad(DerError::InvalidString))?),
                0x87 if n.content.len() == 4 || n.content.len() == 16 => GeneralName::Ip(n.content.to_vec()),
                0x87 => return Err(bad(DerError::InvalidString)),
                t => GeneralName::Other(t & 0x1f),
            });
        }
        Ok(names)
    }

    /// The DNS names among the subject alternative names.
    pub fn dns_names(&self) -> Result<Vec<String>, X509Error> {
        Ok(self.subject_alt_names()?.into_iter().filter_map(|n| if let GeneralName::Dns(d) = n { Some(d) } else { None }).collect())
    }

    /// The certificate's SHA-256 fingerprint.
    pub fn fingerprint_sha256(&self) -> [u8; 32] {
        sha256::digest(&self.der)
    }

    /// The SHA-256 of the public key info, which names an issuer for revocation lookups.
    pub fn spki_sha256(&self) -> [u8; 32] {
        sha256::digest(&self.spki)
    }

    /// True when issuer and subject are the same name (a root, or a self-issued certificate).
    pub fn is_self_issued(&self) -> bool {
        self.issuer.raw == self.subject.raw
    }

    /// True when `now` (Unix seconds) is inside the validity window, ends included.
    pub fn is_valid_at(&self, now: u64) -> bool {
        self.not_before <= now && now <= self.not_after
    }
}

// -- Assembling ---------------------------------------------------------------------------------------------

/// What `encode` puts in a version 3 certificate.
pub struct Template<'a> {
    pub serial: &'a [u8],
    /// The signature algorithm, with absent parameters.
    pub signature_algorithm: &'a Oid,
    /// Name attributes as `(short name or dotted identifier, value)`, most general first.
    pub issuer: &'a [(&'a str, &'a str)],
    pub subject: &'a [(&'a str, &'a str)],
    pub not_before: u64,
    pub not_after: u64,
    /// The DER of the SubjectPublicKeyInfo.
    pub spki: &'a [u8],
    pub dns_names: &'a [&'a str],
    /// Adds a critical basic constraints extension marking a certificate authority.
    pub authority: bool,
}

fn encode_name(attrs: &[(&str, &str)]) -> Result<Vec<u8>, X509Error> {
    let mut rdns = Vec::new();
    for (name, value) in attrs {
        let oid = der::oid(&attribute_oid(name)?)?;
        rdns.push(der::set(&[der::sequence(&[oid, der::utf8_string(value)])]));
    }
    Ok(der::sequence(&rdns))
}

fn encode_extension(arcs: &[u64], critical: bool, value: &[u8]) -> Result<Vec<u8>, X509Error> {
    let oid = der::oid(&Oid::from_arcs(arcs.to_vec()).map_err(DerError::Oid)?)?;
    let mut parts = vec![oid];
    if critical {
        parts.push(der::boolean(true));
    }
    parts.push(der::octet_string(value));
    Ok(der::sequence(&parts))
}

/// Assembles the DER of a version 3 certificate around `signature`, which the caller computed over the signed body
/// (`Certificate::tbs` of the result, which does not depend on the signature bytes).
pub fn encode(t: &Template<'_>, signature: &[u8]) -> Result<Vec<u8>, X509Error> {
    let alg = der::sequence(&[der::oid(t.signature_algorithm)?]);
    let mut exts = Vec::new();
    if t.authority {
        exts.push(encode_extension(&OID_BASIC_CONSTRAINTS, true, &der::sequence(&[der::boolean(true)]))?);
    }
    if !t.dns_names.is_empty() {
        let names: Vec<Vec<u8>> = t.dns_names.iter().map(|d| der::encode(0x82, d.as_bytes())).collect();
        exts.push(encode_extension(&OID_SUBJECT_ALT_NAME, false, &der::sequence(&names))?);
    }
    let mut body = vec![
        der::explicit(0, &der::integer_u64(2)),
        der::integer_unsigned(t.serial),
        alg.clone(),
        encode_name(t.issuer)?,
        der::sequence(&[der::time(t.not_before), der::time(t.not_after)]),
        encode_name(t.subject)?,
        t.spki.to_vec(),
    ];
    if !exts.is_empty() {
        body.push(der::explicit(3, &der::sequence(&exts)));
    }
    Ok(der::sequence(&[der::sequence(&body), alg, der::bit_string(0, signature)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot_backup::from_civil;

    fn spki() -> Vec<u8> {
        let alg = der::sequence(&[der::oid(&Oid::parse("1.2.840.10045.2.1").unwrap()).unwrap(), der::oid(&Oid::parse("1.2.840.10045.3.1.7").unwrap()).unwrap()]);
        der::sequence(&[alg, der::bit_string(0, &[4u8; 65])])
    }

    fn sample(authority: bool, dns: &[&str]) -> Vec<u8> {
        let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
        let spki = spki();
        let t = Template {
            serial: &[0x00, 0x8a, 0x01],
            signature_algorithm: &alg,
            issuer: &[("C", "IS"), ("O", "Example, Ltd."), ("CN", "Example Root")],
            subject: &[("O", "Example, Ltd."), ("CN", "www.example.is")],
            not_before: from_civil(2026, 1, 1, 0, 0, 0).unwrap(),
            not_after: from_civil(2027, 1, 1, 0, 0, 0).unwrap(),
            spki: &spki,
            dns_names: dns,
            authority,
        };
        encode(&t, &[9, 9, 9]).unwrap()
    }

    #[test]
    fn a_certificate_round_trips_through_encode_and_parse() {
        let der = sample(false, &["www.example.is", "*.example.is"]);
        let c = parse(&der).unwrap();
        assert_eq!(c.version, 3);
        assert_eq!(c.serial, vec![0x00, 0x8a, 0x01]);
        assert_eq!(c.signature_algorithm.to_string(), "1.2.840.10045.4.3.2");
        assert_eq!(c.not_before, from_civil(2026, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(c.not_after, from_civil(2027, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(c.signature, vec![9, 9, 9]);
        assert_eq!(c.spki, spki());
        assert_eq!(c.der, der);
        assert_eq!(c.tbs[0], der::SEQUENCE);
        assert!(der.windows(c.tbs.len()).any(|w| w == &c.tbs[..]), "the signed body is a slice of the certificate");
    }

    #[test]
    fn names_read_back_most_specific_first_with_escapes() {
        let c = parse(&sample(false, &[])).unwrap();
        assert_eq!(c.issuer.to_string(), "CN=Example Root,O=Example\\, Ltd.,C=IS");
        assert_eq!(c.subject.to_string(), "CN=www.example.is,O=Example\\, Ltd.");
        assert_eq!(c.subject.common_name(), Some("www.example.is"));
        assert_eq!(c.issuer.first(&[2, 5, 4, 6]), Some("IS"));
        assert!(!c.is_self_issued());
    }

    #[test]
    fn extensions_are_read_by_type() {
        let c = parse(&sample(true, &["a.example.is"])).unwrap();
        assert_eq!(c.basic_constraints().unwrap(), Some((true, None)));
        assert!(c.can_sign_certificates().unwrap());
        assert!(c.extension(&OID_BASIC_CONSTRAINTS).unwrap().critical);
        assert_eq!(c.dns_names().unwrap(), vec!["a.example.is".to_string()]);
        assert_eq!(c.subject_alt_names().unwrap(), vec![GeneralName::Dns("a.example.is".into())]);
        let leaf = parse(&sample(false, &[])).unwrap();
        assert_eq!(leaf.basic_constraints().unwrap(), None);
        assert!(!leaf.can_sign_certificates().unwrap());
        assert!(leaf.dns_names().unwrap().is_empty());
    }

    #[test]
    fn validity_is_inclusive_at_both_ends() {
        let c = parse(&sample(false, &[])).unwrap();
        assert!(c.is_valid_at(c.not_before));
        assert!(c.is_valid_at(c.not_after));
        assert!(!c.is_valid_at(c.not_before - 1));
        assert!(!c.is_valid_at(c.not_after + 1));
    }

    #[test]
    fn fingerprints_follow_the_bytes() {
        let a = parse(&sample(false, &[])).unwrap();
        let b = parse(&sample(true, &[])).unwrap();
        assert_ne!(a.fingerprint_sha256(), b.fingerprint_sha256());
        assert_eq!(a.spki_sha256(), b.spki_sha256());
        assert_eq!(a.fingerprint_sha256(), sha256::digest(&a.der));
    }

    #[test]
    fn a_self_issued_certificate_is_recognized() {
        let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
        let spki = spki();
        let name = [("CN", "Local Root")];
        let t = Template { serial: &[1], signature_algorithm: &alg, issuer: &name, subject: &name, not_before: 100, not_after: 200, spki: &spki, dns_names: &[], authority: true };
        let c = parse(&encode(&t, &[1]).unwrap()).unwrap();
        assert!(c.is_self_issued());
    }

    #[test]
    fn broken_certificates_are_refused() {
        let good = sample(false, &[]);
        // truncated
        assert!(matches!(parse(&good[..good.len() - 1]), Err(X509Error::Der(_))));
        // trailing byte
        let mut extra = good.clone();
        extra.push(0);
        assert_eq!(parse(&extra), Err(X509Error::Der(DerError::TrailingData)));
        // not a sequence
        assert!(matches!(parse(&der::null()), Err(X509Error::Der(DerError::UnexpectedTag { .. }))));
        // validity backwards
        let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
        let spki = spki();
        let name = [("CN", "x")];
        let t = Template { serial: &[1], signature_algorithm: &alg, issuer: &name, subject: &name, not_before: 200, not_after: 100, spki: &spki, dns_names: &[], authority: false };
        assert_eq!(parse(&encode(&t, &[1]).unwrap()), Err(X509Error::BadValidity));
    }

    #[test]
    fn differing_signature_algorithms_are_refused() {
        let good = sample(false, &[]);
        let c = parse(&good).unwrap();
        // flip the last arc of the outer algorithm (the second occurrence of the identifier bytes)
        let needle = [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
        let positions: Vec<usize> = good.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect();
        assert_eq!(positions.len(), 2);
        let mut bad = good.clone();
        bad[positions[1] + needle.len() - 1] = 0x03;
        assert_eq!(parse(&bad), Err(X509Error::SignatureAlgorithmMismatch));
        assert!(c.signature_algorithm.arcs().ends_with(&[4, 3, 2]));
    }

    #[test]
    fn an_unknown_name_attribute_is_refused_when_encoding() {
        let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
        let spki = spki();
        let name = [("nonsense", "x")];
        let t = Template { serial: &[1], signature_algorithm: &alg, issuer: &name, subject: &name, not_before: 1, not_after: 2, spki: &spki, dns_names: &[], authority: false };
        assert_eq!(encode(&t, &[1]), Err(X509Error::UnknownAttribute("nonsense".into())));
    }
}
