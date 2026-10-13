//! Binary property lists (`bplist00`), read side. The result is the same [`Plist`] value the XML and ASCII readers
//! give, so every caller of [`crate::plist::parse_bytes`] gets binary files for free.
//!
//! Mapping: a null object reads as an empty string (the XML writer does the same), dates are converted from seconds
//! since 2001-01-01 to the UTC text form `YYYY-MM-DDTHH:MM:SSZ` (whole seconds), and UID values are refused because the
//! model has no place for them outside keyed archives. Object graphs that nest deeper than [`MAX_DEPTH`] are refused,
//! which also stops cyclic references from recursing forever.

use crate::plist::Plist;

const MAGIC: &[u8] = b"bplist00";
const TRAILER: usize = 32;
const MAX_DEPTH: usize = 512;
/// Seconds from the Unix epoch to 2001-01-01T00:00:00Z, the binary date origin.
const DATE_EPOCH_OFFSET: f64 = 978_307_200.0;

/// Reads a whole binary property list. The caller has checked the `bplist` prefix.
pub fn parse(bytes: &[u8]) -> Result<Plist, String> {
    if bytes.len() < MAGIC.len() + TRAILER || &bytes[..MAGIC.len()] != MAGIC {
        return Err("not a binary property list".into());
    }
    let t = &bytes[bytes.len() - TRAILER..];
    let offset_size = t[6] as usize;
    let ref_size = t[7] as usize;
    let count = be(&t[8..16])?;
    let top = be(&t[16..24])?;
    let table = be(&t[24..32])?;
    if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
        return Err("binary property list has a bad offset or reference size".into());
    }
    let count = usize::try_from(count).map_err(|_| "binary property list has too many objects".to_string())?;
    let table = usize::try_from(table).map_err(|_| "offset table out of range".to_string())?;
    let table_len = count.checked_mul(offset_size).ok_or_else(|| "offset table out of range".to_string())?;
    if count == 0 || table.checked_add(table_len).map_or(true, |end| end > bytes.len() - TRAILER) {
        return Err("offset table out of range".into());
    }
    let top = usize::try_from(top).map_err(|_| "top object out of range".to_string())?;
    let r = Reader { bytes, table, offset_size, ref_size, count };
    r.object(top as u64, 0)
}

/// Big-endian unsigned integer of 1 to 8 bytes.
fn be(b: &[u8]) -> Result<u64, String> {
    if b.is_empty() || b.len() > 8 {
        return Err("bad integer width in binary property list".into());
    }
    Ok(b.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64))
}

struct Reader<'a> {
    bytes: &'a [u8],
    table: usize,
    offset_size: usize,
    ref_size: usize,
    count: usize,
}

