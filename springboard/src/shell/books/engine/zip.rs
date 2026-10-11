//! A read-only ZIP reader (central directory, stored and deflated members),
//! enough for EPUB containers.

use super::inflate;

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    method: u16,
    csize: usize,
    usize_: usize,
    offset: usize,
}

pub struct Zip {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

fn u16_at(d: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?))
}

impl Zip {
    /// Parses the central directory; None when `data` is not a ZIP.
    pub fn new(data: Vec<u8>) -> Option<Zip> {
        let eocd = (0..=data.len().checked_sub(22)?).rev().take(65_557).find(|&p| data[p..p + 4] == [0x50, 0x4b, 0x05, 0x06])?;
        let count = u16_at(&data, eocd + 10)? as usize;
        let mut p = u32_at(&data, eocd + 16)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(&data, p)? != 0x0201_4b50 {
                return None;
            }
            let method = u16_at(&data, p + 10)?;
            let csize = u32_at(&data, p + 20)? as usize;
            let usize_ = u32_at(&data, p + 24)? as usize;
            let (nlen, xlen, clen) = (u16_at(&data, p + 28)? as usize, u16_at(&data, p + 30)? as usize, u16_at(&data, p + 32)? as usize);
            let offset = u32_at(&data, p + 42)? as usize;
            let name = String::from_utf8_lossy(data.get(p + 46..p + 46 + nlen)?).into_owned();
            entries.push(Entry { name, method, csize, usize_, offset });
            p += 46 + nlen + xlen + clen;
        }
        Some(Zip { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name).or_else(|| self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)))
    }

    /// The decompressed bytes of member `name`.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.find(name)?;
        let p = e.offset;
        if u32_at(&self.data, p)? != 0x0403_4b50 {
            return None;
        }
        let (nlen, xlen) = (u16_at(&self.data, p + 26)? as usize, u16_at(&self.data, p + 28)? as usize);
        let start = p + 30 + nlen + xlen;
        let raw = self.data.get(start..start + e.csize)?;
        match e.method {
            0 => Some(raw.to_vec()),
            8 => {
                let out = inflate::inflate(raw).ok()?;
                (out.len() == e.usize_ || e.usize_ == 0).then_some(out)
            }
            _ => None,
        }
    }
}

#[cfg(any(test, feature = "testkit"))]
pub mod tests {
    #[cfg(test)]
    use super::*;

    /// Builds a ZIP with the given `(name, method, compressed bytes, uncompressed length)` members.
    pub fn build(members: &[(&str, u16, &[u8], usize)]) -> Vec<u8> {
        let mut out = vec![];
        let mut central = vec![];
        for (name, method, comp, ulen) in members {
            let offset = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]); // time, date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc (unchecked)
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&(*ulen as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(comp);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            central.extend_from_slice(&(*ulen as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(members.len() as u16).to_le_bytes());
        out.extend_from_slice(&(members.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_and_deflated_members() {
        let z = build(&[("a.txt", 0, b"plain", 5), ("dir/b.txt", 8, &[0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00], 5)]);
        let zip = Zip::new(z).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a.txt", "dir/b.txt"]);
        assert_eq!(zip.read("a.txt").unwrap(), b"plain");
        assert_eq!(zip.read("dir/b.txt").unwrap(), b"hello");
        assert_eq!(zip.read("DIR/B.TXT").unwrap(), b"hello", "names match case-insensitively as a fallback");
        assert!(zip.read("missing").is_none());
        assert!(zip.contains("a.txt"));
    }

    #[test]
    fn rejects_non_zip_and_bad_sizes() {
        assert!(Zip::new(b"not a zip at all, definitely not".to_vec()).is_none());
        assert!(Zip::new(vec![]).is_none());
        let z = build(&[("x", 8, &[0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00], 99)]);
        assert!(Zip::new(z).unwrap().read("x").is_none(), "declared size mismatch");
    }
}
