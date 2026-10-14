//! Wire type for an app's custom rotors, `ToShell::AxRotors`, tag `0x213`: the named lists the screen reader steps through
//! beside the standard rotors (`bifrost_ui::ax_rotor`).
//!
//! Layout: `u32 tag`, `u32 count` (at most `MAX_ROTORS`), then per rotor a string name (at most `MAX_NAME` bytes), `u32 n`
//! (at most `MAX_ITEMS`) and `n` pairs of `u32` (low, high) carrying each item's `Node::id`. Decoding refuses a count or a name
//! over its cap; encoding cuts them, so an oversized list is shortened rather than dropped.

use super::axtext::{cut, get};
use super::{u32_, DecodeError, Reader, ToShell};
use bifrost_ui::ax::AxCustomRotor;
use bifrost_ui::ax_rotor::{MAX_CUSTOM_ROTORS, MAX_ROTOR_ITEMS, MAX_ROTOR_NAME};

pub const TAG_AX_ROTORS: u32 = 0x213;

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    if let ToShell::AxRotors(list) = m {
        u32_(b, TAG_AX_ROTORS);
        let n = list.len().min(MAX_CUSTOM_ROTORS);
        u32_(b, n as u32);
        for rot in &list[..n] {
            super::str_(b, cut(&rot.name, MAX_ROTOR_NAME));
            let k = rot.items.len().min(MAX_ROTOR_ITEMS);
            u32_(b, k as u32);
            for id in &rot.items[..k] {
                u32_(b, *id as u32);
                u32_(b, (*id >> 32) as u32);
            }
        }
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    if t != TAG_AX_ROTORS {
        return Err(DecodeError::UnknownTag(t));
    }
    let n = r.u32()? as usize;
    if n > MAX_CUSTOM_ROTORS {
        return Err(DecodeError::BadValue);
    }
    let mut list = Vec::with_capacity(n);
    for _ in 0..n {
        let name = get(r, MAX_ROTOR_NAME)?;
        let k = r.u32()? as usize;
        if k > MAX_ROTOR_ITEMS {
            return Err(DecodeError::BadValue);
        }
        let mut items = Vec::with_capacity(k);
        for _ in 0..k {
            items.push(r.u32()? as u64 | ((r.u32()? as u64) << 32));
        }
        list.push(AxCustomRotor { name, items });
    }
    Ok(ToShell::AxRotors(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_rotors_round_trip_with_wide_ids() {
        let list = vec![AxCustomRotor::new("Unread", vec![1 << 40 | 5, 2]), AxCustomRotor::new("Errors", vec![u64::MAX])];
        assert_eq!(ToShell::decode(&ToShell::AxRotors(list.clone()).encode()), Ok(ToShell::AxRotors(list)));
        assert_eq!(ToShell::decode(&ToShell::AxRotors(Vec::new()).encode()), Ok(ToShell::AxRotors(Vec::new())));
    }

    #[test]
    fn oversized_lists_are_cut_on_encode_and_refused_on_decode() {
        let many: Vec<AxCustomRotor> = (0..MAX_CUSTOM_ROTORS + 3).map(|i| AxCustomRotor::new(&format!("r{i}"), vec![i as u64])).collect();
        let ToShell::AxRotors(back) = ToShell::decode(&ToShell::AxRotors(many).encode()).unwrap() else { panic!() };
        assert_eq!(back.len(), MAX_CUSTOM_ROTORS);

        let mut bad = ToShell::AxRotors(vec![AxCustomRotor::new("x", vec![1])]).encode();
        bad[4..8].copy_from_slice(&(MAX_CUSTOM_ROTORS as u32 + 1).to_le_bytes());
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }
}
