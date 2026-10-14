//! Wire types for the accessibility text attributes. Tags `0x210` to `0x212`:
//! `ToShell::AxText` 0x210 (selection, caret and visible range per text element), `ToShell::AxMeta` 0x211
//! (`accessibilityTitle` and `accessibilityHelp`), `ToShell::Announce` 0x212 (the announcement-requested accessibility notification).
//! Lists hold at most `MAX_ENTRIES`; strings are cut on encode and refused when longer on decode.

use super::{str_, u32_, DecodeError, Reader, ToShell};
use bifrost_ui::ax::{AxMeta, AxText, Priority};

pub const MAX_ENTRIES: usize = 32;
pub const MAX_META: usize = 160;
pub const MAX_ANNOUNCE: usize = 512;

pub(crate) fn cut(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

pub(crate) fn get(r: &mut Reader, max: usize) -> Result<String, DecodeError> {
    let s = r.str()?;
    if s.len() > max {
        return Err(DecodeError::BadValue);
    }
    Ok(s)
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    match m {
        ToShell::AxText(list) => {
            u32_(b, 0x210);
            let n = list.len().min(MAX_ENTRIES);
            u32_(b, n as u32);
            for t in &list[..n] {
                u32_(b, t.id as u32);
                u32_(b, (t.id >> 32) as u32);
                for v in [t.caret, t.sel_start, t.sel_end, t.vis_start, t.vis_end, t.len] {
                    u32_(b, v);
                }
                b.push(t.secure as u8);
            }
        }
        ToShell::AxMeta(list) => {
            u32_(b, 0x211);
            let n = list.len().min(MAX_ENTRIES);
            u32_(b, n as u32);
            for e in &list[..n] {
                u32_(b, e.id as u32);
                u32_(b, (e.id >> 32) as u32);
                str_(b, cut(&e.title, MAX_META));
                str_(b, cut(&e.help, MAX_META));
            }
        }
        ToShell::Announce { text, priority } => {
            u32_(b, 0x212);
            b.push(*priority as u8);
            str_(b, cut(text, MAX_ANNOUNCE));
        }
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    Ok(match t {
        0x210 => {
            let n = r.u32()? as usize;
            if n > MAX_ENTRIES {
                return Err(DecodeError::BadValue);
            }
            let mut list = Vec::with_capacity(n);
            for _ in 0..n {
                let id = r.u32()? as u64 | ((r.u32()? as u64) << 32);
                let (caret, sel_start, sel_end, vis_start, vis_end, len) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let secure = match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(DecodeError::BadValue),
                };
                if sel_start > sel_end || vis_start > vis_end {
                    return Err(DecodeError::BadValue);
                }
                list.push(AxText { id, caret, sel_start, sel_end, vis_start, vis_end, secure, len });
            }
            ToShell::AxText(list)
        }
        0x211 => {
            let n = r.u32()? as usize;
            if n > MAX_ENTRIES {
                return Err(DecodeError::BadValue);
            }
            let mut list = Vec::with_capacity(n);
            for _ in 0..n {
                let id = r.u32()? as u64 | ((r.u32()? as u64) << 32);
                list.push(AxMeta { id, title: get(r, MAX_META)?, help: get(r, MAX_META)? });
            }
            ToShell::AxMeta(list)
        }
        0x212 => {
            let priority = Priority::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
            ToShell::Announce { priority, text: get(r, MAX_ANNOUNCE)? }
        }
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ax_text_round_trips_and_rejects_inverted_ranges() {
        let list = vec![
            AxText { id: 1 << 40 | 5, caret: 3, sel_start: 1, sel_end: 3, vis_start: 0, vis_end: 9, secure: false, len: 9 },
            AxText { id: 2, caret: 4, sel_start: 4, sel_end: 4, vis_start: 0, vis_end: 4, secure: true, len: 4 },
        ];
        assert_eq!(ToShell::decode(&ToShell::AxText(list.clone()).encode()), Ok(ToShell::AxText(list)));
        let bad = vec![AxText { id: 1, caret: 0, sel_start: 5, sel_end: 2, vis_start: 0, vis_end: 0, secure: false, len: 5 }];
        assert_eq!(ToShell::decode(&ToShell::AxText(bad).encode()), Err(DecodeError::BadValue));
    }

    #[test]
    fn ax_meta_and_announce_round_trip_within_caps() {
        let m = ToShell::AxMeta(vec![AxMeta { id: 9, title: "Name".into(), help: "Type your name".into() }]);
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        let long = ToShell::Announce { text: "\u{e9}".repeat(900), priority: Priority::High };
        let ToShell::Announce { text, priority } = ToShell::decode(&long.encode()).unwrap() else { panic!() };
        assert!(text.len() <= MAX_ANNOUNCE);
        assert_eq!(priority, Priority::High);
        let mut bad = ToShell::Announce { text: "x".into(), priority: Priority::Low }.encode();
        bad[4] = 7;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }
}
