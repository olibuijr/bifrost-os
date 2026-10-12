//! Record sharing: a share grants other users access to one record and the zone it lives in.
//!
//! Mirrors the reference share model (share, participant, permission and acceptance types). A `Share` names its root
//! record and an owner. The owner is always a participant with read and write access. Other users are invited with `add_participant`, and they reach the share
//! once they `accept` it. A share also has a public permission, which applies to anyone holding its URL who is not
//! a listed participant. Uninvited users can ask for access when `allows_access_requests` is on. The owner can deny
//! or block those requests.
//!
//! Access rules, decided here because the reference page does not state them in full:
//! - the owner always reads and writes;
//! - an accepted participant has the permission it was given;
//! - a pending participant has no access until it accepts;
//! - a removed participant has no access, and the public permission does not apply to it;
//! - anyone else gets the public permission.
//!
//! Participants are keyed by a user identifier string, the same key the rest of the platform uses for an account.
//! The share does not check a user's identity or send invitations; a driver does that and calls these methods.
//! Pure data: no network, no disk, no URL is opened.
//!
//! Gaps: no per-participant expiry, no invitation email or message, no one-time request URLs, no serialised
//! share record for the store (a share is held in memory by its owner), and no record-level permission inside the
//! zone; the share permission applies to the whole root record.

use std::collections::{BTreeMap, BTreeSet};

use crate::record_store::RecordID;
use crate::record_sync::{ErrorCode, RecordError};

/// What a participant may do with the shared record (participant permission).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    None,
    ReadOnly,
    ReadWrite,
    Unknown,
}

impl Permission {
    pub fn allows_read(self) -> bool {
        matches!(self, Permission::ReadOnly | Permission::ReadWrite)
    }

    pub fn allows_write(self) -> bool {
        self == Permission::ReadWrite
    }
}

/// Where a participant stands in the share (participant acceptance status).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acceptance {
    Unknown,
    Pending,
    Accepted,
    Removed,
}

/// A user's part in a share (participant).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    pub user: String,
    pub permission: Permission,
    pub status: Acceptance,
    pub is_owner: bool,
}

/// A share of one root record. The owner is set when the share is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    root: RecordID,
    share_id: String,
    url: String,
    owner: String,
    participants: BTreeMap<String, Participant>,
    public: Permission,
    max_participants: usize,
    allows_access_requests: bool,
    requesters: BTreeSet<String>,
    blocked: BTreeSet<String>,
}

