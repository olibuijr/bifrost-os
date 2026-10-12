//! GSS-API object identifiers (`gss_OID`, `gss_OID_set`, `gss_oid_to_str`, `gss_oid_equal`,
//! `gss_create_empty_oid_set`, `gss_add_oid_set_member`, `gss_test_oid_set_member`): the names that select a security
//! mechanism, a quality of protection or a name type.
//!
//! An object identifier is a sequence of arcs written in dotted form, such as `1.2.840.113549`. Its element bytes are
//! the content octets of its ASN.1 encoding (without the tag and length): the first two arcs share one sub-identifier
//! of `40 * first + second`, and every sub-identifier is base 128, most significant group first, with the high bit set
//! on every byte except the last. The page names the operations and the descriptor types; the byte form here is the
//! standard ASN.1 rule, and the tests check it against a published value.
//!
//! Not here: the mechanism registry that maps an identifier to an implementation, and the release calls, which Rust
//! drop replaces.

use std::fmt;

/// An object identifier, held as its arcs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Oid {
    arcs: Vec<u32>,
}

/// Why an identifier cannot be built or decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OidError {
    /// The dotted text is not a sequence of decimal arcs.
    Malformed,
    /// Fewer than two arcs. A single arc names no object.
    TooFewArcs,
    /// The first arc is above 2.
    FirstArc,
    /// The first arc is 0 or 1 and the second is 40 or more, so the combined sub-identifier is ambiguous.
    SecondArc,
    /// The element bytes end inside a sub-identifier.
    Truncated,
    /// A sub-identifier starts with a zero group (`0x80`), which ASN.1 forbids.
    NonMinimal,
    /// A sub-identifier or an arc does not fit its integer width.
    Overflow,
}

impl fmt::Display for OidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OidError::Malformed => "object identifier is not dotted decimal",
            OidError::TooFewArcs => "object identifier needs at least two arcs",
            OidError::FirstArc => "first arc of an object identifier is above 2",
            OidError::SecondArc => "second arc is 40 or more under a first arc of 0 or 1",
            OidError::Truncated => "object identifier bytes end inside a sub-identifier",
            OidError::NonMinimal => "sub-identifier starts with a zero group",
            OidError::Overflow => "object identifier value does not fit its width",
        })
    }
}

impl std::error::Error for OidError {}

impl Oid {
    /// Builds an identifier from its arcs, checking the first two against the ASN.1 rule.
    pub fn from_arcs(arcs: &[u32]) -> Result<Oid, OidError> {
        if arcs.len() < 2 {
            return Err(OidError::TooFewArcs);
        }
        if arcs[0] > 2 {
            return Err(OidError::FirstArc);
        }
        if arcs[0] < 2 && arcs[1] >= 40 {
            return Err(OidError::SecondArc);
        }
        Ok(Oid { arcs: arcs.to_vec() })
    }

    /// Parses dotted decimal text, the form `gss_oid_to_str` writes.
    pub fn parse(text: &str) -> Result<Oid, OidError> {
        let mut arcs = Vec::new();
        for part in text.split('.') {
            arcs.push(part.parse::<u32>().map_err(|_| OidError::Malformed)?);
        }
        Oid::from_arcs(&arcs)
    }

    /// The arcs in order.
    pub fn arcs(&self) -> &[u32] {
        &self.arcs
    }

    /// `gss_oid_to_str`: the dotted decimal form.
    pub fn to_dotted(&self) -> String {
        self.arcs.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(".")
    }

    /// The element bytes of the identifier: the content octets of its ASN.1 encoding.
    pub fn to_der_content(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_sub_identifier(&mut out, self.arcs[0] as u64 * 40 + self.arcs[1] as u64);
        for &arc in &self.arcs[2..] {
            push_sub_identifier(&mut out, arc as u64);
        }
        out
    }

    /// Decodes the element bytes produced by [`Oid::to_der_content`].
    pub fn from_der_content(bytes: &[u8]) -> Result<Oid, OidError> {
        let mut subs = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x80 {
                return Err(OidError::NonMinimal);
            }
            let mut value: u64 = 0;
            loop {
                if i >= bytes.len() {
                    return Err(OidError::Truncated);
                }
                let b = bytes[i];
                i += 1;
                value = value
                    .checked_mul(128)
                    .and_then(|v| v.checked_add((b & 0x7f) as u64))
                    .ok_or(OidError::Overflow)?;
                if b & 0x80 == 0 {
                    break;
                }
            }
            subs.push(value);
        }
        if subs.is_empty() {
            return Err(OidError::TooFewArcs);
        }
        let first_sub = subs[0];
        let (first, second) = if first_sub < 40 {
            (0, first_sub)
        } else if first_sub < 80 {
            (1, first_sub - 40)
        } else {
            (2, first_sub - 80)
        };
        let mut arcs = vec![first];
        arcs.push(u32::try_from(second).map_err(|_| OidError::Overflow)?);
        for &s in &subs[1..] {
            arcs.push(u32::try_from(s).map_err(|_| OidError::Overflow)?);
        }
        Oid::from_arcs(&arcs)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_dotted())
    }
}

