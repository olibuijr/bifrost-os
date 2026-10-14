//! Wire types for Guided Access (single-app mode) and its restrictions. Tags:
//! `ToShell::GuidedAccessRequest` `0x170` (an app asks to start or end a session, with its restrictions),
//! `ToApp::GuidedAccessState` `0x171` (the shell's answer: a session is or is not running), and
//! `ToApp::GuidedRestriction` `0x172` (the user changed one restriction's state in the session options).
//!
//! Layouts: the request is `u32 tag`, `u8 on`, `u32 count` (at most `MAX_RESTRICTIONS`), then per restriction a string id
//! (at most `MAX_ID` bytes), title and detail (at most `MAX_TEXT` each) and `u8 allowed`. The state is `u32 tag`, `u8 active`.
//! The change is `u32 tag`, string id, `u8 allowed`. Decoding refuses a count or a string over its cap and any flag that is
//! not 0 or 1.

use super::axtext::{cut, get};
use super::{str_, u32_, DecodeError, Reader, ToApp, ToShell};

pub const TAG_REQUEST: u32 = 0x170;
pub const TAG_STATE: u32 = 0x171;
pub const TAG_RESTRICTION: u32 = 0x172;
/// Most restrictions one session declares.
pub const MAX_RESTRICTIONS: usize = 16;
/// Longest restriction identifier, in bytes.
pub const MAX_ID: usize = 64;
/// Longest title or detail of a restriction, in bytes.
pub const MAX_TEXT: usize = 160;

/// One app-defined guided-access restriction: an identifier the app knows it by, the title and
/// description the session options show, and whether the restriction is allowed (`true`) or denied (`false`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidedRestriction {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub allowed: bool,
}

impl GuidedRestriction {
    /// A restriction that is allowed, with no detail text.
    pub fn new(id: &str, title: &str) -> Self {
        GuidedRestriction { id: id.into(), title: title.into(), detail: String::new(), allowed: true }
    }

    /// The same restriction with its description text.
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = detail.into();
        self
    }

    /// The same restriction in the given state.
    pub fn set_allowed(mut self, allowed: bool) -> Self {
        self.allowed = allowed;
        self
    }
}

fn flag(r: &mut Reader) -> Result<bool, DecodeError> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::BadValue),
    }
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    if let ToShell::GuidedAccessRequest { on, restrictions } = m {
        u32_(b, TAG_REQUEST);
        b.push(*on as u8);
        let n = restrictions.len().min(MAX_RESTRICTIONS);
        u32_(b, n as u32);
        for g in &restrictions[..n] {
            str_(b, cut(&g.id, MAX_ID));
            str_(b, cut(&g.title, MAX_TEXT));
            str_(b, cut(&g.detail, MAX_TEXT));
            b.push(g.allowed as u8);
        }
    }
}

pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    match m {
        ToApp::GuidedAccessState { active } => {
            u32_(b, TAG_STATE);
            b.push(*active as u8);
        }
        ToApp::GuidedRestriction { id, allowed } => {
            u32_(b, TAG_RESTRICTION);
            str_(b, cut(id, MAX_ID));
            b.push(*allowed as u8);
        }
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    if t != TAG_REQUEST {
        return Err(DecodeError::UnknownTag(t));
    }
    let on = flag(r)?;
    let n = r.u32()? as usize;
    if n > MAX_RESTRICTIONS {
        return Err(DecodeError::BadValue);
    }
    let mut restrictions = Vec::with_capacity(n);
    for _ in 0..n {
        let id = get(r, MAX_ID)?;
        let title = get(r, MAX_TEXT)?;
        let detail = get(r, MAX_TEXT)?;
        let allowed = flag(r)?;
        restrictions.push(GuidedRestriction { id, title, detail, allowed });
    }
    Ok(ToShell::GuidedAccessRequest { on, restrictions })
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    Ok(match t {
        TAG_STATE => ToApp::GuidedAccessState { active: flag(r)? },
        TAG_RESTRICTION => {
            let id = get(r, MAX_ID)?;
            ToApp::GuidedRestriction { id, allowed: flag(r)? }
        }
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_request_round_trips() {
        let m = ToShell::GuidedAccessRequest {
            on: true,
            restrictions: vec![GuidedRestriction::new("touch", "Touch").with_detail("Lock the touch panel"), GuidedRestriction::new("sleep", "Sleep").set_allowed(false)],
        };
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        let end = ToShell::GuidedAccessRequest { on: false, restrictions: Vec::new() };
        assert_eq!(ToShell::decode(&end.encode()), Ok(end));
    }

    #[test]
    fn the_shell_answers_round_trip() {
        let state = ToApp::GuidedAccessState { active: true };
        assert_eq!(ToApp::decode(&state.encode()), Ok(state));
        let change = ToApp::GuidedRestriction { id: "sleep".into(), allowed: false };
        assert_eq!(ToApp::decode(&change.encode()), Ok(change));
    }

    #[test]
    fn caps_and_flags_are_enforced_on_decode() {
        let many = ToShell::GuidedAccessRequest { on: true, restrictions: vec![GuidedRestriction::new("a", "A"); MAX_RESTRICTIONS + 1] };
        let ToShell::GuidedAccessRequest { restrictions, .. } = ToShell::decode(&many.encode()).unwrap() else { panic!() };
        assert_eq!(restrictions.len(), MAX_RESTRICTIONS, "encoding cuts the list to the cap");

        let mut bad = ToShell::GuidedAccessRequest { on: true, restrictions: Vec::new() }.encode();
        bad[4] = 9;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue), "a flag other than 0 or 1 is refused");

        let mut big = ToShell::GuidedAccessRequest { on: true, restrictions: vec![GuidedRestriction::new("a", "A")] }.encode();
        big[5..9].copy_from_slice(&((MAX_RESTRICTIONS as u32) + 1).to_le_bytes());
        assert_eq!(ToShell::decode(&big), Err(DecodeError::BadValue));
    }
}
