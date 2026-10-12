//! Access to the photo library for an app: the authorization state, the one-time prompt, and limited access where the
//! person picks which items an app may see. The first request decides; later requests return the state they already
//! have, so an app cannot ask again. A restricted library refuses every request. In limited access, the app sees only
//! the item identifiers the person chose. Pure: the shell shows the prompt and passes the answer in.

use std::collections::BTreeSet;

/// The access state of one app for the photo library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The app has not asked yet.
    NotDetermined,
    /// Access is blocked by policy; the person cannot grant it.
    Restricted,
    /// The person refused access.
    Denied,
    /// The app sees the whole library.
    Authorized,
    /// The app sees only the items the person chose.
    Limited,
}

/// The person's answer to the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Allow,
    /// Allow only these item identifiers.
    AllowLimited(Vec<String>),
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The library is restricted by policy, so the prompt cannot be answered.
    Restricted,
    /// The selection can change only in limited access.
    NotLimited(Status),
}

/// The access of one app, with the items it may see in limited access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    status: Status,
    chosen: BTreeSet<String>,
}

impl Access {
    /// Starts from a known state. Use `NotDetermined` for an app that has never asked.
    pub fn new(status: Status) -> Access {
        Access { status, chosen: BTreeSet::new() }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// The identifiers the app may see in limited access, in order.
    pub fn chosen(&self) -> Vec<String> {
        self.chosen.iter().cloned().collect()
    }

    /// Answers the prompt. Only a first request changes the state: when the state is already decided, the current
    /// state comes back and nothing changes. A restricted library refuses the request.
    pub fn request(&mut self, answer: Answer) -> Result<Status, Error> {
        if self.status == Status::Restricted {
            return Err(Error::Restricted);
        }
        if self.status != Status::NotDetermined {
            return Ok(self.status);
        }
        match answer {
            Answer::Allow => self.status = Status::Authorized,
            Answer::AllowLimited(ids) => {
                self.chosen = ids.into_iter().collect();
                self.status = Status::Limited;
            }
            Answer::Deny => self.status = Status::Denied,
        }
        Ok(self.status)
    }

    /// Replaces the chosen items. Only limited access has a selection to change.
    pub fn select(&mut self, ids: &[String]) -> Result<(), Error> {
        if self.status != Status::Limited {
            return Err(Error::NotLimited(self.status));
        }
        self.chosen = ids.iter().cloned().collect();
        Ok(())
    }

    /// True when the app may see the item with this identifier.
    pub fn allows(&self, id: &str) -> bool {
        match self.status {
            Status::Authorized => true,
            Status::Limited => self.chosen.contains(id),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn first_allow_grants_the_whole_library() {
        let mut access = Access::new(Status::NotDetermined);
        assert_eq!(access.request(Answer::Allow), Ok(Status::Authorized));
        assert!(access.allows("any-item"));
    }

    #[test]
    fn a_later_request_keeps_the_first_answer() {
        let mut access = Access::new(Status::NotDetermined);
        access.request(Answer::Deny).unwrap();
        // The prompt is not shown again: asking to allow now returns the denial and changes nothing.
        assert_eq!(access.request(Answer::Allow), Ok(Status::Denied));
        assert!(!access.allows("any-item"));
    }

    #[test]
    fn limited_access_shows_only_the_chosen_items() {
        let mut access = Access::new(Status::NotDetermined);
        let status = access.request(Answer::AllowLimited(ids(&["p2", "p1", "p2"]))).unwrap();
        assert_eq!(status, Status::Limited);
        // Duplicates collapse and the list comes back ordered.
        assert_eq!(access.chosen(), ids(&["p1", "p2"]));
        assert!(access.allows("p1"));
        assert!(!access.allows("p3"));
    }

    #[test]
    fn selection_changes_only_in_limited_access() {
        let mut limited = Access::new(Status::Limited);
        limited.select(&ids(&["p9"])).unwrap();
        assert_eq!(limited.chosen(), ids(&["p9"]));
        assert!(limited.allows("p9"));
        let mut authorized = Access::new(Status::Authorized);
        assert_eq!(authorized.select(&ids(&["p9"])), Err(Error::NotLimited(Status::Authorized)));
    }

    #[test]
    fn restricted_library_refuses_the_request() {
        let mut access = Access::new(Status::Restricted);
        assert_eq!(access.request(Answer::Allow), Err(Error::Restricted));
        assert_eq!(access.status(), Status::Restricted);
        assert!(!access.allows("any-item"));
    }

    #[test]
    fn denied_and_undecided_apps_see_nothing() {
        assert!(!Access::new(Status::NotDetermined).allows("p1"));
        assert!(!Access::new(Status::Denied).allows("p1"));
        assert_eq!(Access::new(Status::Denied).chosen(), Vec::<String>::new());
    }
}
