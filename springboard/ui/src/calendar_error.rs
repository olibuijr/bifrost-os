//! Error codes for calendar and reminder store operations: the set of reasons a save, a delete or a store request can
//! fail, each with a category the caller can act on. The codes follow the official error code list in name and
//! meaning. The internal `last` marker is not a code and is left out.

use std::fmt;

/// Why a calendar or reminder operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    EventNotMutable,
    NoCalendar,
    NoEndDate,
    NoStartDate,
    DatesInverted,
    InternalFailure,
    CalendarReadOnly,
    DurationGreaterThanRecurrence,
    AlarmGreaterThanRecurrence,
    StartDateTooFarInFuture,
    StartDateCollidesWithOtherOccurrence,
    ObjectBelongsToDifferentStore,
    InvitesCannotBeMoved,
    InvalidSpan,
    CalendarHasNoSource,
    CalendarSourceCannotBeModified,
    CalendarIsImmutable,
    SourceDoesNotAllowCalendarAddDelete,
    RecurringReminderRequiresDueDate,
    StructuredLocationsNotSupported,
    ReminderLocationsNotSupported,
    AlarmProximityNotSupported,
    CalendarDoesNotAllowEvents,
    CalendarDoesNotAllowReminders,
    SourceDoesNotAllowReminders,
    SourceDoesNotAllowEvents,
    PriorityIsInvalid,
    InvalidEntityType,
    ProcedureAlarmsNotMutable,
    EventStoreNotAuthorized,
    OsNotSupported,
    InvalidInviteReplyCalendar,
    NotificationsCollectionFlagNotSet,
    SourceMismatch,
    NotificationCollectionMismatch,
    NotificationSavedWithoutCollection,
}

/// The broad kind of a failure, which tells the caller what to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// The user or the source has not granted the access. Ask again or explain.
    Access,
    /// The value given is not allowed. Fix the input and retry.
    Validation,
    /// The object or calendar cannot be changed. Save a copy somewhere else instead.
    ReadOnly,
    /// The source or the platform does not support the feature.
    Unsupported,
    /// Something inside the store failed. Report it.
    Internal,
}

impl ErrorCode {
    /// Every code, in the order of the official list.
    pub const ALL: [ErrorCode; 36] = [
        ErrorCode::EventNotMutable,
        ErrorCode::NoCalendar,
        ErrorCode::NoEndDate,
        ErrorCode::NoStartDate,
        ErrorCode::DatesInverted,
        ErrorCode::InternalFailure,
        ErrorCode::CalendarReadOnly,
        ErrorCode::DurationGreaterThanRecurrence,
        ErrorCode::AlarmGreaterThanRecurrence,
        ErrorCode::StartDateTooFarInFuture,
        ErrorCode::StartDateCollidesWithOtherOccurrence,
        ErrorCode::ObjectBelongsToDifferentStore,
        ErrorCode::InvitesCannotBeMoved,
        ErrorCode::InvalidSpan,
        ErrorCode::CalendarHasNoSource,
        ErrorCode::CalendarSourceCannotBeModified,
        ErrorCode::CalendarIsImmutable,
        ErrorCode::SourceDoesNotAllowCalendarAddDelete,
        ErrorCode::RecurringReminderRequiresDueDate,
        ErrorCode::StructuredLocationsNotSupported,
        ErrorCode::ReminderLocationsNotSupported,
        ErrorCode::AlarmProximityNotSupported,
        ErrorCode::CalendarDoesNotAllowEvents,
        ErrorCode::CalendarDoesNotAllowReminders,
        ErrorCode::SourceDoesNotAllowReminders,
        ErrorCode::SourceDoesNotAllowEvents,
        ErrorCode::PriorityIsInvalid,
        ErrorCode::InvalidEntityType,
        ErrorCode::ProcedureAlarmsNotMutable,
        ErrorCode::EventStoreNotAuthorized,
        ErrorCode::OsNotSupported,
        ErrorCode::InvalidInviteReplyCalendar,
        ErrorCode::NotificationsCollectionFlagNotSet,
        ErrorCode::SourceMismatch,
        ErrorCode::NotificationCollectionMismatch,
        ErrorCode::NotificationSavedWithoutCollection,
    ];

    /// The category of this code.
    pub fn category(self) -> Category {
        use ErrorCode::*;
        match self {
            EventStoreNotAuthorized | CalendarDoesNotAllowEvents | CalendarDoesNotAllowReminders
            | SourceDoesNotAllowEvents | SourceDoesNotAllowReminders | SourceDoesNotAllowCalendarAddDelete => {
                Category::Access
            }
            EventNotMutable | CalendarReadOnly | CalendarIsImmutable | CalendarSourceCannotBeModified
            | InvitesCannotBeMoved | ProcedureAlarmsNotMutable => Category::ReadOnly,
            OsNotSupported | StructuredLocationsNotSupported | ReminderLocationsNotSupported
            | AlarmProximityNotSupported => Category::Unsupported,
            InternalFailure | InvalidSpan => Category::Internal,
            _ => Category::Validation,
        }
    }

