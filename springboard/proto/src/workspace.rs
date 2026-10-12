//! Wire types for the workspace (`bifrost_app::workspace`): opening files and links with the default app, revealing and
//! recycling files, asking about apps, and hearing about launches, terminations, activations and sleep.
//! Tags: `ToApp` 100 and 101, `ToShell` 0x230 and 0x231.
//!
//! Files are named by document grants the app already holds (`GrantId`), never by path: the shell resolves the grant
//! and refuses a grant the app does not own. Every list and string is capped on encode and refused on decode.

use super::{str_, u32_, DecodeError, Reader, ToApp, ToShell};

pub const MAX_GRANTS: usize = 16;
pub const MAX_ARG: usize = 1024;
pub const MAX_ITEMS: usize = 32;
pub const MAX_ITEM: usize = 96;

/// What a `ToShell::WorkspaceRequest` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceOp {
    /// open each granted file with its default app
    OpenFile,
    /// open `arg` (a web URL or a scheme some installed app declares) with its default app
    OpenUrl,
    /// open each granted file with the installed app `arg`
    OpenWith,
    /// show each granted file in Files
    Reveal,
    /// move each granted file to Recently Deleted; needs a write grant
    Recycle,
    /// whether app `arg` is installed: its id in the items, or none
    UrlForApp,
    /// the id of the frontmost app
    Frontmost,
    /// the ids of the running apps
    Running,
}

impl WorkspaceOp {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::OpenFile,
            1 => Self::OpenUrl,
            2 => Self::OpenWith,
            3 => Self::Reveal,
            4 => Self::Recycle,
            5 => Self::UrlForApp,
            6 => Self::Frontmost,
            7 => Self::Running,
            _ => return None,
        })
    }
}

/// The outcome of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceStatus {
    Ok,
    /// the app holds no grant for a file, or lacks write access, or the target is not allowed
    Denied,
    /// no such app, or nothing handles the type or scheme
    NotFound,
    Unsupported,
    Failed,
}

impl WorkspaceStatus {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Ok,
            1 => Self::Denied,
            2 => Self::NotFound,
            3 => Self::Unsupported,
            4 => Self::Failed,
            _ => return None,
        })
    }
}

/// What `ToApp::WorkspaceEvent` reports (the workspace notification center).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceEventKind {
    DidLaunchApplication,
    DidTerminateApplication,
    DidActivateApplication,
    WillSleep,
    DidWake,
    ScreensDidSleep,
    ScreensDidWake,
}

