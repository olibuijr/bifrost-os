//! User and group ID change events: the record a monitor gets when a process changes its real, effective or saved
//! IDs. The model holds a process's credentials and applies the six ID-setting calls with the permission rules
//! of the Linux credential calls (setuid(2), seteuid(2), setreuid(2) and their group forms). A call that
//! succeeds returns the event it would report; a refused call returns `NotPermitted` and leaves the IDs as they
//! were. Pure model: no shell path calls it yet, and no process is switched by it.
//!
//! Privilege is approximated as "effective ID is 0" for both user and group calls. The Linux capability check
//! (CAP_SETUID, CAP_SETGID) is not modelled.

/// The three IDs a process holds for one kind of identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdSet {
    pub real: u32,
    pub effective: u32,
    pub saved: u32,
}

impl IdSet {
    pub fn new(id: u32) -> Self {
        IdSet { real: id, effective: id, saved: id }
    }
}

/// The user and group IDs of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub uid: IdSet,
    pub gid: IdSet,
}

/// The event for a successful ID change, with the values the call set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityEvent {
    SetUid { uid: u32 },
    SetEuid { euid: u32 },
    SetReuid { ruid: u32, euid: u32 },
    SetGid { gid: u32 },
    SetEgid { egid: u32 },
    SetRegid { rgid: u32, egid: u32 },
}

impl IdentityEvent {
    /// Stable lower-case name, used in logs and in the control socket.
    pub fn name(&self) -> &'static str {
        match self {
            IdentityEvent::SetUid { .. } => "setuid",
            IdentityEvent::SetEuid { .. } => "seteuid",
            IdentityEvent::SetReuid { .. } => "setreuid",
            IdentityEvent::SetGid { .. } => "setgid",
            IdentityEvent::SetEgid { .. } => "setegid",
            IdentityEvent::SetRegid { .. } => "setregid",
        }
    }
}

/// Refusal of an ID change by an unprivileged process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityError {
    NotPermitted,
}

/// setuid rule: a privileged caller sets all three IDs. Any other caller may set only the effective ID, and only
/// to its real or saved ID.
fn set_all(ids: &mut IdSet, privileged: bool, value: u32) -> Result<(), IdentityError> {
    if privileged {
        *ids = IdSet::new(value);
        Ok(())
    } else if value == ids.real || value == ids.saved {
        ids.effective = value;
        Ok(())
    } else {
        Err(IdentityError::NotPermitted)
    }
}

/// seteuid rule: a privileged caller may set any value. Any other caller may set the effective ID to its real,
/// effective or saved ID.
fn set_effective(ids: &mut IdSet, privileged: bool, value: u32) -> Result<(), IdentityError> {
    if privileged || value == ids.real || value == ids.effective || value == ids.saved {
        ids.effective = value;
        Ok(())
    } else {
        Err(IdentityError::NotPermitted)
    }
}

/// setreuid rule. `None` leaves an ID unchanged. An unprivileged caller may set the real ID to its real or
/// effective ID, and the effective ID to its real, effective or saved ID. The saved ID follows the new effective
/// ID when the real ID is set, or when the effective ID is set to a value other than the old real ID.
fn set_real_effective(
    ids: &mut IdSet,
    privileged: bool,
    real: Option<u32>,
    effective: Option<u32>,
) -> Result<(), IdentityError> {
    if !privileged {
        if let Some(r) = real {
            if r != ids.real && r != ids.effective {
                return Err(IdentityError::NotPermitted);
            }
        }
        if let Some(e) = effective {
            if e != ids.real && e != ids.effective && e != ids.saved {
                return Err(IdentityError::NotPermitted);
            }
        }
    }
    let old_real = ids.real;
    let new_effective = effective.unwrap_or(ids.effective);
    if real.is_some() || (effective.is_some() && new_effective != old_real) {
        ids.saved = new_effective;
    }
    ids.real = real.unwrap_or(ids.real);
    ids.effective = new_effective;
    Ok(())
}

impl Credentials {
    pub fn new(uid: u32, gid: u32) -> Self {
        Credentials { uid: IdSet::new(uid), gid: IdSet::new(gid) }
    }

    fn privileged(&self) -> bool {
        self.uid.effective == 0
    }

    pub fn set_uid(&mut self, uid: u32) -> Result<IdentityEvent, IdentityError> {
        let privileged = self.privileged();
        set_all(&mut self.uid, privileged, uid)?;
        Ok(IdentityEvent::SetUid { uid })
    }

    pub fn set_euid(&mut self, euid: u32) -> Result<IdentityEvent, IdentityError> {
        let privileged = self.privileged();
        set_effective(&mut self.uid, privileged, euid)?;
        Ok(IdentityEvent::SetEuid { euid })
    }

    pub fn set_reuid(&mut self, ruid: Option<u32>, euid: Option<u32>) -> Result<IdentityEvent, IdentityError> {
        let privileged = self.privileged();
        set_real_effective(&mut self.uid, privileged, ruid, euid)?;
        Ok(IdentityEvent::SetReuid { ruid: self.uid.real, euid: self.uid.effective })
    }

