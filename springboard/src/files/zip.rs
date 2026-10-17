//! ZIP create and extract for the Files app: a seekable central-directory
//! reader (stored and deflated members, CRC checked), a writer (stored, or
//! LZ77 + fixed-Huffman deflate), and path handling that refuses zip-slip.
//! Members can be encrypted with the traditional ZIP cipher (ZipCrypto):
//! `Writer::add_bytes_encrypted` seals them and `Archive::read_with_password`
//! opens them. No zip64. Progress callbacks return `false` to cancel.

use super::fs;
use bifrost_render::cg::pdf::inflate;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

/// Largest member read into memory when extracting.
pub const MEMBER_LIMIT: u64 = 256 << 20;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Cancelled")
}

// ---------------------------------------------------------------- CRC-32

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = crc_table();

/// Incremental CRC-32 (IEEE), for files read in chunks.
#[derive(Clone, Copy)]
pub struct Crc(u32);

impl Crc {
    pub fn new() -> Crc {
        Crc(!0)
    }

    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 = CRC_TABLE[((self.0 ^ b as u32) & 0xFF) as usize] ^ (self.0 >> 8);
        }
    }

    pub fn value(&self) -> u32 {
        !self.0
    }
}

impl Default for Crc {
    fn default() -> Self {
        Crc::new()
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc::new();
    c.update(data);
    c.value()
}

// ------------------------------------------------------------ path safety

/// The relative path an archive member name may be extracted to, or None
/// when it would escape the destination (absolute, `..`, drive letter, NUL)
/// or is empty. Backslashes count as separators and `.` parts are dropped.
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    if name.contains('\0') {
        return None;
    }
    let norm = name.replace('\\', "/");
    if norm.starts_with('/') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in norm.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.len() >= 2 && p.as_bytes()[1] == b':' && p.as_bytes()[0].is_ascii_alphabetic() => return None,
            p => out.push(p),
        }
    }
    let ok = out.components().next().is_some() && out.components().all(|c| matches!(c, Component::Normal(_)));
    ok.then_some(out)
}

// ------------------------------------------------------- traditional cipher

/// The three key words of the traditional ZIP cipher (PKWARE "ZipCrypto").
struct ZipKeys {
    k0: u32,
    k1: u32,
    k2: u32,
}

/// One CRC-32 step over a byte, the update the cipher uses for its first and third key words.
fn crc_step(c: u32, b: u8) -> u32 {
    CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
}

impl ZipKeys {
    fn new(password: &[u8]) -> Self {
        let mut k = ZipKeys { k0: 0x1234_5678, k1: 0x2345_6789, k2: 0x3456_7890 };
        for &b in password {
            k.update(b);
        }
        k
    }

    fn update(&mut self, b: u8) {
        self.k0 = crc_step(self.k0, b);
        self.k1 = self.k1.wrapping_add(self.k0 & 0xFF).wrapping_mul(134_775_813).wrapping_add(1);
        self.k2 = crc_step(self.k2, (self.k1 >> 24) as u8);
    }

    /// The keystream byte for the next plain byte.
    fn stream(&self) -> u8 {
        let t = self.k2 | 2;
        (t.wrapping_mul(t ^ 1) >> 8) as u8
    }
}

/// Seals `header` (12 bytes) and `body` with the cipher. The header goes first, as the format requires.
fn encrypt_member(header: &[u8; 12], body: &[u8], password: &[u8]) -> Vec<u8> {
    let mut keys = ZipKeys::new(password);
    let mut out = Vec::with_capacity(12 + body.len());
    for &p in header.iter().chain(body) {
        out.push(p ^ keys.stream());
        keys.update(p);
    }
    out
}

/// Opens a sealed member: the 12-byte header is dropped after its check byte (the high byte of the
/// member CRC) is compared, so a wrong password is refused here.
fn decrypt_member(raw: &[u8], crc: u32, password: &[u8]) -> io::Result<Vec<u8>> {
    if raw.len() < 12 {
        return Err(bad("Damaged ZIP member"));
    }
    let mut keys = ZipKeys::new(password);
    let plain: Vec<u8> = raw
        .iter()
        .map(|&c| {
            let p = c ^ keys.stream();
            keys.update(p);
            p
        })
        .collect();
    if plain[11] != (crc >> 24) as u8 {
        return Err(bad("Wrong password"));
    }
    Ok(plain[12..].to_vec())
}

