//! Wire types for Preview (`QLPreviewPanel`). Tags `0x1e0` to `0x1ef`:
//! `ToShell::Preview` 0x1e0, `ToShell::PreviewClose` 0x1e1. Strings are cut to a byte cap on encode and refused when longer on decode.

use super::{str_, u32_, DecodeError, Reader, ToShell};

pub const MAX_ITEMS: usize = 16;
pub const MAX_URL: usize = 256;
pub const MAX_TITLE: usize = 96;

/// `QLPreviewItem`: `previewItemURL` and `previewItemTitle`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewItem {
    pub url: String,
    pub title: String,
}

impl PreviewItem {
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        PreviewItem { url: url.into(), title: title.into() }
    }

    /// The last path segment of an absolute path or `file://` URL that names a file. None for other URLs and for a
    /// URL that ends in a slash. Percent escapes are not decoded.
    pub fn file_name(&self) -> Option<&str> {
        let path = self.url.strip_prefix("file://").unwrap_or(self.url.as_str());
        if !path.starts_with('/') {
            return None;
        }
        path.rsplit('/').next().filter(|name| !name.is_empty())
    }
}

fn cut(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

fn get(r: &mut Reader, max: usize) -> Result<String, DecodeError> {
    let s = r.str()?;
    if s.len() > max {
        return Err(DecodeError::BadValue);
    }
    Ok(s)
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    match m {
        ToShell::Preview { items, index } => {
            u32_(b, 0x1e0);
            let n = items.len().min(MAX_ITEMS);
            u32_(b, (*index).min(n.saturating_sub(1) as u32));
            u32_(b, n as u32);
            for it in &items[..n] {
                str_(b, cut(&it.url, MAX_URL));
                str_(b, cut(&it.title, MAX_TITLE));
            }
        }
        ToShell::PreviewClose => u32_(b, 0x1e1),
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    Ok(match t {
        0x1e0 => {
            let index = r.u32()?;
            let n = r.u32()? as usize;
            if n > MAX_ITEMS {
                return Err(DecodeError::BadValue);
            }
            let items = (0..n)
                .map(|_| Ok(PreviewItem { url: get(r, MAX_URL)?, title: get(r, MAX_TITLE)? }))
                .collect::<Result<Vec<_>, DecodeError>>()?;
            ToShell::Preview { items, index }
        }
        0x1e1 => ToShell::PreviewClose,
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_round_trips_and_stays_in_one_packet() {
        let it = PreviewItem { url: "f".repeat(900), title: "é".repeat(300) };
        let bytes = ToShell::Preview { items: vec![it; 40], index: 99 }.encode();
        assert!(bytes.len() <= crate::MAX_PACKET);
        let ToShell::Preview { items, index } = ToShell::decode(&bytes).unwrap() else { panic!() };
        assert_eq!((items.len(), index), (MAX_ITEMS, MAX_ITEMS as u32 - 1));
        assert!(items[0].title.len() <= MAX_TITLE);
        assert_eq!(ToShell::decode(&ToShell::PreviewClose.encode()), Ok(ToShell::PreviewClose));
    }
}
