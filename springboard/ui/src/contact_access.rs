//! Access to the person's contacts for one app: the answer the person gave to the access prompt, and which contacts
//! the app may see. A `Picked` grant holds only the contacts the person chose in the access picker, and it is
//! read-only. `Restricted` is set by the system (for example by parental controls) and no answer here changes it.
//! Pure state: the shell shows the prompt, calls `request` with the answer, and asks `can_read` before it returns
//! any record from the store.

use std::collections::BTreeSet;

/// What the person has let one app do with contacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The person has not answered the prompt yet.
    NotDetermined,
    /// The system blocks access; the person cannot change it here.
    Restricted,
    /// The person refused access.
    Denied,
    /// Every contact, read and write.
    Authorized,
    /// Only the contacts the person picked, read-only.
    Limited,
}

impl Status {
    /// The wire name: `not_determined`, `restricted`, `denied`, `authorized` or `limited`.
    pub fn name(self) -> &'static str {
        match self {
            Status::NotDetermined => "not_determined",
            Status::Restricted => "restricted",
            Status::Denied => "denied",
            Status::Authorized => "authorized",
            Status::Limited => "limited",
        }
    }

    /// The status a wire name stands for, or `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Status> {
        [Status::NotDetermined, Status::Restricted, Status::Denied, Status::Authorized, Status::Limited]
            .into_iter()
            .find(|status| status.name() == name)
    }
}

/// The person's answer, given at the prompt or later in Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Every contact.
    All,
    /// Only these contact identifiers, chosen in the access picker.
    Picked(BTreeSet<u64>),
    /// No access.
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The system restricts the app, so no answer applies.
    Restricted,
    /// The prompt was answered already; later changes go through `change_in_settings`.
    AlreadyDecided,
    /// A pick with no contacts in it. The state is left as it was.
    NothingPicked,
    /// A limited grant reads only the picked contacts and cannot write.
    ReadOnly,
    /// The app has no write access.
    NotAuthorized,
}

/// The access state of one app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    status: Status,
    granted: BTreeSet<u64>,
}

impl Default for Access {
    fn default() -> Self {
        Self::new()
    }
}

impl Access {
    /// A new app that has not been asked yet.
    pub fn new() -> Self {
        Access { status: Status::NotDetermined, granted: BTreeSet::new() }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// The contacts a limited grant holds; empty for every other status.
    pub fn picked(&self) -> &BTreeSet<u64> {
        &self.granted
    }

    /// Answers the prompt. It is shown once: after any answer, or while the system restricts the app, this is refused.
    pub fn request(&mut self, choice: Choice) -> Result<Status, Error> {
        match self.status {
            Status::Restricted => Err(Error::Restricted),
            Status::NotDetermined => self.apply(choice),
            _ => Err(Error::AlreadyDecided),
        }
    }

    /// A change the person makes in Settings. Only a system restriction blocks it.
    pub fn change_in_settings(&mut self, choice: Choice) -> Result<Status, Error> {
        if self.status == Status::Restricted {
            return Err(Error::Restricted);
        }
        self.apply(choice)
    }

    fn apply(&mut self, choice: Choice) -> Result<Status, Error> {
        match choice {
            Choice::All => {
                self.status = Status::Authorized;
                self.granted.clear();
            }
            Choice::Deny => {
                self.status = Status::Denied;
                self.granted.clear();
            }
            Choice::Picked(ids) => {
                if ids.is_empty() {
                    return Err(Error::NothingPicked);
                }
                self.status = Status::Limited;
                self.granted = ids;
            }
        }
        Ok(self.status)
    }

    /// Whether the app may read the contact with this identifier.
    pub fn can_read(&self, id: u64) -> bool {
        match self.status {
            Status::Authorized => true,
            Status::Limited => self.granted.contains(&id),
            _ => false,
        }
    }

    /// Whether the app may change contacts. Only full access writes; a limited grant is read-only.
    pub fn check_write(&self) -> Result<(), Error> {
        match self.status {
            Status::Authorized => Ok(()),
            Status::Limited => Err(Error::ReadOnly),
            _ => Err(Error::NotAuthorized),
        }
    }

    /// The identifiers the app may see, in the order given.
    pub fn visible(&self, ids: &[u64]) -> Vec<u64> {
        ids.iter().copied().filter(|id| self.can_read(*id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picked(ids: &[u64]) -> Choice {
        Choice::Picked(ids.iter().copied().collect())
    }

    #[test]
    fn a_new_app_has_no_access_until_it_is_asked() {
        let a = Access::new();
        assert_eq!(a.status(), Status::NotDetermined);
        assert!(!a.can_read(1));
        assert_eq!(a.check_write(), Err(Error::NotAuthorized));
    }

    #[test]
    fn full_access_reads_and_writes_every_contact() {
        let mut a = Access::new();
        assert_eq!(a.request(Choice::All), Ok(Status::Authorized));
        assert!(a.can_read(42));
        assert_eq!(a.check_write(), Ok(()));
        assert_eq!(a.visible(&[3, 1, 2]), vec![3, 1, 2]);
    }

    #[test]
    fn the_prompt_is_shown_once() {
        let mut a = Access::new();
        assert_eq!(a.request(Choice::Deny), Ok(Status::Denied));
        assert_eq!(a.request(Choice::All), Err(Error::AlreadyDecided));
        assert_eq!(a.status(), Status::Denied);
        assert!(!a.can_read(1));
    }

    #[test]
    fn a_picked_grant_is_read_only_and_limited_to_the_picks() {
        let mut a = Access::new();
        assert_eq!(a.request(picked(&[2, 5])), Ok(Status::Limited));
        assert!(a.can_read(2) && a.can_read(5));
        assert!(!a.can_read(3));
        assert_eq!(a.visible(&[1, 2, 3, 5]), vec![2, 5]);
        assert_eq!(a.check_write(), Err(Error::ReadOnly));
    }

    #[test]
    fn an_empty_pick_changes_nothing() {
        let mut a = Access::new();
        assert_eq!(a.request(picked(&[])), Err(Error::NothingPicked));
        assert_eq!(a.status(), Status::NotDetermined);
        assert!(a.picked().is_empty());
    }

    #[test]
    fn a_system_restriction_blocks_every_answer() {
        let mut a = Access { status: Status::Restricted, granted: BTreeSet::new() };
        assert_eq!(a.request(Choice::All), Err(Error::Restricted));
        assert_eq!(a.change_in_settings(Choice::All), Err(Error::Restricted));
        assert!(!a.can_read(1));
    }

    #[test]
    fn status_wire_names_read_back() {
        for status in [
            Status::NotDetermined,
            Status::Restricted,
            Status::Denied,
            Status::Authorized,
            Status::Limited,
        ] {
            assert_eq!(Status::from_name(status.name()), Some(status));
        }
        assert_eq!(Status::from_name("all"), None);
    }

    #[test]
    fn settings_can_change_an_answer_and_drop_the_old_picks() {
        let mut a = Access::new();
        a.request(picked(&[7])).unwrap();
        assert_eq!(a.change_in_settings(Choice::Deny), Ok(Status::Denied));
        assert!(a.picked().is_empty());
        assert_eq!(a.change_in_settings(Choice::All), Ok(Status::Authorized));
        assert!(a.can_read(99));
    }
}
