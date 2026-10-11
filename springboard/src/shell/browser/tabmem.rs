//! What must survive a tab discard, and its compact byte format.
//!
//! When `budget::DiscardLevel::DropDom` throws a tab's DOM away, the tab keeps
//! only a `DiscardState`: enough to reload the page and put the user back
//! where they were. The format is little-endian: magic `TDS1`, history index
//! (u32), scroll x/y (f32), url, title (u32 length + UTF-8 each), then a u32 count of key/value string pairs.

const MAGIC: &[u8; 4] = b"TDS1";
/// Refuse absurd inputs when decoding (corrupt or hostile bytes).
const MAX_FORM_FIELDS: usize = 10_000;

/// Errors from `DiscardState::decode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    Truncated,
    BadUtf8,
    TooMany,
}

/// One remembered form control value, keyed by the page's own identifier
/// (name, id or document-order path).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormValue {
    pub key: String,
    pub value: String,
}

/// The state a discarded tab keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscardState {
    pub url: String,
    pub title: String,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub form: Vec<FormValue>,
    /// Position in the tab's session history.
    pub history_index: u32,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| DecodeError::BadUtf8)
    }
}

impl DiscardState {
    /// Serialises into the compact format.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + self.url.len() + self.title.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.history_index.to_le_bytes());
        out.extend_from_slice(&self.scroll_x.to_le_bytes());
        out.extend_from_slice(&self.scroll_y.to_le_bytes());
        put_str(&mut out, &self.url);
        put_str(&mut out, &self.title);
        out.extend_from_slice(&(self.form.len() as u32).to_le_bytes());
        for f in &self.form {
            put_str(&mut out, &f.key);
            put_str(&mut out, &f.value);
        }
        out
    }

    /// Restores from `encode` output; never panics on bad input.
    pub fn decode(bytes: &[u8]) -> Result<DiscardState, DecodeError> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        let history_index = r.u32()?;
        let scroll_x = r.f32()?;
        let scroll_y = r.f32()?;
        let url = r.string()?;
        let title = r.string()?;
        let n = r.u32()? as usize;
        if n > MAX_FORM_FIELDS {
            return Err(DecodeError::TooMany);
        }
        let mut form = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            form.push(FormValue { key: r.string()?, value: r.string()? });
        }
        Ok(DiscardState { url, title, scroll_x, scroll_y, form, history_index })
    }

    /// Approximate resident bytes of this state.
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<DiscardState>() + self.url.len() + self.title.len() + self.form.iter().map(|f| f.key.len() + f.value.len() + 48).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DiscardState {
        DiscardState {
            url: "https://example.com/a?b=c#d".into(),
            title: "Hæ heimur \u{1F600}".into(),
            scroll_x: 12.5,
            scroll_y: 3456.75,
            form: vec![
                FormValue { key: "q".into(), value: "search term".into() },
                FormValue { key: "#pw".into(), value: String::new() },
            ],
            history_index: 7,
        }
    }

    #[test]
    fn round_trip() {
        let s = sample();
        assert_eq!(DiscardState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn round_trip_empty() {
        let s = DiscardState::default();
        assert_eq!(DiscardState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = sample().encode();
        b[0] = b'X';
        assert_eq!(DiscardState::decode(&b), Err(DecodeError::BadMagic));
    }

    #[test]
    fn every_truncation_errors_without_panic() {
        let b = sample().encode();
        for n in 0..b.len() {
            assert!(DiscardState::decode(&b[..n]).is_err(), "prefix {n}");
        }
    }

    #[test]
    fn rejects_bad_utf8() {
        let mut b = DiscardState { url: "ab".into(), ..Default::default() }.encode();
        // url bytes start after magic(4)+idx(4)+sx(4)+sy(4)+len(4)
        b[20] = 0xFF;
        assert_eq!(DiscardState::decode(&b), Err(DecodeError::BadUtf8));
    }

    #[test]
    fn rejects_huge_field_count_and_lengths() {
        let mut b = DiscardState::default().encode();
        let at = b.len() - 4;
        b[at..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(DiscardState::decode(&b), Err(DecodeError::TooMany));
        let mut b = DiscardState::default().encode();
        b[16..20].copy_from_slice(&u32::MAX.to_le_bytes()); // url length
        assert_eq!(DiscardState::decode(&b), Err(DecodeError::Truncated));
    }

    #[test]
    fn compact() {
        let b = DiscardState { url: "https://a.b/".into(), title: "t".into(), ..Default::default() }.encode();
        assert_eq!(b.len(), 4 + 12 + 4 + 12 + 4 + 1 + 4);
    }
}
