//! The native protocol's tagged-value encoding: every value is a one-byte tag followed by its big-endian payload.
//! Tags: `L` u32, `B` u8, `R` u64, `r` i64, `a` sample spec, `x` arbitrary, `1`/`0` booleans, `T` timeval,
//! `U` microseconds, `m` channel map, `v` cvolume, `V` volume, `P` property list, `f` format info,
//! `t` string, `N` the null string.

pub const T_STRING: u8 = b't';
pub const T_NULL: u8 = b'N';
pub const T_U32: u8 = b'L';
pub const T_U8: u8 = b'B';
pub const T_U64: u8 = b'R';
pub const T_S64: u8 = b'r';
pub const T_SAMPLE_SPEC: u8 = b'a';
pub const T_ARBITRARY: u8 = b'x';
pub const T_TRUE: u8 = b'1';
pub const T_FALSE: u8 = b'0';
pub const T_TIMEVAL: u8 = b'T';
pub const T_USEC: u8 = b'U';
pub const T_CHANNEL_MAP: u8 = b'm';
pub const T_CVOLUME: u8 = b'v';
pub const T_VOLUME: u8 = b'V';
pub const T_PROPLIST: u8 = b'P';
pub const T_FORMAT_INFO: u8 = b'f';

/// Builds a tagged payload.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.push(T_U32);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(T_U8);
        self.buf.push(v);
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.push(T_U64);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn s64(&mut self, v: i64) -> &mut Self {
        self.buf.push(T_S64);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn usec(&mut self, v: u64) -> &mut Self {
        self.buf.push(T_USEC);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.buf.push(if v { T_TRUE } else { T_FALSE });
        self
    }
    pub fn string(&mut self, v: Option<&str>) -> &mut Self {
        match v {
            Some(s) => {
                self.buf.push(T_STRING);
                self.buf.extend_from_slice(s.as_bytes());
                self.buf.push(0);
            }
            None => self.buf.push(T_NULL),
        }
        self
    }
    pub fn arbitrary(&mut self, v: &[u8]) -> &mut Self {
        self.buf.push(T_ARBITRARY);
        self.buf.extend_from_slice(&(v.len() as u32).to_be_bytes());
        self.buf.extend_from_slice(v);
        self
    }
    pub fn timeval(&mut self, sec: u32, usec: u32) -> &mut Self {
        self.buf.push(T_TIMEVAL);
        self.buf.extend_from_slice(&sec.to_be_bytes());
        self.buf.extend_from_slice(&usec.to_be_bytes());
        self
    }
    pub fn sample_spec(&mut self, format: u8, channels: u8, rate: u32) -> &mut Self {
        self.buf.push(T_SAMPLE_SPEC);
        self.buf.push(format);
        self.buf.push(channels);
        self.buf.extend_from_slice(&rate.to_be_bytes());
        self
    }
    pub fn channel_map(&mut self, map: &[u8]) -> &mut Self {
        self.buf.push(T_CHANNEL_MAP);
        self.buf.push(map.len() as u8);
        self.buf.extend_from_slice(map);
        self
    }
    pub fn cvolume(&mut self, vols: &[u32]) -> &mut Self {
        self.buf.push(T_CVOLUME);
        self.buf.push(vols.len() as u8);
        for v in vols {
            self.buf.extend_from_slice(&v.to_be_bytes());
        }
        self
    }
    pub fn volume(&mut self, v: u32) -> &mut Self {
        self.buf.push(T_VOLUME);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    /// Entries are (key, value bytes); a string value includes its trailing NUL.
    pub fn proplist(&mut self, entries: &[(String, Vec<u8>)]) -> &mut Self {
        self.buf.push(T_PROPLIST);
        for (k, v) in entries {
            self.string(Some(k));
            self.u32(v.len() as u32);
            self.arbitrary(v);
        }
        self.string(None);
        self
    }
    /// A format info: the encoding (1 = PCM) and an empty property list.
    pub fn format_info(&mut self, encoding: u8) -> &mut Self {
        self.buf.push(T_FORMAT_INFO);
        self.u8(encoding);
        self.proplist(&[]);
        self
    }
}

/// Why a payload did not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

/// Reads a tagged payload.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }
    fn tag(&mut self, t: u8) -> Result<(), Malformed> {
        if self.data.get(self.pos) == Some(&t) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Malformed)
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        let end = self.pos.checked_add(n).ok_or(Malformed)?;
        let s = self.data.get(self.pos..end).ok_or(Malformed)?;
        self.pos = end;
        Ok(s)
    }
    pub fn u32(&mut self) -> Result<u32, Malformed> {
        self.tag(T_U32)?;
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// Wire primitive the server does not read yet (no incoming command carries it).
    #[allow(dead_code)]
    pub fn u8(&mut self) -> Result<u8, Malformed> {
        self.tag(T_U8)?;
        Ok(self.take(1)?[0])
    }
    /// Wire primitive the server does not read yet (no incoming command carries it).
    #[allow(dead_code)]
    pub fn u64(&mut self) -> Result<u64, Malformed> {
        self.tag(T_U64)?;
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    #[cfg(test)]
    pub fn s64(&mut self) -> Result<i64, Malformed> {
        self.tag(T_S64)?;
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    #[cfg(test)]
    pub fn usec(&mut self) -> Result<u64, Malformed> {
        self.tag(T_USEC)?;
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn bool(&mut self) -> Result<bool, Malformed> {
        match self.data.get(self.pos) {
            Some(&T_TRUE) => {
                self.pos += 1;
                Ok(true)
            }
            Some(&T_FALSE) => {
                self.pos += 1;
                Ok(false)
            }
            _ => Err(Malformed),
        }
    }
    pub fn string(&mut self) -> Result<Option<String>, Malformed> {
        match self.data.get(self.pos) {
            Some(&T_NULL) => {
                self.pos += 1;
                Ok(None)
            }
            Some(&T_STRING) => {
                let rest = &self.data[self.pos + 1..];
                let n = rest.iter().position(|&b| b == 0).ok_or(Malformed)?;
                let s = String::from_utf8_lossy(&rest[..n]).into_owned();
                self.pos += 1 + n + 1;
                Ok(Some(s))
            }
            _ => Err(Malformed),
        }
    }
    pub fn arbitrary(&mut self) -> Result<&'a [u8], Malformed> {
        self.tag(T_ARBITRARY)?;
        let n = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
    pub fn timeval(&mut self) -> Result<(u32, u32), Malformed> {
        self.tag(T_TIMEVAL)?;
        let b = self.take(8)?;
        Ok((u32::from_be_bytes(b[..4].try_into().unwrap()), u32::from_be_bytes(b[4..].try_into().unwrap())))
    }
    /// (format, channels, rate)
    pub fn sample_spec(&mut self) -> Result<(u8, u8, u32), Malformed> {
        self.tag(T_SAMPLE_SPEC)?;
        let b = self.take(6)?;
        Ok((b[0], b[1], u32::from_be_bytes(b[2..6].try_into().unwrap())))
    }
    pub fn channel_map(&mut self) -> Result<Vec<u8>, Malformed> {
        self.tag(T_CHANNEL_MAP)?;
        let n = self.take(1)?[0] as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub fn cvolume(&mut self) -> Result<Vec<u32>, Malformed> {
        self.tag(T_CVOLUME)?;
        let n = self.take(1)?[0] as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(u32::from_be_bytes(self.take(4)?.try_into().unwrap()));
        }
        Ok(v)
    }
    /// Wire primitive the server does not read yet (volumes arrive as cvolume).
    #[allow(dead_code)]
    pub fn volume(&mut self) -> Result<u32, Malformed> {
        self.tag(T_VOLUME)?;
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn proplist(&mut self) -> Result<Vec<(String, Vec<u8>)>, Malformed> {
        self.tag(T_PROPLIST)?;
        let mut out = Vec::new();
        while let Some(key) = self.string()? {
            let len = self.u32()? as usize;
            let val = self.arbitrary()?;
            if val.len() != len {
                return Err(Malformed);
            }
            out.push((key, val.to_vec()));
        }
        Ok(out)
    }
    // Not read by the server yet: the client's format list is not parsed.
    /// Skips a format info; returns its encoding.
    #[allow(dead_code)]
    pub fn format_info(&mut self) -> Result<u8, Malformed> {
        self.tag(T_FORMAT_INFO)?;
        let enc = self.u8()?;
        self.proplist()?;
        Ok(enc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_is_tag_then_big_endian() {
        let mut w = Writer::new();
        w.u32(0x0102_0304);
        assert_eq!(w.buf, vec![b'L', 1, 2, 3, 4]);
    }

    #[test]
    fn string_is_nul_terminated_and_null_is_n() {
        let mut w = Writer::new();
        w.string(Some("ab")).string(None);
        assert_eq!(w.buf, vec![b't', b'a', b'b', 0, b'N']);
    }

    #[test]
    fn sample_spec_layout() {
        let mut w = Writer::new();
        w.sample_spec(3, 2, 44100);
        // 44100 = 0x0000AC44
        assert_eq!(w.buf, vec![b'a', 3, 2, 0, 0, 0xAC, 0x44]);
    }

    #[test]
    fn round_trip_all() {
        let mut w = Writer::new();
        w.u32(7).u8(9).u64(1 << 40).s64(-5).usec(123).bool(true).bool(false).string(Some("héllo")).string(None);
        w.arbitrary(&[1, 2, 3]).timeval(4, 5).sample_spec(5, 2, 48000).channel_map(&[0, 1]).cvolume(&[0x10000, 0x8000]).volume(3);
        w.proplist(&[("application.name".to_string(), b"mpv\0".to_vec())]);
        w.format_info(1);
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.u32(), Ok(7));
        assert_eq!(r.u8(), Ok(9));
        assert_eq!(r.u64(), Ok(1 << 40));
        assert_eq!(r.s64(), Ok(-5));
        assert_eq!(r.usec(), Ok(123));
        assert_eq!(r.bool(), Ok(true));
        assert_eq!(r.bool(), Ok(false));
        assert_eq!(r.string(), Ok(Some("héllo".to_string())));
        assert_eq!(r.string(), Ok(None));
        assert_eq!(r.arbitrary(), Ok(&[1u8, 2, 3][..]));
        assert_eq!(r.timeval(), Ok((4, 5)));
        assert_eq!(r.sample_spec(), Ok((5, 2, 48000)));
        assert_eq!(r.channel_map(), Ok(vec![0, 1]));
        assert_eq!(r.cvolume(), Ok(vec![0x10000, 0x8000]));
        assert_eq!(r.volume(), Ok(3));
        assert_eq!(r.proplist(), Ok(vec![("application.name".to_string(), b"mpv\0".to_vec())]));
        assert_eq!(r.format_info(), Ok(1));
        assert!(r.is_empty());
    }

    #[test]
    fn wrong_tag_and_truncation_are_malformed() {
        let mut r = Reader::new(&[b'B', 1]);
        assert_eq!(r.u32(), Err(Malformed));
        let mut r = Reader::new(&[b'L', 0, 0]);
        assert_eq!(r.u32(), Err(Malformed));
        let mut r = Reader::new(&[b't', b'a']);
        assert_eq!(r.string(), Err(Malformed));
        let mut r = Reader::new(&[b'x', 0, 0, 0, 9, 1]);
        assert_eq!(r.arbitrary(), Err(Malformed));
    }
}
