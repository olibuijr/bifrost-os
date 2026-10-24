//! Wire type for the named custom actions of an app's elements, `ToShell::AxActions`, tag `0x215`
//! (`bifrost_ui::ax_action`). The shell lists the names on the AT-SPI `Action` interface and runs one by sending
//! `ToApp::AxSet { action: AxAction::Custom(name) }` back.
//!
//! Layout: `u32 tag`, `u32 count` (at most `MAX_ACTION_ELEMENTS`), then per element a `u32` pair (low, high) carrying its
//! `Node::id`, `u32 n` (at most `MAX_ACTIONS_PER_ELEMENT`) and `n` strings (each at most `MAX_ACTION_NAME` bytes). Decoding
//! refuses a count or a name over its cap; encoding cuts them, so an oversized list is shortened rather than dropped.

use super::axtext::{cut, get};
use super::{str_, u32_, DecodeError, Reader, ToShell};
use bifrost_ui::ax_action::{AxActionSet, MAX_ACTIONS_PER_ELEMENT, MAX_ACTION_ELEMENTS, MAX_ACTION_NAME};

pub const TAG_AX_ACTIONS: u32 = 0x215;

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    if let ToShell::AxActions(list) = m {
        u32_(b, TAG_AX_ACTIONS);
        let n = list.len().min(MAX_ACTION_ELEMENTS);
        u32_(b, n as u32);
        for set in &list[..n] {
            u32_(b, set.id as u32);
            u32_(b, (set.id >> 32) as u32);
            let k = set.names.len().min(MAX_ACTIONS_PER_ELEMENT);
            u32_(b, k as u32);
            for name in &set.names[..k] {
                str_(b, cut(name, MAX_ACTION_NAME));
            }
        }
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    if t != TAG_AX_ACTIONS {
        return Err(DecodeError::UnknownTag(t));
    }
    let n = r.u32()? as usize;
    if n > MAX_ACTION_ELEMENTS {
        return Err(DecodeError::BadValue);
    }
    let mut list = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.u32()? as u64 | ((r.u32()? as u64) << 32);
        let k = r.u32()? as usize;
        if k > MAX_ACTIONS_PER_ELEMENT {
            return Err(DecodeError::BadValue);
        }
        let mut names = Vec::with_capacity(k);
        for _ in 0..k {
            names.push(get(r, MAX_ACTION_NAME)?);
        }
        list.push(AxActionSet { id, names });
    }
    Ok(ToShell::AxActions(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AxAction, ToApp};

    #[test]
    fn action_sets_round_trip_with_wide_ids() {
        let list = vec![AxActionSet::new(1 << 40 | 5, &["Archive", "Mark as Read"]), AxActionSet::new(u64::MAX, &["R\u{e9}pondre"])];
        assert_eq!(ToShell::decode(&ToShell::AxActions(list.clone()).encode()), Ok(ToShell::AxActions(list)));
        assert_eq!(ToShell::decode(&ToShell::AxActions(Vec::new()).encode()), Ok(ToShell::AxActions(Vec::new())));
    }

    #[test]
    fn oversized_lists_are_cut_on_encode_and_refused_on_decode() {
        let many: Vec<String> = (0..MAX_ACTIONS_PER_ELEMENT + 3).map(|i| format!("a{i}")).collect();
        let m = ToShell::AxActions(vec![AxActionSet { id: 1, names: many }]);
        let ToShell::AxActions(back) = ToShell::decode(&m.encode()).unwrap() else { panic!() };
        assert_eq!(back[0].names.len(), MAX_ACTIONS_PER_ELEMENT);

        let mut bad = ToShell::AxActions(vec![AxActionSet::new(1, &["x"])]).encode();
        bad[4..8].copy_from_slice(&(MAX_ACTION_ELEMENTS as u32 + 1).to_le_bytes());
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }

    #[test]
    fn a_custom_action_request_round_trips() {
        let m = ToApp::AxSet { id: 1 << 33 | 2, action: AxAction::Custom("Archive".into()) };
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }
}