impl Share {
    /// Creates a share of `root`. `base_url` is the address invitations are sent from; the share URL is that address
    /// with `share_id` appended after one slash. `max_participants` caps the invited users, not the owner.
    /// The public permission starts at `None`, so the URL gives no access until the owner sets it.
    pub fn new(
        root: RecordID,
        share_id: impl Into<String>,
        owner: impl Into<String>,
        base_url: &str,
        max_participants: usize,
    ) -> Result<Share, RecordError> {
        let owner = owner.into();
        let share_id = share_id.into();
        if owner.is_empty() {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "a share needs an owner"));
        }
        if share_id.is_empty() {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "a share needs an identifier"));
        }
        if max_participants == 0 {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "a share must allow at least one participant"));
        }
        let mut participants = BTreeMap::new();
        participants.insert(
            owner.clone(),
            Participant { user: owner.clone(), permission: Permission::ReadWrite, status: Acceptance::Accepted, is_owner: true },
        );
        let url = format!("{}/{}", base_url.trim_end_matches('/'), share_id);
        Ok(Share {
            root,
            share_id,
            url,
            owner,
            participants,
            public: Permission::None,
            max_participants,
            allows_access_requests: false,
            requesters: BTreeSet::new(),
            blocked: BTreeSet::new(),
        })
    }

    pub fn root(&self) -> &RecordID {
        &self.root
    }

    pub fn share_id(&self) -> &str {
        &self.share_id
    }

    /// The address for inviting participants (`url`).
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn public_permission(&self) -> Permission {
        self.public
    }

    /// Sets the permission for anyone with the URL. The owner's own permission cannot change.
    pub fn set_public_permission(&mut self, permission: Permission) -> Result<(), RecordError> {
        check_grantable(permission)?;
        self.public = permission;
        Ok(())
    }

    /// Every participant, the owner first, then the rest in user order. Removed participants stay listed so that
    /// the owner can see who was taken off.
    pub fn participants(&self) -> Vec<&Participant> {
        let mut out: Vec<&Participant> = self.participants.values().filter(|p| p.is_owner).collect();
        out.extend(self.participants.values().filter(|p| !p.is_owner));
        out
    }

    pub fn participant(&self, user: &str) -> Option<&Participant> {
        self.participants.get(user)
    }

    /// Invites a user with `permission`. A user already listed gets the new permission and goes back to pending.
    /// Inviting over the cap fails with `TooManyParticipants`, and inviting a blocked user fails with
    /// `PermissionFailure`.
    pub fn add_participant(&mut self, user: &str, permission: Permission) -> Result<(), RecordError> {
        check_user(user)?;
        check_grantable(permission)?;
        if self.blocked.contains(user) {
            return Err(RecordError::new(ErrorCode::PermissionFailure, format!("{user:?} is blocked from this share")));
        }
        if self.participants.get(user).map_or(false, |p| p.is_owner) {
            return Err(RecordError::new(ErrorCode::AlreadyShared, "the owner is already part of the share"));
        }
        let listed = self.invited_count();
        let already_active = self.participants.get(user).map_or(false, |p| p.status != Acceptance::Removed);
        if !already_active && listed >= self.max_participants {
            return Err(RecordError::new(
                ErrorCode::TooManyParticipants,
                format!("a share takes at most {} participants", self.max_participants),
            ));
        }
        self.requesters.remove(user);
        self.participants.insert(
            user.to_string(),
            Participant { user: user.to_string(), permission, status: Acceptance::Pending, is_owner: false },
        );
        Ok(())
    }

    /// Records that a pending participant accepted the invitation. Only a pending participant can accept.
    pub fn accept(&mut self, user: &str) -> Result<(), RecordError> {
        let p = self.participants.get_mut(user).ok_or_else(|| unknown_user(user))?;
        if p.is_owner || p.status != Acceptance::Pending {
            return Err(RecordError::new(ErrorCode::InvalidArguments, format!("{user:?} has no invitation to accept")));
        }
        p.status = Acceptance::Accepted;
        Ok(())
    }

    /// Changes an invited user's permission. The owner's permission cannot change.
    pub fn set_permission(&mut self, user: &str, permission: Permission) -> Result<(), RecordError> {
        check_grantable(permission)?;
        let p = self.participants.get_mut(user).ok_or_else(|| unknown_user(user))?;
        if p.is_owner {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "the owner's permission cannot change"));
        }
        p.permission = permission;
        Ok(())
    }

    /// Takes a participant off the share. The entry stays with status `Removed`. The owner cannot be removed.
    pub fn remove_participant(&mut self, user: &str) -> Result<(), RecordError> {
        let p = self.participants.get_mut(user).ok_or_else(|| unknown_user(user))?;
        if p.is_owner {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "the owner cannot be removed from the share"));
        }
        p.status = Acceptance::Removed;
        Ok(())
    }

    /// What `user` may do with the root record, under the access rules in the module comment.
    pub fn access_for(&self, user: &str) -> Permission {
        match self.participants.get(user) {
            Some(p) if p.is_owner => Permission::ReadWrite,
            Some(p) => match p.status {
                Acceptance::Accepted => p.permission,
                Acceptance::Pending | Acceptance::Unknown => Permission::None,
                Acceptance::Removed => Permission::None,
            },
            None => self.public,
        }
    }

    pub fn can_read(&self, user: &str) -> bool {
        self.access_for(user).allows_read()
    }

    pub fn can_write(&self, user: &str) -> bool {
        self.access_for(user).allows_write()
    }

    /// Whether uninvited users may ask for access (`allowsAccessRequests`).
    pub fn allows_access_requests(&self) -> bool {
        self.allows_access_requests
    }

    pub fn set_allows_access_requests(&mut self, allowed: bool) {
        self.allows_access_requests = allowed;
    }

    /// An uninvited user asks for access. Needs requests to be allowed. Blocked users, and users already listed in
    /// the share, cannot ask.
    pub fn request_access(&mut self, user: &str) -> Result<(), RecordError> {
        check_user(user)?;
        if !self.allows_access_requests {
            return Err(RecordError::new(ErrorCode::PermissionFailure, "this share takes no access requests"));
        }
        if self.blocked.contains(user) {
            return Err(RecordError::new(ErrorCode::PermissionFailure, format!("{user:?} is blocked from this share")));
        }
        if self.participants.contains_key(user) {
            return Err(RecordError::new(ErrorCode::AlreadyShared, format!("{user:?} is already part of the share")));
        }
        self.requesters.insert(user.to_string());
        Ok(())
    }

    /// The users waiting for an answer to an access request, in user order.
    pub fn requesters(&self) -> Vec<&str> {
        self.requesters.iter().map(|s| s.as_str()).collect()
    }

    /// Denies pending requests from `users`. A denied user can ask again later.
    pub fn deny_requesters(&mut self, users: &[&str]) {
        for user in users {
            self.requesters.remove(*user);
        }
    }

    /// Blocks `users`: their pending requests are dropped, and they cannot ask or be invited until unblocked.
    pub fn block_requesters(&mut self, users: &[&str]) {
        for user in users {
            self.requesters.remove(*user);
            self.blocked.insert(user.to_string());
        }
    }

    /// Lifts the block on `users`. They may ask for access again.
    pub fn unblock_identities(&mut self, users: &[&str]) {
        for user in users {
            self.blocked.remove(*user);
        }
    }

    pub fn blocked_identities(&self) -> Vec<&str> {
        self.blocked.iter().map(|s| s.as_str()).collect()
    }

    /// Invited participants that have not been removed, the owner excluded. The cap applies to this count.
    fn invited_count(&self) -> usize {
        self.participants.values().filter(|p| !p.is_owner && p.status != Acceptance::Removed).count()
    }
}

