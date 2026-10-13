//! Login items (`SMAppService`) and menu-bar extras (`NativeStatusItem`). Tags 0x1b0..=0x1bf in both directions.

use super::{str_, u32_, DecodeError, Reader, ToApp, ToShell};

pub const MAX_STATUS_MENU: usize = 32;
pub const MAX_STATUS_TEXT: usize = 64;
pub const MAX_STATUS_ITEMS_PER_APP: usize = 4;

/// What `SMAppService.register()` and friends ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginOp {
    Register,
    Unregister,
    Query,
    /// `openSystemSettingsLoginItems()`: the shell opens Settings on the Login Items page
    OpenSettings,
}

impl LoginOp {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Register,
            1 => Self::Unregister,
            2 => Self::Query,
            3 => Self::OpenSettings,
            _ => return None,
        })
    }
}

/// `SMAppService.Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStatus {
    NotRegistered,
    Enabled,
    /// never produced by BifrOSt (there is no Login Items approval step); kept for source parity
    RequiresApproval,
    NotFound,
}

impl LoginStatus {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::NotRegistered,
            1 => Self::Enabled,
            2 => Self::RequiresApproval,
            3 => Self::NotFound,
            _ => return None,
        })
    }
}

/// One `NativeMenuItem` of a status item's menu. `separator` rows ignore the other fields.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StatusMenuItem {
    pub id: u32,
    pub title: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
}

fn put(b: &mut Vec<u8>, s: &str) {
    let cut: String = s.chars().take(MAX_STATUS_TEXT).collect();
    str_(b, &cut);
}

fn get(r: &mut Reader) -> Result<String, DecodeError> {
    let s = r.str()?;
    if s.chars().count() > MAX_STATUS_TEXT {
        return Err(DecodeError::BadValue);
    }
    Ok(s)
}

fn flag(r: &mut Reader) -> Result<bool, DecodeError> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::BadValue),
    }
}

pub(crate) mod tag {
    pub const LOGIN_ITEM: u32 = 0x1b0;
    pub const STATUS_ITEM_SET: u32 = 0x1b1;
    pub const STATUS_ITEM_REMOVE: u32 = 0x1b2;
    pub const LOGIN_ITEM_STATUS: u32 = 0x1b0;
    pub const STATUS_ITEM_ACTION: u32 = 0x1b1;
}

pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    match m {
        ToApp::LoginItemStatus { request, status } => {
            u32_(b, tag::LOGIN_ITEM_STATUS);
            u32_(b, *request);
            b.push(*status as u8);
        }
        ToApp::StatusItemAction { item, id } => {
            u32_(b, tag::STATUS_ITEM_ACTION);
            u32_(b, *item);
            u32_(b, *id);
        }
        _ => {}
    }
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    Ok(match t {
        tag::LOGIN_ITEM_STATUS => ToApp::LoginItemStatus { request: r.u32()?, status: LoginStatus::from_wire(r.u8()?).ok_or(DecodeError::BadValue)? },
        tag::STATUS_ITEM_ACTION => ToApp::StatusItemAction { item: r.u32()?, id: r.u32()? },
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    match m {
        ToShell::LoginItem { request, op } => {
            u32_(b, tag::LOGIN_ITEM);
            u32_(b, *request);
            b.push(*op as u8);
        }
        ToShell::StatusItemSet { item, visible, title, symbol, menu } => {
            u32_(b, tag::STATUS_ITEM_SET);
            u32_(b, *item);
            b.push(*visible as u8);
            put(b, title);
            put(b, symbol);
            let n = menu.len().min(MAX_STATUS_MENU);
            u32_(b, n as u32);
            for i in menu.iter().take(n) {
                u32_(b, i.id);
                b.push(i.enabled as u8);
                b.push(i.checked as u8);
                b.push(i.separator as u8);
                put(b, &i.title);
            }
        }
        ToShell::StatusItemRemove { item } => {
            u32_(b, tag::STATUS_ITEM_REMOVE);
            u32_(b, *item);
        }
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    Ok(match t {
        tag::LOGIN_ITEM => ToShell::LoginItem { request: r.u32()?, op: LoginOp::from_wire(r.u8()?).ok_or(DecodeError::BadValue)? },
        tag::STATUS_ITEM_SET => {
            let item = r.u32()?;
            let visible = flag(r)?;
            let title = get(r)?;
            let symbol = get(r)?;
            let n = r.u32()? as usize;
            if n > MAX_STATUS_MENU {
                return Err(DecodeError::BadValue);
            }
            let mut menu = Vec::new();
            for _ in 0..n {
                let id = r.u32()?;
                let enabled = flag(r)?;
                let checked = flag(r)?;
                let separator = flag(r)?;
                menu.push(StatusMenuItem { id, enabled, checked, separator, title: get(r)? });
            }
            ToShell::StatusItemSet { item, visible, title, symbol, menu }
        }
        tag::STATUS_ITEM_REMOVE => ToShell::StatusItemRemove { item: r.u32()? },
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_status_item_messages_round_trip() {
        for op in [LoginOp::Register, LoginOp::Unregister, LoginOp::Query, LoginOp::OpenSettings] {
            let m = ToShell::LoginItem { request: 3, op };
            assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        }
        for status in [LoginStatus::NotRegistered, LoginStatus::Enabled, LoginStatus::RequiresApproval, LoginStatus::NotFound] {
            let m = ToApp::LoginItemStatus { request: 3, status };
            assert_eq!(ToApp::decode(&m.encode()), Ok(m));
        }
        let set = ToShell::StatusItemSet {
            item: 1,
            visible: true,
            title: "42%".into(),
            symbol: "bolt".into(),
            menu: vec![StatusMenuItem { id: 7, title: "Open ü".into(), enabled: true, checked: true, separator: false }, StatusMenuItem { separator: true, ..Default::default() }],
        };
        assert_eq!(ToShell::decode(&set.encode()), Ok(set));
        let rm = ToShell::StatusItemRemove { item: 1 };
        assert_eq!(ToShell::decode(&rm.encode()), Ok(rm));
        let act = ToApp::StatusItemAction { item: 1, id: 7 };
        assert_eq!(ToApp::decode(&act.encode()), Ok(act));
    }

    #[test]
    fn status_item_limits_are_enforced() {
        let big = ToShell::StatusItemSet {
            item: 1,
            visible: true,
            title: "t".repeat(1000),
            symbol: String::new(),
            menu: (0..100).map(|id| StatusMenuItem { id, title: "x".repeat(1000), enabled: true, ..Default::default() }).collect(),
        };
        let bytes = big.encode();
        assert!(bytes.len() < crate::MAX_PACKET);
        let Ok(ToShell::StatusItemSet { title, menu, .. }) = ToShell::decode(&bytes) else { panic!("decode") };
        assert_eq!(title.chars().count(), MAX_STATUS_TEXT);
        assert_eq!(menu.len(), MAX_STATUS_MENU);
        let mut p = Vec::new();
        u32_(&mut p, tag::LOGIN_ITEM);
        u32_(&mut p, 1);
        p.push(9);
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
    }
}
