//! Access to calendar events and reminders for one app: the answer the person gave to each access prompt, and what
//! that answer allows. Events and reminders are separate entity types with their own answers. Events have a write-only
//! grant (add events, never read them) and a full grant (read and write); reminders have only the full grant. Once
//! an entity type has an answer, or the system restricts it, later requests show the current status and no prompt.
//! Pure state: the shell shows the prompt and calls `request_*` with the answer.

/// The access status of the app for one entity type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// The person has not answered the prompt yet.
    NotDetermined,
    /// The system blocks access; no answer here changes it.
    Restricted,
    /// The person refused access.
    Denied,
    /// The app may add events but not read them. Events only.
    WriteOnly,
    /// The app may read and write.
    FullAccess,
}

/// The two kinds of data the store holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityType {
    Event,
    Reminder,
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The reminders grant is full access only; write-only applies to events.
    NotOfferedForReminders,
}

/// The access state of one app: one status for events and one for reminders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    events: Status,
    reminders: Status,
}

impl Default for Access {
    fn default() -> Self {
        Self::new()
    }
}

impl Access {
    /// A new app that has not been asked about either entity type.
    pub fn new() -> Self {
        Access { events: Status::NotDetermined, reminders: Status::NotDetermined }
    }

    /// The status for one entity type.
    pub fn status(&self, entity: EntityType) -> Status {
        match entity {
            EntityType::Event => self.events,
            EntityType::Reminder => self.reminders,
        }
    }

    /// Answers the write-only prompt for events. `granted` is the person's answer. Any earlier answer, or a system
    /// restriction, is kept and the current status is returned.
    pub fn request_write_only_events(&mut self, granted: bool) -> Status {
        self.decide(EntityType::Event, granted, Status::WriteOnly)
    }

    /// Answers the full-access prompt for events. The same rule as the write-only prompt applies.
    pub fn request_full_events(&mut self, granted: bool) -> Status {
        self.decide(EntityType::Event, granted, Status::FullAccess)
    }

    /// Answers the full-access prompt for reminders. The same rule as the events prompt applies.
    pub fn request_full_reminders(&mut self, granted: bool) -> Status {
        self.decide(EntityType::Reminder, granted, Status::FullAccess)
    }

    /// Answers a request for the given entity type and grant. Write-only is not offered for reminders.
    pub fn request(&mut self, entity: EntityType, write_only: bool, granted: bool) -> Result<Status, Error> {
        match (entity, write_only) {
            (EntityType::Reminder, true) => Err(Error::NotOfferedForReminders),
            (EntityType::Event, true) => Ok(self.request_write_only_events(granted)),
            (EntityType::Event, false) => Ok(self.request_full_events(granted)),
            (EntityType::Reminder, false) => Ok(self.request_full_reminders(granted)),
        }
    }

    fn decide(&mut self, entity: EntityType, granted: bool, grant: Status) -> Status {
        let current = self.status(entity);
        if current != Status::NotDetermined {
            return current;
        }
        let next = if granted { grant } else { Status::Denied };
        match entity {
            EntityType::Event => self.events = next,
            EntityType::Reminder => self.reminders = next,
        }
        next
    }

    /// Whether the app may read entities of this type. Only full access reads, so a write-only grant does not.
    pub fn can_read(&self, entity: EntityType) -> bool {
        self.status(entity) == Status::FullAccess
    }

    /// Whether the app may add or change entities of this type.
    pub fn can_write(&self, entity: EntityType) -> bool {
        matches!(self.status(entity), Status::WriteOnly | Status::FullAccess)
    }

    /// A change the person makes in Settings for events: on is full access, off is denied. Unlike a prompt answer it
    /// replaces an earlier answer; only a system restriction blocks it. Returns the status that stands.
    pub fn change_events_in_settings(&mut self, allowed: bool) -> Status {
        if self.events != Status::Restricted {
            self.events = if allowed { Status::FullAccess } else { Status::Denied };
        }
        self.events
    }

    /// Sets the system restriction for one entity type. The restriction wins over every answer given before it.
    pub fn restrict(&mut self, entity: EntityType) {
        match entity {
            EntityType::Event => self.events = Status::Restricted,
            EntityType::Reminder => self.reminders = Status::Restricted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_app_can_neither_read_nor_write_until_it_is_asked() {
        let a = Access::new();
        assert_eq!(a.status(EntityType::Event), Status::NotDetermined);
        assert!(!a.can_read(EntityType::Event));
        assert!(!a.can_write(EntityType::Reminder));
    }

    #[test]
    fn a_write_only_grant_adds_events_but_cannot_read_them() {
        let mut a = Access::new();
        assert_eq!(a.request_write_only_events(true), Status::WriteOnly);
        assert!(a.can_write(EntityType::Event));
        assert!(!a.can_read(EntityType::Event));
        assert!(!a.can_write(EntityType::Reminder));
    }

    #[test]
    fn full_access_to_events_reads_and_writes_events_only() {
        let mut a = Access::new();
        assert_eq!(a.request_full_events(true), Status::FullAccess);
        assert!(a.can_read(EntityType::Event));
        assert!(a.can_write(EntityType::Event));
        assert_eq!(a.status(EntityType::Reminder), Status::NotDetermined);
    }

    #[test]
    fn a_refusal_is_denied_and_the_prompt_is_not_shown_again() {
        let mut a = Access::new();
        assert_eq!(a.request_full_reminders(false), Status::Denied);
        assert_eq!(a.request_full_reminders(true), Status::Denied);
        assert!(!a.can_read(EntityType::Reminder));
        assert!(!a.can_write(EntityType::Reminder));
    }

    #[test]
    fn write_only_is_refused_for_reminders() {
        let mut a = Access::new();
        assert_eq!(a.request(EntityType::Reminder, true, true), Err(Error::NotOfferedForReminders));
        assert_eq!(a.status(EntityType::Reminder), Status::NotDetermined);
        assert_eq!(a.request(EntityType::Event, true, true), Ok(Status::WriteOnly));
    }

    #[test]
    fn settings_replace_an_earlier_answer_but_not_a_restriction() {
        let mut a = Access::new();
        a.request_write_only_events(false);
        assert_eq!(a.change_events_in_settings(true), Status::FullAccess);
        assert_eq!(a.change_events_in_settings(false), Status::Denied);
        a.restrict(EntityType::Event);
        assert_eq!(a.change_events_in_settings(true), Status::Restricted);
    }

    #[test]
    fn a_system_restriction_wins_over_an_earlier_answer() {
        let mut a = Access::new();
        a.request_full_events(true);
        a.restrict(EntityType::Event);
        assert_eq!(a.status(EntityType::Event), Status::Restricted);
        assert!(!a.can_read(EntityType::Event));
        assert!(!a.can_write(EntityType::Event));
        assert_eq!(a.request_full_events(true), Status::Restricted);
    }
}