impl<'a> Reader<'a> {
    /// The object body bytes, refusing anything outside the object area (before the trailer).
    fn slice(&self, at: usize, n: usize) -> Result<&'a [u8], String> {
        let end = at.checked_add(n).ok_or_else(|| "binary property list is truncated".to_string())?;
        if end > self.bytes.len() - TRAILER {
            return Err("binary property list is truncated".into());
        }
        Ok(&self.bytes[at..end])
    }

    fn offset_of(&self, index: u64) -> Result<usize, String> {
        let index = usize::try_from(index).ok().filter(|i| *i < self.count).ok_or_else(|| "object reference out of range".to_string())?;
        let at = self.table + index * self.offset_size;
        let off = be(self.slice(at, self.offset_size)?)?;
        usize::try_from(off)
            .ok()
            .filter(|o| *o < self.bytes.len() - TRAILER)
            .ok_or_else(|| "object offset out of range".to_string())
    }

    /// Reads the object at `index`. Containers recurse with a depth guard.
    fn object(&self, index: u64, depth: usize) -> Result<Plist, String> {
        if depth > MAX_DEPTH {
            return Err("binary property list nests too deeply".into());
        }
        let at = self.offset_of(index)?;
        let marker = self.slice(at, 1)?[0];
        let (kind, info) = (marker >> 4, marker & 0x0F);
        let body = at + 1;
        match kind {
            0x0 => match info {
                0x0 => Ok(Plist::Str(String::new())),
                0x8 => Ok(Plist::Bool(false)),
                0x9 => Ok(Plist::Bool(true)),
                _ => Err(format!("unknown singleton {marker:#04x} in binary property list")),
            },
            0x1 => self.int(body, info),
            0x2 => {
                let b = self.slice(body, 1usize << info)?;
                match b.len() {
                    4 => Ok(Plist::Real(f32::from_bits(be(b)? as u32) as f64)),
                    8 => Ok(Plist::Real(f64::from_bits(be(b)?))),
                    _ => Err("bad real width in binary property list".into()),
                }
            }
            0x3 if info == 0x3 => {
                let secs = f64::from_bits(be(self.slice(body, 8)?)?);
                Ok(Plist::Date(format_unix(secs + DATE_EPOCH_OFFSET)))
            }
            0x4 => {
                let (n, start) = self.length(info, body)?;
                Ok(Plist::Data(self.slice(start, n)?.to_vec()))
            }
            0x5 => {
                let (n, start) = self.length(info, body)?;
                let s = String::from_utf8(self.slice(start, n)?.to_vec()).map_err(|_| "bad ASCII string".to_string())?;
                Ok(Plist::Str(s))
            }
            0x6 => {
                let (n, start) = self.length(info, body)?;
                let bytes = self.slice(start, n.checked_mul(2).ok_or_else(|| "string too long".to_string())?)?;
                let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                let s = String::from_utf16(&units).map_err(|_| "bad UTF-16 string".to_string())?;
                Ok(Plist::Str(s))
            }
            0xA => {
                let (n, start) = self.length(info, body)?;
                let mut items = Vec::new();
                for i in 0..n {
                    let r = self.reference(start + i * self.ref_size)?;
                    items.push(self.object(r, depth + 1)?);
                }
                Ok(Plist::Array(items))
            }
            0xD => {
                let (n, start) = self.length(info, body)?;
                let values_at = start + n * self.ref_size;
                let mut pairs = Vec::new();
                for i in 0..n {
                    let k = self.object(self.reference(start + i * self.ref_size)?, depth + 1)?;
                    let Plist::Str(key) = k else {
                        return Err("binary property list has a dictionary key that is not a string".into());
                    };
                    let v = self.object(self.reference(values_at + i * self.ref_size)?, depth + 1)?;
                    pairs.push((key, v));
                }
                Ok(Plist::Dict(pairs))
            }
            0x8 => Err("UID values are not read outside keyed archives".into()),
            _ => Err(format!("unknown object type {marker:#04x} in binary property list")),
        }
    }

    fn reference(&self, at: usize) -> Result<u64, String> {
        be(self.slice(at, self.ref_size)?)
    }

    fn int(&self, body: usize, info: u8) -> Result<Plist, String> {
        let size = 1usize << info;
        let b = self.slice(body, size)?;
        match size {
            1 | 2 | 4 | 8 => Ok(Plist::Int(be(b)? as i64)),
            16 => {
                // Only values that fit in 64 bits are kept: the high half must be a sign extension of the low half.
                let lo = be(&b[8..])? as i64;
                let fits = (b[..8].iter().all(|&x| x == 0) && lo >= 0) || (b[..8].iter().all(|&x| x == 0xFF) && lo < 0);
                if fits {
                    Ok(Plist::Int(lo))
                } else {
                    Err("integer too large for a property list value".into())
                }
            }
            _ => Err("bad integer width in binary property list".into()),
        }
    }

    /// Element count and body start. Info 0xF means the count follows as an integer object.
    fn length(&self, info: u8, body: usize) -> Result<(usize, usize), String> {
        if info != 0x0F {
            return Ok((info as usize, body));
        }
        let m = self.slice(body, 1)?[0];
        if m >> 4 != 0x1 || (m & 0x0F) > 3 {
            return Err("bad length in binary property list".into());
        }
        let size = 1usize << (m & 0x0F);
        let n = usize::try_from(be(self.slice(body + 1, size)?)?).map_err(|_| "length too large".to_string())?;
        Ok((n, body + 1 + size))
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for seconds since the Unix epoch (UTC, whole seconds).
pub fn format_unix(secs: f64) -> String {
    let s = secs.floor() as i64;
    let days = s.div_euclid(86_400);
    let rem = s.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Gregorian date for a day count from 1970-01-01 (proleptic calendar).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `{ k = v; n = 5; }` as a binary property list, one-byte references and offsets.
    fn dict_fixture() -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend([0xD2, 1, 3, 2, 4]); // object 0 at 8: dict of two pairs
        b.extend([0x51, b'k']); // object 1 at 13: ASCII "k"
        b.extend([0x51, b'v']); // object 2 at 15: ASCII "v"
        b.extend([0x51, b'n']); // object 3 at 17: ASCII "n"
        b.extend([0x10, 0x05]); // object 4 at 19: integer 5
        b.extend([8, 13, 15, 17, 19]); // offset table at 21
        b.extend([0, 0, 0, 0, 0, 0, 1, 1]); // unused, sort version, offset size, reference size
        b.extend(5u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(21u64.to_be_bytes());
        b
    }

    #[test]
    fn reads_dictionary_with_keys_in_stored_order() {
        assert_eq!(
            parse(&dict_fixture()),
            Ok(Plist::Dict(vec![("k".into(), Plist::Str("v".into())), ("n".into(), Plist::Int(5))]))
        );
    }

    #[test]
    fn rejects_truncated_and_wrong_magic() {
        let mut bad = dict_fixture();
        bad[0] = b'x';
        assert!(parse(&bad).is_err());
        let short = dict_fixture()[..30].to_vec();
        assert!(parse(&short).is_err());
    }

    #[test]
    fn dates_count_from_2001() {
        assert_eq!(format_unix(0.0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix(DATE_EPOCH_OFFSET), "2001-01-01T00:00:00Z");
        assert_eq!(format_unix(1_760_342_400.0), "2025-10-13T08:00:00Z");
        assert_eq!(format_unix(-1.0), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn scalars_and_utf16_strings() {
        // array of: true, real 1.5, UTF-16 "\u{c5}" (one unit), data [1, 2]
        let mut b = MAGIC.to_vec();
        b.extend([0xA4, 1, 2, 3, 4]); // object 0 at 8, four refs
        b.extend([0x09]); // object 1 at 13: true
        b.extend([0x23, 0x3F, 0xF8, 0, 0, 0, 0, 0, 0]); // object 2 at 14: real 1.5
        b.extend([0x61, 0x00, 0xC5]); // object 3 at 23: UTF-16 string of one unit
        b.extend([0x42, 1, 2]); // object 4 at 26: data
        b.extend([8, 13, 14, 23, 26]); // offset table at 29
        b.extend([0, 0, 0, 0, 0, 0, 1, 1]);
        b.extend(5u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(29u64.to_be_bytes());
        assert_eq!(
            parse(&b),
            Ok(Plist::Array(vec![
                Plist::Bool(true),
                Plist::Real(1.5),
                Plist::Str("\u{c5}".into()),
                Plist::Data(vec![1, 2])
            ]))
        );
    }

    #[test]
    fn negative_eight_byte_integer_is_signed() {
        let mut b = MAGIC.to_vec();
        b.extend([0x13]); // object 0 at 8: 8-byte integer
        b.extend((-7i64).to_be_bytes());
        b.extend([8]); // offset table at 17
        b.extend([0, 0, 0, 0, 0, 0, 1, 1]);
        b.extend(1u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(17u64.to_be_bytes());
        assert_eq!(parse(&b), Ok(Plist::Int(-7)));
    }

    #[test]
    fn self_referencing_array_is_refused_not_looped() {
        let mut b = MAGIC.to_vec();
        b.extend([0xA1, 0]); // object 0 at 8: array of one, referring to itself
        b.extend([8]); // offset table at 10
        b.extend([0, 0, 0, 0, 0, 0, 1, 1]);
        b.extend(1u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(10u64.to_be_bytes());
        assert!(parse(&b).is_err());
    }
}
