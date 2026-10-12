//! Object identifiers in dotted, arc-list and DER form, and sets of them (std only).
//!
//! An object identifier names a schema, an algorithm or a security mechanism as a
//! sequence of arcs such as `1.2.840.113549`. In DER it is the content of a tagged
//! value: the first two arcs share one sub-identifier (`40 * a + b`), and every other
//! arc is written base 128, most significant group first, with the top bit set on every
//! byte except the last. This module parses and prints the dotted form, converts to and
//! from DER, and keeps a duplicate-free set. It does no ASN.1 parsing beyond one value
//! and no mechanism lookup.

use std::fmt;

/// DER tag of an OBJECT IDENTIFIER.
pub const TAG_OID: u8 = 0x06;

/// Why an identifier or its DER form was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OidError {
    /// Fewer than two arcs, a bad number, or a first arc above 2.
    Syntax,
    /// The second arc is 39 or more when the first arc is 0 or 1.
    SecondArc,
    /// A sub-identifier does not fit in 64 bits.
    Overflow,
    /// The DER bytes end in the middle of a sub-identifier, or are empty.
    Truncated,
    /// The DER tag is not OBJECT IDENTIFIER, or the length does not match the data.
    BadTag,
}

impl fmt::Display for OidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            OidError::Syntax => "object identifier must have two or more numeric arcs",
            OidError::SecondArc => "second arc must be below 40 under arc 0 or 1",
            OidError::Overflow => "object identifier arc is too large",
            OidError::Truncated => "object identifier content is truncated",
            OidError::BadTag => "value is not an object identifier",
        };
        f.write_str(text)
    }
}

impl std::error::Error for OidError {}

/// An object identifier as its arcs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Oid {
    arcs: Vec<u64>,
}

impl Oid {
    /// Parses a dotted identifier such as `1.2.840.113549`.
    pub fn parse(dotted: &str) -> Result<Oid, OidError> {
        let mut arcs = Vec::new();
        for part in dotted.split('.') {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(OidError::Syntax);
            }
            arcs.push(part.parse::<u64>().map_err(|_| OidError::Overflow)?);
        }
        Oid::from_arcs(arcs)
    }

    /// Builds an identifier from arcs, checking the first two.
    pub fn from_arcs(arcs: Vec<u64>) -> Result<Oid, OidError> {
        if arcs.len() < 2 || arcs[0] > 2 {
            return Err(OidError::Syntax);
        }
        if arcs[0] < 2 && arcs[1] >= 40 {
            return Err(OidError::SecondArc);
        }
        Ok(Oid { arcs })
    }

    pub fn arcs(&self) -> &[u64] {
        &self.arcs
    }

    /// The content bytes of the DER encoding, without tag and length.
    pub fn to_der_content(&self) -> Result<Vec<u8>, OidError> {
        let mut out = Vec::new();
        let first = self.arcs[0].checked_mul(40).and_then(|v| v.checked_add(self.arcs[1]));
        let first = first.ok_or(OidError::Overflow)?;
        push_base128(first, &mut out);
        for &arc in &self.arcs[2..] {
            push_base128(arc, &mut out);
        }
        Ok(out)
    }

    /// The full DER encoding: tag 0x06, a one-byte length, then the content.
    pub fn to_der(&self) -> Result<Vec<u8>, OidError> {
        let content = self.to_der_content()?;
        // The short length form covers every identifier that fits a 127-byte content.
        if content.len() > 127 {
            return Err(OidError::Overflow);
        }
        let mut out = vec![TAG_OID, content.len() as u8];
        out.extend_from_slice(&content);
        Ok(out)
    }

    /// Reads content bytes (no tag, no length) back into arcs.
    pub fn from_der_content(content: &[u8]) -> Result<Oid, OidError> {
        if content.is_empty() {
            return Err(OidError::Truncated);
        }
        let mut subs = Vec::new();
        let mut value: u64 = 0;
        let mut open = false;
        for &b in content {
            value = value
                .checked_mul(128)
                .and_then(|v| v.checked_add(u64::from(b & 0x7F)))
                .ok_or(OidError::Overflow)?;
            open = b & 0x80 != 0;
            if !open {
                subs.push(value);
                value = 0;
            }
        }
        if open {
            return Err(OidError::Truncated);
        }
        let first = subs[0];
        let (a, b) = if first < 40 {
            (0, first)
        } else if first < 80 {
            (1, first - 40)
        } else {
            (2, first - 80)
        };
        let mut arcs = vec![a, b];
        arcs.extend_from_slice(&subs[1..]);
        Oid::from_arcs(arcs)
    }

    /// Reads one full DER value (tag, one-byte length, content) as an identifier.
    pub fn from_der(der: &[u8]) -> Result<Oid, OidError> {
        if der.len() < 2 {
            return Err(OidError::Truncated);
        }
        if der[0] != TAG_OID || usize::from(der[1]) != der.len() - 2 {
            return Err(OidError::BadTag);
        }
        Oid::from_der_content(&der[2..])
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, arc) in self.arcs.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{arc}")?;
        }
        Ok(())
    }
}