    /// One sentence that says what went wrong.
    pub fn description(self) -> &'static str {
        use ErrorCode::*;
        match self {
            EventNotMutable => "The event is not mutable, so it cannot be saved or deleted.",
            NoCalendar => "The event is not associated with a calendar.",
            NoEndDate => "The event has no end date set.",
            NoStartDate => "The event has no start date set.",
            DatesInverted => "The end date of the event is before its start date.",
            InternalFailure => "An internal error occurred.",
            CalendarReadOnly => "The calendar is read-only, so events cannot be added to it.",
            DurationGreaterThanRecurrence => "The duration of the event is greater than its recurrence interval.",
            AlarmGreaterThanRecurrence => "The alarm interval is greater than the recurrence interval.",
            StartDateTooFarInFuture => "The start date is further in the future than the calendar can display.",
            StartDateCollidesWithOtherOccurrence => "The start date collides with another occurrence of the event.",
            ObjectBelongsToDifferentStore => "The object belongs to a different calendar store.",
            InvitesCannotBeMoved => "The event is an invitation and cannot be moved.",
            InvalidSpan => "An invalid span was found during a save or a deletion.",
            CalendarHasNoSource => "The calendar cannot be saved until a source is set.",
            CalendarSourceCannotBeModified => "The calendar cannot be moved to another source.",
            CalendarIsImmutable => "The calendar is immutable and cannot be modified or deleted.",
            SourceDoesNotAllowCalendarAddDelete => "The source does not allow calendars to be added or deleted.",
            RecurringReminderRequiresDueDate => "A recurring reminder requires a due date.",
            StructuredLocationsNotSupported => "The source of the calendar does not support structured locations.",
            ReminderLocationsNotSupported => "The source does not support locations on reminders.",
            AlarmProximityNotSupported => "The source does not allow geofences on alarms.",
            CalendarDoesNotAllowEvents => "The calendar does not allow events to be added.",
            CalendarDoesNotAllowReminders => "The calendar does not allow reminders to be added.",
            SourceDoesNotAllowReminders => "The source does not allow calendars that hold reminders.",
            SourceDoesNotAllowEvents => "The source does not allow calendars that hold events.",
            PriorityIsInvalid => "The priority number of the reminder is invalid.",
            InvalidEntityType => "The entity type is invalid.",
            ProcedureAlarmsNotMutable => "Procedure alarms cannot be created or modified.",
            EventStoreNotAuthorized => "The user has not authorized the app to access events or reminders.",
            OsNotSupported => "The action is not supported on this operating system.",
            InvalidInviteReplyCalendar => "The calendar for the invitation reply is invalid or missing.",
            NotificationsCollectionFlagNotSet => "The notification collection does not have the collection flag.",
            SourceMismatch => "The source of the object does not match the source of its container.",
            NotificationCollectionMismatch => "The notification collection does not match the one being saved.",
            NotificationSavedWithoutCollection => "The notification was not added to a collection and saved first.",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl std::error::Error for ErrorCode {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_list_has_the_official_thirty_six_entries_without_duplicates() {
        assert_eq!(ErrorCode::ALL.len(), 36);
        for (i, a) in ErrorCode::ALL.iter().enumerate() {
            for b in &ErrorCode::ALL[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn access_failures_are_access_and_not_validation() {
        assert_eq!(ErrorCode::EventStoreNotAuthorized.category(), Category::Access);
        assert_eq!(ErrorCode::SourceDoesNotAllowEvents.category(), Category::Access);
        assert_eq!(ErrorCode::CalendarDoesNotAllowReminders.category(), Category::Access);
    }

    #[test]
    fn a_date_mistake_is_a_validation_failure() {
        assert_eq!(ErrorCode::DatesInverted.category(), Category::Validation);
        assert_eq!(ErrorCode::NoStartDate.category(), Category::Validation);
        assert_eq!(ErrorCode::PriorityIsInvalid.category(), Category::Validation);
    }

    #[test]
    fn read_only_and_unsupported_codes_are_separate_categories() {
        assert_eq!(ErrorCode::CalendarIsImmutable.category(), Category::ReadOnly);
        assert_eq!(ErrorCode::InvitesCannotBeMoved.category(), Category::ReadOnly);
        assert_eq!(ErrorCode::AlarmProximityNotSupported.category(), Category::Unsupported);
        assert_eq!(ErrorCode::OsNotSupported.category(), Category::Unsupported);
        assert_eq!(ErrorCode::InternalFailure.category(), Category::Internal);
    }

    #[test]
    fn every_code_has_a_sentence_and_display_uses_it() {
        for code in ErrorCode::ALL {
            let text = code.description();
            assert!(text.ends_with('.'), "{:?} needs a full sentence", code);
            assert_eq!(code.to_string(), text);
        }
        assert_eq!(
            ErrorCode::DatesInverted.to_string(),
            "The end date of the event is before its start date."
        );
    }
}