// ------------------------------------------------------------------ reader

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    pub is_dir: bool,
    pub method: u16,
    pub crc: u32,
    pub csize: u64,
    pub size: u64,
    /// Sealed with the traditional ZIP cipher: needs a password (`read_with_password`).
    pub encrypted: bool,
    offset: u64,
}

pub struct Archive<R: Read + Seek> {
    r: R,
    pub members: Vec<Member>,
}

fn u16_at(d: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?))
}

impl<R: Read + Seek> Archive<R> {
    /// Reads the central directory.
    pub fn open(mut r: R) -> io::Result<Archive<R>> {
        let len = r.seek(SeekFrom::End(0))?;
        let tail_len = len.min(22 + 65_535);
        if tail_len < 22 {
            return Err(bad("Not a ZIP archive"));
        }
        r.seek(SeekFrom::Start(len - tail_len))?;
        let mut tail = vec![0u8; tail_len as usize];
        r.read_exact(&mut tail)?;
        let eocd = (0..=tail.len() - 22).rev().find(|&p| tail[p..p + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(|| bad("Not a ZIP archive"))?;
        let count = u16_at(&tail, eocd + 10).unwrap() as usize;
        let cd_size = u32_at(&tail, eocd + 12).unwrap() as u64;
        let cd_off = u32_at(&tail, eocd + 16).unwrap() as u64;
        if cd_off == 0xFFFF_FFFF || count == 0xFFFF {
            return Err(bad("ZIP64 archives are not supported"));
        }
        if cd_off.checked_add(cd_size).map_or(true, |e| e > len) || cd_size > 64 << 20 {
            return Err(bad("Damaged ZIP directory"));
        }
        r.seek(SeekFrom::Start(cd_off))?;
        let mut cd = vec![0u8; cd_size as usize];
        r.read_exact(&mut cd)?;
        let mut members = Vec::with_capacity(count);
        let mut p = 0;
        for _ in 0..count {
            if u32_at(&cd, p).ok_or_else(|| bad("Damaged ZIP directory"))? != 0x0201_4b50 {
                return Err(bad("Damaged ZIP directory"));
            }
            let h = || bad("Damaged ZIP directory");
            let flags = u16_at(&cd, p + 8).ok_or_else(h)?;
            let method = u16_at(&cd, p + 10).ok_or_else(h)?;
            let crc = u32_at(&cd, p + 16).ok_or_else(h)?;
            let csize = u32_at(&cd, p + 20).ok_or_else(h)? as u64;
            let size = u32_at(&cd, p + 24).ok_or_else(h)? as u64;
            let nlen = u16_at(&cd, p + 28).ok_or_else(h)? as usize;
            let xlen = u16_at(&cd, p + 30).ok_or_else(h)? as usize;
            let clen = u16_at(&cd, p + 32).ok_or_else(h)? as usize;
            let offset = u32_at(&cd, p + 42).ok_or_else(h)? as u64;
            let raw = cd.get(p + 46..p + 46 + nlen).ok_or_else(h)?;
            let name = String::from_utf8_lossy(raw).into_owned();
            members.push(Member { is_dir: name.ends_with('/'), name, method, crc, csize, size, encrypted: flags & 1 != 0, offset });
            p += 46 + nlen + xlen + clen;
        }
        Ok(Archive { r, members })
    }

    /// Total uncompressed bytes of all members.
    pub fn total_size(&self) -> u64 {
        self.members.iter().map(|m| m.size).sum()
    }

    /// The decompressed, CRC-checked bytes of one member. An encrypted member is refused: use `read_with_password`.
    pub fn read(&mut self, m: &Member) -> io::Result<Vec<u8>> {
        self.read_member(m, None)
    }

    /// Like `read`, for a member sealed by `Writer::add_bytes_encrypted`. A wrong password is refused by the
    /// header check byte or by the CRC.
    pub fn read_with_password(&mut self, m: &Member, password: &[u8]) -> io::Result<Vec<u8>> {
        self.read_member(m, Some(password))
    }

    fn read_member(&mut self, m: &Member, password: Option<&[u8]>) -> io::Result<Vec<u8>> {
        // output is capped at the declared size, so a bomb cannot outgrow it
        if m.size > MEMBER_LIMIT || m.csize > MEMBER_LIMIT {
            return Err(bad("File too large to extract"));
        }
        self.r.seek(SeekFrom::Start(m.offset))?;
        let mut h = [0u8; 30];
        self.r.read_exact(&mut h)?;
        if u32_at(&h, 0) != Some(0x0403_4b50) {
            return Err(bad("Damaged ZIP member"));
        }
        let skip = u16_at(&h, 26).unwrap() as i64 + u16_at(&h, 28).unwrap() as i64;
        self.r.seek(SeekFrom::Current(skip))?;
        let mut raw = vec![0u8; m.csize as usize];
        self.r.read_exact(&mut raw)?;
        let raw = if m.encrypted {
            let password = password.ok_or_else(|| bad("Encrypted ZIP member needs a password"))?;
            decrypt_member(&raw, m.crc, password)?
        } else {
            raw
        };
        let out = match m.method {
            0 => raw,
            8 => inflate::inflate_limited(&raw, m.size as usize).map_err(|_| bad("Damaged compressed data"))?,
            _ => return Err(bad("Unsupported compression method")),
        };
        if out.len() as u64 != m.size || crc32(&out) != m.crc {
            return Err(bad("Checksum mismatch"));
        }
        Ok(out)
    }
}

/// What an extraction did.
#[derive(Debug, PartialEq)]
pub struct Extracted {
    pub dir: PathBuf,
    pub files: usize,
    /// member names refused for unsafe paths
    pub skipped: Vec<String>,
}

/// Extracts `archive` into a new folder named after it inside `dest` (the
/// name numbered when taken). `progress(done_bytes, total_bytes)` is called
/// after each member; returning false cancels and removes the partial folder.
pub fn extract(archive: &Path, dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<Extracted> {
    let mut ar = Archive::open(std::io::BufReader::new(std::fs::File::open(archive)?))?;
    let stem = archive.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Archive".into());
    let out = fs::unique_name(dest, &stem);
    std::fs::create_dir(&out)?;
    match extract_into(&mut ar, &out, progress) {
        Ok((files, skipped)) => Ok(Extracted { dir: out, files, skipped }),
        Err(e) => {
            let _ = fs::delete(&out);
            Err(e)
        }
    }
}

fn extract_into<R: Read + Seek>(ar: &mut Archive<R>, out: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<(usize, Vec<String>)> {
    let total = ar.total_size();
    let canon = out.canonicalize()?;
    let (mut done, mut files, mut skipped) = (0u64, 0usize, vec![]);
    let members = ar.members.clone();
    for m in &members {
        let Some(rel) = safe_rel(&m.name) else {
            skipped.push(m.name.clone());
            done += m.size;
            if !progress(done, total) {
                return Err(cancelled());
            }
            continue;
        };
        let target = out.join(&rel);
        if m.is_dir {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
                // a symlink planted by an earlier member must not redirect the write
                if !parent.canonicalize()?.starts_with(&canon) || std::fs::symlink_metadata(&target).is_ok_and(|md| md.file_type().is_symlink()) {
                    skipped.push(m.name.clone());
                    done += m.size;
                    continue;
                }
            }
            let data = ar.read(m)?;
            std::fs::File::create(&target)?.write_all(&data)?;
            files += 1;
        }
        done += m.size;
        if !progress(done, total) {
            return Err(cancelled());
        }
    }
    Ok((files, skipped))
}

// ------------------------------------------------------------------ deflate

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    /// `count` bits of `v`, least significant first (extra bits, headers).
    fn put(&mut self, v: u32, count: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code, most significant bit first.
    fn code(&mut self, v: u32, count: u32) {
        let mut r = 0;
        for i in 0..count {
            r |= ((v >> i) & 1) << (count - 1 - i);
        }
        self.put(r, count);
    }

    fn lit(&mut self, sym: u32) {
        match sym {
            0..=143 => self.code(0x30 + sym, 8),
            144..=255 => self.code(0x190 + sym - 144, 9),
            256..=279 => self.code(sym - 256, 7),
            _ => self.code(0xC0 + sym - 280, 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

fn index_of(table: &[u16], v: u16) -> usize {
    table.iter().rposition(|&b| b <= v).unwrap_or(0)
}

/// Raw DEFLATE of `data`: greedy LZ77 over a 32 KiB window, one fixed-Huffman block.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    const HASH: usize = 1 << 15;
    const WINDOW: usize = 32_768;
    const CHAIN: usize = 32;
    let mut bw = Bits { out: Vec::with_capacity(data.len() / 2 + 16), acc: 0, n: 0 };
    bw.put(1, 1); // final block
    bw.put(1, 2); // fixed Huffman
    let mut head = vec![usize::MAX; HASH];
    let mut prev = vec![usize::MAX; data.len()];
    let hash = |p: usize| ((data[p] as usize) << 10 ^ (data[p + 1] as usize) << 5 ^ data[p + 2] as usize) & (HASH - 1);
    let insert = |head: &mut Vec<usize>, prev: &mut Vec<usize>, p: usize| {
        if p + 3 <= data.len() {
            let h = hash(p);
            prev[p] = head[h];
            head[h] = p;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0usize, 0usize);
        if i + 3 <= data.len() {
            let mut cand = head[hash(i)];
            let mut tries = 0;
            while cand != usize::MAX && i - cand <= WINDOW && tries < CHAIN {
                let max = (data.len() - i).min(258);
                let mut l = 0;
                while l < max && data[cand + l] == data[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = i - cand;
                    if l == max {
                        break;
                    }
                }
                cand = prev[cand];
                tries += 1;
            }
        }
        if best_len >= 3 {
            let li = index_of(&LEN_BASE, best_len as u16);
            bw.lit(257 + li as u32);
            bw.put((best_len as u16 - LEN_BASE[li]) as u32, LEN_EXTRA[li] as u32);
            let di = index_of(&DIST_BASE, best_dist as u16);
            bw.code(di as u32, 5);
            bw.put((best_dist as u16 - DIST_BASE[di]) as u32, DIST_EXTRA[di] as u32);
            for k in 0..best_len {
                insert(&mut head, &mut prev, i + k);
            }
            i += best_len;
        } else {
            bw.lit(data[i] as u32);
            insert(&mut head, &mut prev, i);
            i += 1;
        }
    }
    bw.lit(256);
    bw.finish()
}

// ------------------------------------------------------------------ writer

/// Writes a ZIP archive member by member.
pub struct Writer<W: Write> {
    w: W,
    pos: u64,
    central: Vec<u8>,
    count: u16,
}

/// DOS (time, date) words for a Unix time; years before 1980 clamp to it.
pub fn dos_time(unix: i64) -> (u16, u16) {
    let (y, mo, d, h, mi) = fs::civil(unix);
    if y < 1980 {
        return (0, (1 << 5) | 1);
    }
    let year = (y.min(2107) - 1980) as u16;
    ((h as u16) << 11 | (mi as u16) << 5, year << 9 | (mo as u16) << 5 | d as u16)
}

impl<W: Write> Writer<W> {
    pub fn new(w: W) -> Self {
        Writer { w, pos: 0, central: vec![], count: 0 }
    }

    fn put(&mut self, b: &[u8]) -> io::Result<()> {
        self.w.write_all(b)?;
        self.pos += b.len() as u64;
        Ok(())
    }

    fn header(&mut self, name: &str, method: u16, flags: u16, crc: u32, csize: u64, size: u64, mtime: i64, is_dir: bool) -> io::Result<()> {
        if self.count == u16::MAX || csize > u32::MAX as u64 || size > u32::MAX as u64 || self.pos > u32::MAX as u64 {
            return Err(bad("Archive too large (ZIP64 is not supported)"));
        }
        let (t, d) = dos_time(mtime);
        let offset = self.pos as u32;
        let mut l = vec![];
        l.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        l.extend_from_slice(&20u16.to_le_bytes());
        l.extend_from_slice(&flags.to_le_bytes());
        l.extend_from_slice(&method.to_le_bytes());
        l.extend_from_slice(&t.to_le_bytes());
        l.extend_from_slice(&d.to_le_bytes());
        l.extend_from_slice(&crc.to_le_bytes());
        l.extend_from_slice(&(csize as u32).to_le_bytes());
        l.extend_from_slice(&(size as u32).to_le_bytes());
        l.extend_from_slice(&(name.len() as u16).to_le_bytes());
        l.extend_from_slice(&0u16.to_le_bytes());
        l.extend_from_slice(name.as_bytes());
        self.put(&l)?;
        let c = &mut self.central;
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&0x031Eu16.to_le_bytes()); // made by Unix, 3.0
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&flags.to_le_bytes());
        c.extend_from_slice(&method.to_le_bytes());
        c.extend_from_slice(&t.to_le_bytes());
        c.extend_from_slice(&d.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&(csize as u32).to_le_bytes());
        c.extend_from_slice(&(size as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attrs
        let mode: u32 = if is_dir { 0o040_755 } else { 0o100_644 };
        c.extend_from_slice(&(mode << 16).to_le_bytes());
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        self.count += 1;
        Ok(())
    }

    pub fn add_dir(&mut self, name: &str, mtime: i64) -> io::Result<()> {
        let n = if name.ends_with('/') { name.to_string() } else { format!("{name}/") };
        self.header(&n, 0, UTF8_NAMES, 0, 0, 0, mtime, true)
    }

    /// Adds a file held in memory, deflated when that makes it smaller.
    pub fn add_bytes(&mut self, name: &str, data: &[u8], mtime: i64) -> io::Result<()> {
        let crc = crc32(data);
        let packed = deflate(data);
        if packed.len() < data.len() {
            self.header(name, 8, UTF8_NAMES, crc, packed.len() as u64, data.len() as u64, mtime, false)?;
            self.put(&packed)
        } else {
            self.header(name, 0, UTF8_NAMES, crc, data.len() as u64, data.len() as u64, mtime, false)?;
            self.put(data)
        }
    }

    /// Adds a file held in memory, sealed with the traditional ZIP cipher under `password` and deflated when that
    /// makes it smaller. `header` is the 12-byte encryption header: the caller fills the first 11 bytes with random
    /// data; the last byte is set here to the high byte of the file CRC, which `Archive::read_with_password` checks.
    pub fn add_bytes_encrypted(&mut self, name: &str, data: &[u8], mtime: i64, password: &[u8], mut header: [u8; 12]) -> io::Result<()> {
        let crc = crc32(data);
        header[11] = (crc >> 24) as u8;
        let packed = deflate(data);
        let (method, body) = if packed.len() < data.len() { (8, packed) } else { (0, data.to_vec()) };
        let sealed = encrypt_member(&header, &body, password);
        self.header(name, method, UTF8_NAMES | ENCRYPTED, crc, sealed.len() as u64, data.len() as u64, mtime, false)?;
        self.put(&sealed)
    }

    /// Adds a large file stored (uncompressed) straight from `src` in chunks;
    /// `tick(bytes)` reports each chunk and returns false to cancel.
    pub fn add_stored_stream(&mut self, name: &str, mut src: impl Read + Seek, size: u64, mtime: i64, tick: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
        let mut crc = Crc::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut seen = 0u64;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            crc.update(&buf[..n]);
            seen += n as u64;
            if !tick(0) {
                return Err(cancelled());
            }
        }
        if seen != size {
            return Err(bad("File changed while archiving"));
        }
        src.seek(SeekFrom::Start(0))?;
        self.header(name, 0, UTF8_NAMES, crc.value(), size, size, mtime, false)?;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            self.put(&buf[..n])?;
            if !tick(n as u64) {
                return Err(cancelled());
            }
        }
        Ok(())
    }

    /// Writes the central directory and returns the sink.
    pub fn finish(mut self) -> io::Result<W> {
        let cd_off = self.pos;
        let central = std::mem::take(&mut self.central);
        self.put(&central)?;
        let mut e = vec![];
        e.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        e.extend_from_slice(&[0; 4]);
        e.extend_from_slice(&self.count.to_le_bytes());
        e.extend_from_slice(&self.count.to_le_bytes());
        e.extend_from_slice(&(central.len() as u32).to_le_bytes());
        e.extend_from_slice(&(cd_off as u32).to_le_bytes());
        e.extend_from_slice(&0u16.to_le_bytes());
        self.put(&e)?;
        self.w.flush()?;
        Ok(self.w)
    }
}

/// General-purpose flag: names are UTF-8.
const UTF8_NAMES: u16 = 0x0800;
/// General-purpose flag: the member is encrypted.
const ENCRYPTED: u16 = 0x0001;

/// Files above this are stored in chunks instead of deflated in memory.
pub const STREAM_ABOVE: u64 = 32 << 20;

/// The archive name for a selection: the single item's name, else "Archive".
pub fn archive_name(sources: &[PathBuf]) -> String {
    match sources {
        [one] => format!("{}.zip", one.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())),
        _ => "Archive.zip".into(),
    }
}

fn walk(path: &Path, rel: &str, out: &mut Vec<(PathBuf, String, bool)>) -> io::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.file_type().is_symlink() {
        return Ok(()); // links are not followed or stored
    }
    if md.is_dir() {
        out.push((path.to_path_buf(), format!("{rel}/"), true));
        let mut kids: Vec<_> = std::fs::read_dir(path)?.flatten().collect();
        kids.sort_by_key(|e| e.file_name());
        for k in kids {
            walk(&k.path(), &format!("{rel}/{}", k.file_name().to_string_lossy()), out)?;
        }
    } else {
        out.push((path.to_path_buf(), rel.to_string(), false));
    }
    Ok(())
}

/// Zips `sources` (files and folders, stored under their own names) into a
/// new "<name>.zip" inside `dest`. `progress(done_bytes, total_bytes)` is
/// called as data is read; false cancels and removes the partial archive.
pub fn create(sources: &[PathBuf], dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<PathBuf> {
    let mut items = vec![];
    for s in sources {
        let name = s.file_name().ok_or_else(|| bad("Invalid name"))?.to_string_lossy().into_owned();
        walk(s, &name, &mut items)?;
    }
    let total: u64 = items.iter().filter(|i| !i.2).map(|(p, ..)| std::fs::metadata(p).map_or(0, |m| m.len())).sum();
    let out = fs::unique_name(dest, &archive_name(sources));
    let file = std::fs::File::create(&out)?;
    let result = (|| -> io::Result<()> {
        let mut w = Writer::new(std::io::BufWriter::new(file));
        let mut done = 0u64;
        for (path, name, is_dir) in &items {
            let md = std::fs::metadata(path)?;
            let mtime = md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            if *is_dir {
                w.add_dir(name, mtime)?;
            } else if md.len() > STREAM_ABOVE {
                let base = done;
                let mut got = 0u64;
                w.add_stored_stream(name, std::fs::File::open(path)?, md.len(), mtime, &mut |n| {
                    got += n;
                    progress(base + got, total)
                })?;
                done += md.len();
            } else {
                let data = std::fs::read(path)?;
                w.add_bytes(name, &data, mtime)?;
                done += data.len() as u64;
                if !progress(done, total) {
                    return Err(cancelled());
                }
            }
        }
        w.finish()?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(out),
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A hand-built single-member archive: stored, with a real CRC.
    fn hand_built(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = vec![];
        out.extend_from_slice(&[0x50, 0x4b, 3, 4, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        let cd = out.len() as u32;
        out.extend_from_slice(&[0x50, 0x4b, 1, 2, 10, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        let cd_len = out.len() as u32 - cd;
        out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0]);
        out.extend_from_slice(&cd_len.to_le_bytes());
        out.extend_from_slice(&cd.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let mut c = Crc::new();
        c.update(b"1234");
        c.update(b"56789");
        assert_eq!(c.value(), 0xCBF4_3926);
    }

    #[test]
    fn reads_hand_built_archive() {
        let mut ar = Archive::open(Cursor::new(hand_built("a/b.txt", b"hello"))).unwrap();
        assert_eq!(ar.members.len(), 1);
        let m = ar.members[0].clone();
        assert_eq!((m.name.as_str(), m.size, m.is_dir), ("a/b.txt", 5, false));
        assert_eq!(ar.read(&m).unwrap(), b"hello");
    }

    #[test]
    fn detects_corruption() {
        let mut bytes = hand_built("x", b"hello");
        bytes[31] ^= 0xFF; // flip a data byte
        let mut ar = Archive::open(Cursor::new(bytes)).unwrap();
        let m = ar.members[0].clone();
        assert!(ar.read(&m).is_err());
        assert!(Archive::open(Cursor::new(b"not a zip at all, definitely".to_vec())).is_err());
    }

    #[test]
    fn inflation_is_capped_at_the_declared_size() {
        let mut w = Writer::new(vec![]);
        w.add_bytes("bomb", &vec![b'z'; 100_000], 0).unwrap();
        let mut bytes = w.finish().unwrap();
        let cd = bytes.windows(4).position(|x| x == [0x50, 0x4b, 1, 2]).unwrap();
        bytes[cd + 24..cd + 28].copy_from_slice(&10u32.to_le_bytes());
        let mut ar = Archive::open(Cursor::new(bytes)).unwrap();
        let m = ar.members[0].clone();
        assert!(ar.read(&m).is_err());
    }

    #[test]
    fn deflate_round_trips() {
        let mut data = vec![];
        for i in 0..5000u32 {
            data.extend_from_slice(format!("line {} of the file\n", i % 37).as_bytes());
        }
        data.extend((0..3000u32).map(|i| (i * 7 + i / 3) as u8));
        let packed = deflate(&data);
        assert!(packed.len() < data.len());
        assert_eq!(inflate::inflate(&packed).unwrap(), data);
        assert_eq!(inflate::inflate(&deflate(b"")).unwrap(), b"");
        assert_eq!(inflate::inflate(&deflate(b"a")).unwrap(), b"a");
        let run = vec![b'z'; 100_000];
        assert_eq!(inflate::inflate(&deflate(&run)).unwrap(), run);
    }

    #[test]
    fn writer_then_reader() {
        let mut w = Writer::new(vec![]);
        w.add_dir("d", 0).unwrap();
        w.add_bytes("d/one.txt", &b"abc".repeat(1000), 1_700_000_000).unwrap();
        w.add_bytes("d/two.bin", &[1, 2, 3], 0).unwrap();
        let bytes = w.finish().unwrap();
        let mut ar = Archive::open(Cursor::new(bytes)).unwrap();
        let names: Vec<_> = ar.members.iter().map(|m| m.name.clone()).collect();
        assert_eq!(names, ["d/", "d/one.txt", "d/two.bin"]);
        assert!(ar.members[0].is_dir);
        assert_eq!(ar.members[1].method, 8);
        assert_eq!(ar.members[2].method, 0);
        let m = ar.members[1].clone();
        assert_eq!(ar.read(&m).unwrap(), b"abc".repeat(1000));
        assert_eq!(ar.total_size(), 3000 + 3);
    }

    #[test]
    fn streamed_member_round_trips_and_cancels() {
        let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        let mut w = Writer::new(vec![]);
        let mut total = 0;
        w.add_stored_stream("big", Cursor::new(data.clone()), data.len() as u64, 0, &mut |n| {
            total += n;
            true
        })
        .unwrap();
        assert_eq!(total, data.len() as u64);
        let mut ar = Archive::open(Cursor::new(w.finish().unwrap())).unwrap();
        let m = ar.members[0].clone();
        assert_eq!(ar.read(&m).unwrap(), data);
        let mut w = Writer::new(vec![]);
        let r = w.add_stored_stream("big", Cursor::new(data.clone()), data.len() as u64, 0, &mut |_| false);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn encrypted_member_round_trips_with_its_password() {
        let data = b"top secret ".repeat(300);
        let header = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0];
        let mut w = Writer::new(vec![]);
        w.add_bytes_encrypted("vault/note.txt", &data, 0, b"hunter2", header).unwrap();
        w.add_bytes("plain.txt", b"open", 0).unwrap();
        let mut ar = Archive::open(Cursor::new(w.finish().unwrap())).unwrap();
        assert!(ar.members[0].encrypted);
        assert!(!ar.members[1].encrypted);
        let m = ar.members[0].clone();
        assert_eq!(ar.read(&m).unwrap_err().kind(), io::ErrorKind::InvalidData, "no password is refused");
        assert_eq!(ar.read_with_password(&m, b"hunter2").unwrap(), data);
        let plain = ar.members[1].clone();
        assert_eq!(ar.read_with_password(&plain, b"ignored").unwrap(), b"open", "unencrypted members ignore the password");
    }

    #[test]
    fn wrong_password_is_refused() {
        let mut w = Writer::new(vec![]);
        w.add_bytes_encrypted("s.txt", b"classified", 0, b"right", [9; 12]).unwrap();
        let mut ar = Archive::open(Cursor::new(w.finish().unwrap())).unwrap();
        let m = ar.members[0].clone();
        assert!(ar.read_with_password(&m, b"wrong").is_err());
        assert!(ar.read_with_password(&m, b"").is_err());
        assert_eq!(ar.read_with_password(&m, b"right").unwrap(), b"classified");
    }

    #[test]
    fn sealed_member_opens_only_with_its_check_byte() {
        // the key schedule is a pure function of the password, and the check byte (the CRC's high byte) gates the open
        let a = encrypt_member(&[7; 12], b"same bytes", b"pw");
        assert_eq!(a, encrypt_member(&[7; 12], b"same bytes", b"pw"));
        assert_ne!(a, encrypt_member(&[7; 12], b"same bytes", b"pX"));
        assert_eq!(a.len(), 12 + 10);
        assert_eq!(decrypt_member(&a, 7 << 24, b"pw").unwrap(), b"same bytes");
        assert!(decrypt_member(&a, 0, b"pw").is_err(), "check byte 7 does not match a zero CRC");
        assert!(decrypt_member(&a[..5], 0, b"pw").is_err(), "a short member is damaged");
    }

    #[test]
    fn safe_rel_refuses_escapes() {
        assert_eq!(safe_rel("a/b.txt"), Some(PathBuf::from("a/b.txt")));
        assert_eq!(safe_rel("./a//b/"), Some(PathBuf::from("a/b")));
        assert_eq!(safe_rel("a\\b.txt"), Some(PathBuf::from("a/b.txt")));
        for evil in ["../x", "a/../../x", "/etc/passwd", "..\\x", "C:/x", "a\0b", "", "./", "a/.."] {
            assert_eq!(safe_rel(evil), None, "{evil:?}");
        }
        assert_eq!(safe_rel("..hidden/x"), Some(PathBuf::from("..hidden/x")));
    }

    #[test]
    fn dos_time_encodes() {
        assert_eq!(dos_time(0), (0, (1 << 5) | 1));
        // 2026-10-05 12:30
        let unix = 1_790_000_000 - (1_790_000_000 % 86_400) + 12 * 3600 + 30 * 60;
        let (y, mo, d, h, mi) = fs::civil(unix);
        let (t, dt) = dos_time(unix);
        assert_eq!(t, (h as u16) << 11 | (mi as u16) << 5);
        assert_eq!(dt, ((y - 1980) as u16) << 9 | (mo as u16) << 5 | d as u16);
    }

    #[test]
    fn extract_skips_zip_slip_members() {
        let dir = fs::tests::temp("zip-slip");
        let mut w = Writer::new(vec![]);
        w.add_bytes("ok/fine.txt", b"fine", 0).unwrap();
        w.add_bytes("../evil.txt", b"evil", 0).unwrap();
        w.add_bytes("/abs.txt", b"evil", 0).unwrap();
        std::fs::write(dir.join("t.zip"), w.finish().unwrap()).unwrap();
        let mut last = (0, 0);
        let r = extract(&dir.join("t.zip"), &dir, &mut |d, t| {
            last = (d, t);
            true
        })
        .unwrap();
        assert_eq!(r.dir, dir.join("t"));
        assert_eq!(r.files, 1);
        assert_eq!(r.skipped, ["../evil.txt", "/abs.txt"]);
        assert_eq!(std::fs::read(dir.join("t/ok/fine.txt")).unwrap(), b"fine");
        assert!(!dir.join("evil.txt").exists() && !dir.join("t/abs.txt").exists());
        assert_eq!(last.0, last.1);
        // a second extraction numbers the folder
        let r2 = extract(&dir.join("t.zip"), &dir, &mut |_, _| true).unwrap();
        assert_eq!(r2.dir, dir.join("t 2"));
    }

    #[test]
    fn extract_cancel_removes_partial_folder() {
        let dir = fs::tests::temp("zip-cancel");
        std::fs::write(dir.join("h.zip"), hand_built("a.txt", b"hi")).unwrap();
        let e = extract(&dir.join("h.zip"), &dir, &mut |_, _| false).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert!(!dir.join("h").exists());
    }

    #[test]
    fn create_then_extract_folder_tree() {
        let dir = fs::tests::temp("zip-create");
        let src = dir.join("proj");
        std::fs::create_dir_all(src.join("sub/empty")).unwrap();
        std::fs::write(src.join("a.txt"), "alpha ".repeat(500)).unwrap();
        std::fs::write(src.join("sub/b.bin"), [9u8, 8, 7]).unwrap();
        std::fs::write(dir.join("solo.txt"), "solo").unwrap();
        let out_dir = dir.join("out");
        std::fs::create_dir(&out_dir).unwrap();
        let mut ticks = 0;
        let z = create(&[src.clone(), dir.join("solo.txt")], &out_dir, &mut |_, _| {
            ticks += 1;
            true
        })
        .unwrap();
        assert_eq!(z, out_dir.join("Archive.zip"));
        assert!(ticks >= 3);
        let r = extract(&z, &out_dir, &mut |_, _| true).unwrap();
        assert_eq!(r.files, 3);
        assert_eq!(std::fs::read_to_string(r.dir.join("proj/a.txt")).unwrap(), "alpha ".repeat(500));
        assert_eq!(std::fs::read(r.dir.join("proj/sub/b.bin")).unwrap(), [9, 8, 7]);
        assert!(r.dir.join("proj/sub/empty").is_dir());
        assert_eq!(std::fs::read_to_string(r.dir.join("solo.txt")).unwrap(), "solo");
        // a single source names the archive after itself
        assert_eq!(archive_name(&[src]), "proj.zip");
    }

    #[test]
    fn create_cancel_leaves_nothing() {
        let dir = fs::tests::temp("zip-create-cancel");
        std::fs::write(dir.join("f.txt"), "x").unwrap();
        let e = create(&[dir.join("f.txt")], &dir, &mut |_, _| false).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert!(!dir.join("f.txt.zip").exists());
    }
}
