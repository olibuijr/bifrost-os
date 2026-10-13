//! The ZIP container of the office document formats: reading one entry by name (stored or deflated), and writing a
//! package of entries. The reader goes by the central directory, as every ZIP tool does, so entries with data
//! descriptors and extra fields read correctly; it never reads past the bytes it was given.

use bifrost_app::archive;

/// The most an entry may inflate to.
const MAX_ENTRY: usize = 64 << 20;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;

/// Whether the bytes start like a ZIP file.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The offset of the end-of-central-directory record: the last signature in the final 64 KiB.
fn find_end(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 22 {
        return None;
    }
    let last = bytes.len() - 22;
    let first = last.saturating_sub(65_535);
    (first..=last).rev().find(|&at| u32le(bytes, at) == Some(END))
}

/// The names of the entries.
#[cfg_attr(not(test), allow(dead_code))]
pub fn names(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    walk(bytes, &mut |name, _, _, _| {
        out.push(String::from_utf8_lossy(name).into_owned());
        false
    });
    out
}

/// Calls `visit(name, method, compressed size, local header offset)` for each central directory entry until it
/// returns true.
fn walk(bytes: &[u8], visit: &mut dyn FnMut(&[u8], u16, usize, usize) -> bool) -> Option<()> {
    let end = find_end(bytes)?;
    let count = u16le(bytes, end + 10)? as usize;
    let mut at = u32le(bytes, end + 16)? as usize;
    for _ in 0..count {
        if u32le(bytes, at)? != CENTRAL {
            return None;
        }
        let method = u16le(bytes, at + 10)?;
        let compressed = u32le(bytes, at + 20)? as usize;
        let name_len = u16le(bytes, at + 28)? as usize;
        let extra_len = u16le(bytes, at + 30)? as usize;
        let comment_len = u16le(bytes, at + 32)? as usize;
        let local = u32le(bytes, at + 42)? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len)?;
        if visit(name, method, compressed, local) {
            return Some(());
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    Some(())
}

/// The contents of the entry called `name`, inflated when it is deflated. `None` when there is no such entry, the
/// archive is damaged, or the entry uses another method.
pub fn read_entry(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut found: Option<(u16, usize, usize)> = None;
    walk(bytes, &mut |entry, method, compressed, local| {
        if entry == name.as_bytes() {
            found = Some((method, compressed, local));
            true
        } else {
            false
        }
    })?;
    let (method, compressed, local) = found?;
    if u32le(bytes, local)? != LOCAL {
        return None;
    }
    let name_len = u16le(bytes, local + 26)? as usize;
    let extra_len = u16le(bytes, local + 28)? as usize;
    let start = local + 30 + name_len + extra_len;
    let data = bytes.get(start..start.checked_add(compressed)?)?;
    match method {
        0 => Some(data.to_vec()),
        8 => archive::decompress(data, MAX_ENTRY).ok(),
        _ => None,
    }
}

/// The CRC-32 (IEEE) of the data, as ZIP stores it.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

/// A package of the entries, in order, each stored without compression. Packages whose first entry must be readable
/// without inflating (the `mimetype` of an OpenDocument file) rely on this.
pub fn write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    build(entries, false)
}

/// Like [`write`], with each entry deflated.
#[cfg_attr(not(test), allow(dead_code))]
pub fn write_deflated(entries: &[(&str, &[u8])]) -> Vec<u8> {
    build(entries, true)
}

fn build(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let crc = crc32(data);
        let (method, body): (u16, Vec<u8>) = if deflate { (8, archive::compress(data)) } else { (0, data.to_vec()) };
        let offset = out.len() as u32;
        // 1980-01-01 00:00, the earliest date a ZIP can carry
        let (time, date) = (0u16, 33u16);
        out.extend(LOCAL.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(method.to_le_bytes());
        out.extend(time.to_le_bytes());
        out.extend(date.to_le_bytes());
        out.extend(crc.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(&body);

        central.extend(CENTRAL.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(method.to_le_bytes());
        central.extend(time.to_le_bytes());
        central.extend(date.to_le_bytes());
        central.extend(crc.to_le_bytes());
        central.extend((body.len() as u32).to_le_bytes());
        central.extend((data.len() as u32).to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(0u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let directory_offset = out.len() as u32;
    let directory_size = central.len() as u32;
    out.extend(&central);
    out.extend(END.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend(directory_size.to_le_bytes());
    out.extend(directory_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn stored_entries_read_back_by_name() {
        let zip = write(&[("a.txt", &b"alpha"[..]), ("dir/b.xml", &b"<b/>"[..])]);
        assert!(is_zip(&zip));
        assert_eq!(read_entry(&zip, "a.txt"), Some(b"alpha".to_vec()));
        assert_eq!(read_entry(&zip, "dir/b.xml"), Some(b"<b/>".to_vec()));
        assert_eq!(read_entry(&zip, "missing"), None);
        assert_eq!(names(&zip), vec!["a.txt".to_string(), "dir/b.xml".to_string()]);
    }

    #[test]
    fn deflated_entries_inflate() {
        let text = "the same words again and again ".repeat(40);
        let zip = write_deflated(&[("t.txt", text.as_bytes())]);
        assert!(zip.len() < text.len(), "the entry is compressed");
        assert_eq!(read_entry(&zip, "t.txt"), Some(text.into_bytes()));
    }

    #[test]
    fn damaged_or_foreign_bytes_read_as_nothing() {
        assert_eq!(read_entry(b"not a zip", "a"), None);
        assert!(!is_zip(b"{\\rtf1}"));
        let mut zip = write(&[("a", &b"data"[..])]);
        zip.truncate(zip.len() - 10);
        assert_eq!(read_entry(&zip, "a"), None);
        assert!(names(&[]).is_empty());
    }
}