/// Appends one sub-identifier in base 128: the high bit marks every byte but the last.
fn push_sub_identifier(out: &mut Vec<u8>, value: u64) {
    let mut groups = [0u8; 10];
    let mut n = 0;
    let mut v = value;
    loop {
        groups[n] = (v & 0x7f) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for k in (0..n).rev() {
        out.push(if k == 0 { groups[k] } else { groups[k] | 0x80 });
    }
}

/// `gss_OID_set`: a list of identifiers, such as the mechanisms an application offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OidSet {
    members: Vec<Oid>,
}

impl OidSet {
    /// `gss_create_empty_oid_set`.
    pub fn new() -> OidSet {
        OidSet::default()
    }

    /// `gss_add_oid_set_member`: appends the identifier.
    pub fn add(&mut self, oid: Oid) {
        self.members.push(oid);
    }

    /// `gss_test_oid_set_member`: true when an equal identifier is in the set.
    pub fn contains(&self, oid: &Oid) -> bool {
        self.members.iter().any(|m| m == oid)
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// True when the set has no members.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members in the order they were added.
    pub fn iter(&self) -> std::slice::Iter<'_, Oid> {
        self.members.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_text_parses_and_writes_back() {
        let oid = Oid::parse("1.2.840.113554.1.2.2").unwrap();
        assert_eq!(oid.arcs(), &[1, 2, 840, 113554, 1, 2, 2]);
        assert_eq!(oid.to_dotted(), "1.2.840.113554.1.2.2");
        assert_eq!(oid.to_string(), "1.2.840.113554.1.2.2");
    }

    #[test]
    fn element_bytes_match_the_worked_encoding() {
        // 1.2.840.113549: the first sub-identifier is 1*40+2 = 42 = 0x2A. 840 = 6*128 + 72 gives 0x86 0x48.
        // 113549 = 6*128^2 + 119*128 + 13 gives 0x86 0xF7 0x0D.
        let oid = Oid::parse("1.2.840.113549").unwrap();
        assert_eq!(oid.to_der_content(), vec![0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D]);
    }

    #[test]
    fn element_bytes_decode_back_to_the_same_arcs() {
        let bytes = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D];
        let oid = Oid::from_der_content(&bytes).unwrap();
        assert_eq!(oid.to_dotted(), "1.2.840.113549");
        assert_eq!(oid.to_der_content(), bytes.to_vec());
    }

    #[test]
    fn a_second_arc_above_39_is_legal_under_the_first_arc_two() {
        // 2.100: the first sub-identifier is 2*40+100 = 180 = 1*128 + 52, so the bytes are 0x81 0x34.
        let oid = Oid::parse("2.100").unwrap();
        assert_eq!(oid.to_der_content(), vec![0x81, 0x34]);
        assert_eq!(Oid::from_der_content(&[0x81, 0x34]).unwrap().to_dotted(), "2.100");
    }

    #[test]
    fn invalid_arcs_are_refused_with_the_reason() {
        assert_eq!(Oid::parse("3.1"), Err(OidError::FirstArc));
        assert_eq!(Oid::parse("1.40"), Err(OidError::SecondArc));
        assert_eq!(Oid::parse("1"), Err(OidError::TooFewArcs));
        assert_eq!(Oid::parse("1.x.2"), Err(OidError::Malformed));
    }

    #[test]
    fn truncated_and_non_minimal_bytes_are_refused() {
        // 0x86 has the continuation bit but nothing follows it.
        assert_eq!(Oid::from_der_content(&[0x2A, 0x86]), Err(OidError::Truncated));
        // A sub-identifier that starts with 0x80 is not minimal.
        assert_eq!(Oid::from_der_content(&[0x2A, 0x80, 0x01]), Err(OidError::NonMinimal));
    }

    #[test]
    fn a_set_tests_membership_by_equality() {
        let mut set = OidSet::new();
        assert!(set.is_empty());
        set.add(Oid::parse("1.2.3").unwrap());
        set.add(Oid::parse("1.2.840").unwrap());
        assert_eq!(set.len(), 2);
        assert!(set.contains(&Oid::parse("1.2.840").unwrap()));
        assert!(!set.contains(&Oid::parse("1.2.4").unwrap()));
    }
}
