//! Sleep assertions that an app holds while it works: the registry behind an app's begin and end of an activity
//! with idle-sleep options. Two flags keep the display or the machine awake while the assertion is held:
//! [`ActivityOptions::IDLE_DISPLAY_SLEEP_DISABLED`] and [`ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED`]. Other
//! flags are stored but do not block sleep.
//!
//! This is a pure model. It hands out ids, keeps each reason string and answers whether sleep is blocked. The
//! system layer decides what that means: `springboard/src/bare/sleep.rs` reads the blockers it already sees, and
//! nothing here talks to a power service.

use std::collections::BTreeMap;

/// Option flags for an activity. The bit values are local to this model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivityOptions(pub u64);

impl ActivityOptions {
    /// Keep the screen powered on while the activity runs.
    pub const IDLE_DISPLAY_SLEEP_DISABLED: ActivityOptions = ActivityOptions(1 << 0);
    /// Keep the machine from sleeping while idle.
    pub const IDLE_SYSTEM_SLEEP_DISABLED: ActivityOptions = ActivityOptions(1 << 1);
    /// Work the user asked for; not to be throttled.
    pub const USER_INITIATED: ActivityOptions = ActivityOptions(1 << 2);
    /// Timing-sensitive work; not to be throttled.
    pub const LATENCY_CRITICAL: ActivityOptions = ActivityOptions(1 << 3);
    /// Work that may be deferred.
    pub const BACKGROUND: ActivityOptions = ActivityOptions(1 << 4);

    pub const fn union(self, other: ActivityOptions) -> ActivityOptions {
        ActivityOptions(self.0 | other.0)
    }

    /// True when every flag in `other` is set in `self`.
    pub const fn contains(self, other: ActivityOptions) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when the options keep the display on.
    pub const fn blocks_display_sleep(self) -> bool {
        self.contains(ActivityOptions::IDLE_DISPLAY_SLEEP_DISABLED)
    }

    /// True when the options keep the machine from sleeping.
    pub const fn blocks_system_sleep(self) -> bool {
        self.contains(ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    options: ActivityOptions,
    reason: String,
}

/// The assertions currently held, keyed by id. Ids start at 1 and are never reused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SleepAssertions {
    next_id: u64,
    held: BTreeMap<u64, Held>,
}

impl SleepAssertions {
    pub fn new() -> Self {
        SleepAssertions { next_id: 1, held: BTreeMap::new() }
    }

    /// Begins an activity and returns its id. The reason is kept as given, so the shell can show why sleep is held.
    pub fn begin(&mut self, options: ActivityOptions, reason: &str) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.held.insert(id, Held { options, reason: reason.to_string() });
        id
    }

    /// Ends an activity. Returns `false` when the id is not held.
    pub fn end(&mut self, id: u64) -> bool {
        self.held.remove(&id).is_some()
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// True while any held assertion keeps the display on.
    pub fn blocks_display_sleep(&self) -> bool {
        self.held.values().any(|h| h.options.blocks_display_sleep())
    }

    /// True while any held assertion keeps the machine awake.
    pub fn blocks_system_sleep(&self) -> bool {
        self.held.values().any(|h| h.options.blocks_system_sleep())
    }

    /// The reasons of the assertions that block sleep, in the order they were begun.
    pub fn sleep_reasons(&self) -> Vec<&str> {
        self.held
            .values()
            .filter(|h| h.options.blocks_display_sleep() || h.options.blocks_system_sleep())
            .map(|h| h.reason.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAY: ActivityOptions = ActivityOptions::IDLE_DISPLAY_SLEEP_DISABLED;
    const SYSTEM: ActivityOptions = ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED;

    #[test]
    fn ids_start_at_one_and_increase() {
        let mut a = SleepAssertions::new();
        assert_eq!(a.begin(DISPLAY, "video"), 1);
        assert_eq!(a.begin(SYSTEM, "download"), 2);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn display_flag_blocks_only_display_sleep() {
        let mut a = SleepAssertions::new();
        a.begin(DISPLAY, "presentation");
        assert!(a.blocks_display_sleep());
        assert!(!a.blocks_system_sleep());
    }

    #[test]
    fn system_flag_blocks_only_system_sleep() {
        let mut a = SleepAssertions::new();
        a.begin(SYSTEM, "copy");
        assert!(a.blocks_system_sleep());
        assert!(!a.blocks_display_sleep());
    }

    #[test]
    fn non_sleep_flags_are_held_but_do_not_block() {
        let mut a = SleepAssertions::new();
        let id = a.begin(ActivityOptions::USER_INITIATED.union(ActivityOptions::LATENCY_CRITICAL), "edit");
        assert_eq!(a.len(), 1);
        assert!(!a.blocks_display_sleep());
        assert!(!a.blocks_system_sleep());
        assert!(a.sleep_reasons().is_empty());
        assert!(a.end(id));
    }

    #[test]
    fn ending_the_last_blocking_assertion_releases_sleep() {
        let mut a = SleepAssertions::new();
        let video = a.begin(DISPLAY, "video");
        let copy = a.begin(SYSTEM, "copy");
        assert!(a.end(video));
        assert!(!a.blocks_display_sleep());
        assert!(a.blocks_system_sleep(), "the copy still holds the machine awake");
        assert!(a.end(copy));
        assert!(a.is_empty());
        assert!(!a.blocks_system_sleep());
    }

    #[test]
    fn ending_an_unknown_id_reports_false_and_changes_nothing() {
        let mut a = SleepAssertions::new();
        let id = a.begin(DISPLAY, "video");
        assert!(!a.end(id + 7));
        assert_eq!(a.len(), 1);
        assert!(a.end(id));
        assert!(!a.end(id), "a second end of the same id is refused");
    }

    #[test]
    fn sleep_reasons_list_blockers_in_begin_order() {
        let mut a = SleepAssertions::new();
        a.begin(SYSTEM, "first");
        a.begin(ActivityOptions::BACKGROUND, "not a blocker");
        a.begin(DISPLAY.union(SYSTEM), "third");
        assert_eq!(a.sleep_reasons(), vec!["first", "third"]);
    }

    #[test]
    fn options_contains_and_union_work_on_bits() {
        let both = DISPLAY.union(SYSTEM);
        assert!(both.contains(DISPLAY));
        assert!(both.contains(SYSTEM));
        assert!(!DISPLAY.contains(SYSTEM));
        assert_eq!(both.0, 0b11);
    }
}