    pub fn set_gid(&mut self, gid: u32) -> Result<IdentityEvent, IdentityError> {
        let privileged = self.privileged();
        set_all(&mut self.gid, privileged, gid)?;
        Ok(IdentityEvent::SetGid { gid })
    }

    pub fn set_egid(&mut self, egid: u32) -> Result<IdentityEvent, IdentityError> {
        let privileged = self.privileged();
        set_effective(&mut self.gid, privileged, egid)?;
        Ok(IdentityEvent::SetEgid { egid })
    }

    pub fn set_regid(&mut self, rgid: Option<u32>, egid: Option<u32>) -> Result<IdentityEvent, IdentityError> {
        let privileged = self.privileged();
        set_real_effective(&mut self.gid, privileged, rgid, egid)?;
        Ok(IdentityEvent::SetRegid { rgid: self.gid.real, egid: self.gid.effective })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(real: u32, effective: u32, saved: u32) -> IdSet {
        IdSet { real, effective, saved }
    }

    #[test]
    fn unprivileged_setuid_to_root_is_refused_and_changes_nothing() {
        let mut c = Credentials::new(1000, 100);
        assert_eq!(c.set_uid(0), Err(IdentityError::NotPermitted));
        assert_eq!(c, Credentials::new(1000, 100));
    }

    #[test]
    fn root_setuid_sets_all_three_ids() {
        let mut c = Credentials::new(0, 0);
        let event = c.set_uid(1000).unwrap();
        assert_eq!(event, IdentityEvent::SetUid { uid: 1000 });
        assert_eq!(event.name(), "setuid");
        assert_eq!(c.uid, ids(1000, 1000, 1000));
    }

    #[test]
    fn setuid_binary_is_privileged_through_its_effective_id() {
        // Real 1000, effective 0 (set-user-ID root): the caller may drop all three IDs.
        let mut c = Credentials { uid: ids(1000, 0, 0), gid: IdSet::new(100) };
        c.set_uid(1000).unwrap();
        assert_eq!(c.uid, ids(1000, 1000, 1000));
    }

    #[test]
    fn unprivileged_setuid_to_saved_id_changes_only_the_effective_id() {
        let mut c = Credentials { uid: ids(1000, 1001, 1000), gid: IdSet::new(100) };
        c.set_uid(1000).unwrap();
        assert_eq!(c.uid, ids(1000, 1000, 1000));
    }

    #[test]
    fn seteuid_accepts_saved_id_and_refuses_others() {
        let mut c = Credentials { uid: ids(1000, 1000, 1001), gid: IdSet::new(100) };
        assert_eq!(c.set_euid(1001), Ok(IdentityEvent::SetEuid { euid: 1001 }));
        assert_eq!(c.uid.effective, 1001);
        assert_eq!(c.set_euid(5), Err(IdentityError::NotPermitted));
        assert_eq!(c.uid.effective, 1001);
    }

    #[test]
    fn setreuid_moves_saved_id_only_when_the_rule_says_so() {
        let mut c = Credentials { uid: ids(1000, 1000, 1001), gid: IdSet::new(100) };
        // Real ID 1001 is neither the real nor the effective ID: refused.
        assert_eq!(c.set_reuid(Some(1001), None), Err(IdentityError::NotPermitted));
        // Effective set to saved 1001, which differs from the old real 1000: saved follows it.
        assert_eq!(
            c.set_reuid(None, Some(1001)),
            Ok(IdentityEvent::SetReuid { ruid: 1000, euid: 1001 })
        );
        assert_eq!(c.uid, ids(1000, 1001, 1001));
    }

    #[test]
    fn setreuid_keeps_saved_id_when_effective_equals_old_real() {
        let mut c = Credentials { uid: ids(1000, 1001, 1002), gid: IdSet::new(100) };
        c.set_reuid(None, Some(1000)).unwrap();
        assert_eq!(c.uid, ids(1000, 1000, 1002));
    }

    #[test]
    fn setreuid_with_real_set_makes_saved_follow_effective() {
        let mut c = Credentials { uid: ids(1000, 1001, 1002), gid: IdSet::new(100) };
        c.set_reuid(Some(1000), Some(1000)).unwrap();
        assert_eq!(c.uid, ids(1000, 1000, 1000));
    }

    #[test]
    fn group_calls_follow_the_same_rules() {
        let mut user = Credentials::new(1000, 100);
        assert_eq!(user.set_gid(200), Err(IdentityError::NotPermitted));
        assert_eq!(user.set_gid(100), Ok(IdentityEvent::SetGid { gid: 100 }));

        let mut root = Credentials::new(0, 100);
        assert_eq!(
            root.set_regid(Some(300), Some(400)),
            Ok(IdentityEvent::SetRegid { rgid: 300, egid: 400 })
        );
        assert_eq!(root.gid, ids(300, 400, 400));
        assert_eq!(IdentityEvent::SetEgid { egid: 1 }.name(), "setegid");
    }
}