impl WorkspaceEventKind {
    pub const ALL: [WorkspaceEventKind; 7] = [
        Self::DidLaunchApplication,
        Self::DidTerminateApplication,
        Self::DidActivateApplication,
        Self::WillSleep,
        Self::DidWake,
        Self::ScreensDidSleep,
        Self::ScreensDidWake,
    ];
    pub fn from_wire(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    /// The bit of this kind in a `WorkspaceSubscribe` mask.
    pub fn bit(self) -> u32 {
        1 << (self as u32)
    }
    /// True for the three application events, which carry an app id.
    pub fn is_application(self) -> bool {
        matches!(self, Self::DidLaunchApplication | Self::DidTerminateApplication | Self::DidActivateApplication)
    }
}

/// All kinds set.
pub const ALL_EVENTS: u32 = 0x7f;

fn put(b: &mut Vec<u8>, s: &str, max: usize) {
    let cut: String = s.chars().take(max).collect();
    str_(b, &cut);
}

fn get(r: &mut Reader, max: usize) -> Result<String, DecodeError> {
    let s = r.str()?;
    if s.chars().count() > max {
        return Err(DecodeError::BadValue);
    }
    Ok(s)
}

pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    match m {
        ToApp::WorkspaceReply { request, status, items } => {
            u32_(b, tag::WORKSPACE_REPLY);
            u32_(b, *request);
            b.push(*status as u8);
            let n = items.len().min(MAX_ITEMS);
            u32_(b, n as u32);
            for i in &items[..n] {
                put(b, i, MAX_ITEM);
            }
        }
        ToApp::WorkspaceEvent { kind, app_id, name } => {
            u32_(b, tag::WORKSPACE_EVENT);
            b.push(*kind as u8);
            put(b, app_id, MAX_ITEM);
            put(b, name, MAX_ITEM);
        }
        _ => {}
    }
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    Ok(match t {
        tag::WORKSPACE_REPLY => {
            let request = r.u32()?;
            let status = WorkspaceStatus::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
            let n = r.u32()? as usize;
            if n > MAX_ITEMS {
                return Err(DecodeError::BadValue);
            }
            let items = (0..n).map(|_| get(r, MAX_ITEM)).collect::<Result<Vec<_>, _>>()?;
            ToApp::WorkspaceReply { request, status, items }
        }
        tag::WORKSPACE_EVENT => {
            let kind = WorkspaceEventKind::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
            let app_id = get(r, MAX_ITEM)?;
            let name = get(r, MAX_ITEM)?;
            ToApp::WorkspaceEvent { kind, app_id, name }
        }
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    match m {
        ToShell::WorkspaceRequest { request, op, grants, arg } => {
            u32_(b, tag::WORKSPACE_REQUEST);
            u32_(b, *request);
            b.push(*op as u8);
            let n = grants.len().min(MAX_GRANTS);
            u32_(b, n as u32);
            for g in &grants[..n] {
                b.extend_from_slice(g);
            }
            put(b, arg, MAX_ARG);
        }
        ToShell::WorkspaceSubscribe { mask } => {
            u32_(b, tag::WORKSPACE_SUBSCRIBE);
            u32_(b, *mask & ALL_EVENTS);
        }
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    Ok(match t {
        tag::WORKSPACE_REQUEST => {
            let request = r.u32()?;
            let op = WorkspaceOp::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
            let n = r.u32()? as usize;
            if n > MAX_GRANTS {
                return Err(DecodeError::BadValue);
            }
            let grants = (0..n).map(|_| r.grant()).collect::<Result<Vec<_>, _>>()?;
            let arg = get(r, MAX_ARG)?;
            ToShell::WorkspaceRequest { request, op, grants, arg }
        }
        tag::WORKSPACE_SUBSCRIBE => ToShell::WorkspaceSubscribe { mask: r.u32()? & ALL_EVENTS },
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

pub(crate) mod tag {
    pub const WORKSPACE_REPLY: u32 = 100;
    pub const WORKSPACE_EVENT: u32 = 101;
    pub const WORKSPACE_REQUEST: u32 = 0x230;
    pub const WORKSPACE_SUBSCRIBE: u32 = 0x231;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_replies_round_trip() {
        let g: GrantId = [3; 16];
        let m = ToShell::WorkspaceRequest { request: 9, op: WorkspaceOp::Recycle, grants: vec![g, [4; 16]], arg: "x y".into() };
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        let s = ToShell::WorkspaceSubscribe { mask: WorkspaceEventKind::DidWake.bit() };
        assert_eq!(ToShell::decode(&s.encode()), Ok(s));
        let r = ToApp::WorkspaceReply { request: 9, status: WorkspaceStatus::Denied, items: vec!["a".into(), "b".into()] };
        assert_eq!(ToApp::decode(&r.encode()), Ok(r));
        let e = ToApp::WorkspaceEvent { kind: WorkspaceEventKind::DidTerminateApplication, app_id: "is.test".into(), name: "Test".into() };
        assert_eq!(ToApp::decode(&e.encode()), Ok(e));
    }

    #[test]
    fn too_many_grants_and_unknown_values_are_refused() {
        let mut p = Vec::new();
        u32_(&mut p, tag::WORKSPACE_REQUEST);
        u32_(&mut p, 1);
        p.push(0);
        u32_(&mut p, MAX_GRANTS as u32 + 1);
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
        let mut q = Vec::new();
        u32_(&mut q, tag::WORKSPACE_REQUEST);
        u32_(&mut q, 1);
        q.push(200);
        assert_eq!(ToShell::decode(&q), Err(DecodeError::BadValue));
    }

    #[test]
    fn the_mask_only_carries_known_events() {
        assert_eq!(ALL_EVENTS, (1 << WorkspaceEventKind::ALL.len()) - 1);
        let s = ToShell::WorkspaceSubscribe { mask: 0xffff_ffff };
        assert_eq!(ToShell::decode(&s.encode()), Ok(ToShell::WorkspaceSubscribe { mask: ALL_EVENTS }));
    }
}