fn check_user(user: &str) -> Result<(), RecordError> {
    if user.is_empty() {
        return Err(RecordError::new(ErrorCode::InvalidArguments, "a user identifier must not be empty"));
    }
    Ok(())
}

/// A share can grant read-only or read-write access. `None` and `Unknown` are not grants.
fn check_grantable(permission: Permission) -> Result<(), RecordError> {
    match permission {
        Permission::ReadOnly | Permission::ReadWrite | Permission::None => Ok(()),
        Permission::Unknown => {
            Err(RecordError::new(ErrorCode::InvalidArguments, "the permission of a share cannot be unknown"))
        }
    }
}

fn unknown_user(user: &str) -> RecordError {
    RecordError::new(ErrorCode::UnknownItem, format!("{user:?} is not part of the share"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> RecordID {
        RecordID::new("notes", "note-1")
    }

    fn share(max: usize) -> Share {
        Share::new(root(), "abc123", "owner-1", "https://share.example/", max).unwrap()
    }

    #[test]
    fn owner_reads_and_writes_and_url_joins_one_slash() {
        let s = share(5);
        assert_eq!(s.url(), "https://share.example/abc123");
        assert_eq!(s.owner(), "owner-1");
        assert!(s.can_read("owner-1") && s.can_write("owner-1"));
        assert_eq!(s.participants().len(), 1);
    }

    #[test]
    fn invited_user_has_no_access_until_accepted() {
        let mut s = share(5);
        s.add_participant("ann", Permission::ReadOnly).unwrap();
        assert_eq!(s.participant("ann").unwrap().status, Acceptance::Pending);
        assert_eq!(s.access_for("ann"), Permission::None);
        s.accept("ann").unwrap();
        assert!(s.can_read("ann"));
        assert!(!s.can_write("ann"));
        // Accepting twice is refused: there is no invitation left to accept.
        assert_eq!(s.accept("ann").unwrap_err().code(), ErrorCode::InvalidArguments);
    }

    #[test]
    fn participant_cap_counts_invited_users_and_removal_frees_a_seat() {
        let mut s = share(2);
        s.add_participant("a", Permission::ReadOnly).unwrap();
        s.add_participant("b", Permission::ReadWrite).unwrap();
        let err = s.add_participant("c", Permission::ReadOnly).unwrap_err();
        assert_eq!(err.code(), ErrorCode::TooManyParticipants);
        s.remove_participant("a").unwrap();
        assert_eq!(s.participant("a").unwrap().status, Acceptance::Removed);
        s.add_participant("c", Permission::ReadOnly).unwrap();
        assert_eq!(s.participants().len(), 4);
    }

    #[test]
    fn public_permission_applies_to_strangers_but_not_to_removed_participants() {
        let mut s = share(5);
        assert_eq!(s.access_for("stranger"), Permission::None);
        s.set_public_permission(Permission::ReadOnly).unwrap();
        assert!(s.can_read("stranger"));
        assert!(!s.can_write("stranger"));
        s.add_participant("ann", Permission::ReadWrite).unwrap();
        s.accept("ann").unwrap();
        s.remove_participant("ann").unwrap();
        assert_eq!(s.access_for("ann"), Permission::None);
    }

    #[test]
    fn owner_cannot_be_removed_or_demoted() {
        let mut s = share(5);
        assert_eq!(s.remove_participant("owner-1").unwrap_err().code(), ErrorCode::InvalidArguments);
        assert_eq!(s.set_permission("owner-1", Permission::ReadOnly).unwrap_err().code(), ErrorCode::InvalidArguments);
        assert!(s.can_write("owner-1"));
    }

    #[test]
    fn access_requests_are_refused_when_off_and_blocked_users_stay_out() {
        let mut s = share(5);
        assert_eq!(s.request_access("eve").unwrap_err().code(), ErrorCode::PermissionFailure);
        s.set_allows_access_requests(true);
        s.request_access("eve").unwrap();
        s.request_access("bob").unwrap();
        assert_eq!(s.requesters(), vec!["bob", "eve"]);
        s.block_requesters(&["eve"]);
        assert_eq!(s.requesters(), vec!["bob"]);
        assert_eq!(s.request_access("eve").unwrap_err().code(), ErrorCode::PermissionFailure);
        assert_eq!(s.add_participant("eve", Permission::ReadOnly).unwrap_err().code(), ErrorCode::PermissionFailure);
        s.unblock_identities(&["eve"]);
        s.request_access("eve").unwrap();
        assert_eq!(s.requesters(), vec!["bob", "eve"]);
    }

    #[test]
    fn denying_a_request_removes_it_and_listed_users_cannot_request() {
        let mut s = share(5);
        s.set_allows_access_requests(true);
        s.add_participant("ann", Permission::ReadOnly).unwrap();
        assert_eq!(s.request_access("ann").unwrap_err().code(), ErrorCode::AlreadyShared);
        s.request_access("carl").unwrap();
        s.deny_requesters(&["carl"]);
        assert!(s.requesters().is_empty());
        assert!(s.blocked_identities().is_empty());
    }

    #[test]
    fn unknown_permission_and_empty_owner_are_rejected() {
        assert_eq!(Share::new(root(), "x", "", "https://s", 1).unwrap_err().code(), ErrorCode::InvalidArguments);
        assert_eq!(Share::new(root(), "x", "o", "https://s", 0).unwrap_err().code(), ErrorCode::InvalidArguments);
        let mut s = share(5);
        assert_eq!(s.set_public_permission(Permission::Unknown).unwrap_err().code(), ErrorCode::InvalidArguments);
        assert_eq!(s.add_participant("ann", Permission::Unknown).unwrap_err().code(), ErrorCode::InvalidArguments);
        assert_eq!(s.accept("nobody").unwrap_err().code(), ErrorCode::UnknownItem);
    }
}