/// Writes one value in base 128 with continuation bits.
fn push_base128(mut value: u64, out: &mut Vec<u8>) {
    let mut groups = [0u8; 10];
    let mut n = 0;
    loop {
        groups[n] = (value & 0x7F) as u8;
        n += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        let cont = if i > 0 { 0x80 } else { 0 };
        out.push(groups[i] | cont);
    }
}

/// A set of identifiers with no duplicates, kept in insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OidSet {
    items: Vec<Oid>,
}

impl OidSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an identifier. Returns false when it was already present.
    pub fn add(&mut self, oid: Oid) -> bool {
        if self.contains(&oid) {
            return false;
        }
        self.items.push(oid);
        true
    }

    pub fn contains(&self, oid: &Oid) -> bool {
        self.items.iter().any(|o| o == oid)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Oid> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsadsi_identifier_encodes_to_the_known_der_bytes() {
        // 1.2 is 42 (0x2A); 840 is 0x86 0x48; 113549 is 0x86 0xF7 0x0D.
        let oid = Oid::parse("1.2.840.113549").unwrap();
        assert_eq!(
            oid.to_der_content().unwrap(),
            vec![0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D]
        );
        assert_eq!(oid.to_der().unwrap()[..2], [0x06, 0x06]);
    }

    #[test]
    fn common_name_identifier_matches_its_der_form() {
        let oid = Oid::parse("2.5.4.3").unwrap();
        assert_eq!(oid.to_der().unwrap(), vec![0x06, 0x03, 0x55, 0x04, 0x03]);
        assert_eq!(Oid::from_der(&[0x06, 0x03, 0x55, 0x04, 0x03]).unwrap(), oid);
        assert_eq!(oid.to_string(), "2.5.4.3");
    }

    #[test]
    fn first_arc_two_allows_a_second_arc_above_39() {
        // 2.999 is 80 + 999 = 1079, which is 0x88 0x37 in base 128.
        let oid = Oid::parse("2.999").unwrap();
        assert_eq!(oid.to_der_content().unwrap(), vec![0x88, 0x37]);
        assert_eq!(Oid::from_der_content(&[0x88, 0x37]).unwrap().arcs(), &[2, 999]);
        assert_eq!(Oid::parse("1.40"), Err(OidError::SecondArc));
        assert_eq!(Oid::parse("3.1"), Err(OidError::Syntax));
        assert_eq!(Oid::parse("1"), Err(OidError::Syntax));
        assert_eq!(Oid::parse("1..2"), Err(OidError::Syntax));
    }

    #[test]
    fn malformed_der_is_refused() {
        assert_eq!(Oid::from_der(&[0x04, 0x01, 0x2A]), Err(OidError::BadTag));
        assert_eq!(Oid::from_der(&[0x06, 0x02, 0x2A]), Err(OidError::BadTag));
        // A sub-identifier whose last byte still has the continuation bit set.
        assert_eq!(Oid::from_der_content(&[0x2A, 0x86]), Err(OidError::Truncated));
        assert_eq!(Oid::from_der_content(&[]), Err(OidError::Truncated));
    }

    #[test]
    fn set_keeps_one_copy_of_each_identifier() {
        let mut set = OidSet::new();
        let kerberos = Oid::parse("1.2.840.113554.1.2.2").unwrap();
        assert!(set.add(kerberos.clone()));
        assert!(!set.add(Oid::parse("1.2.840.113554.1.2.2").unwrap()));
        assert!(set.add(Oid::parse("1.3.6.1.5.5.2").unwrap()));
        assert_eq!(set.len(), 2);
        assert!(set.contains(&kerberos));
        assert!(!set.contains(&Oid::parse("1.2.3").unwrap()));
        assert!(!set.is_empty());
    }

    #[test]
    fn arcs_above_64_bits_are_refused() {
        assert_eq!(Oid::parse("1.18446744073709551616"), Err(OidError::Overflow));
        let big = Oid::from_arcs(vec![2, u64::MAX]).unwrap();
        assert_eq!(big.to_der_content(), Err(OidError::Overflow));
    }
}
