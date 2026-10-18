//! Process assertions (RunningBoard): the claims an app makes so that the shell keeps it running while nothing shows it
//! (audio playing, a download, an export). Pure model with no clock of its own: the shell passes a millisecond clock.
//!
//! Every assertion has a reason the user can read in a diagnostic and a time limit, so a forgotten or crashed holder
//! never keeps an app running for ever: an app that still needs to run renews by taking a new assertion before the
//! old one expires. An app holds at most [`MAX_PER_APP`] assertions at once. `Occlusion` (in `occlusion.rs`) reads
//! "does the app hold one" as `Event::Assertion`; this module decides that answer.

use std::collections::BTreeMap;

/// Assertions one app may hold at the same time.
pub const MAX_PER_APP: usize = 8;
/// Longest reason text, in bytes.
pub const MAX_REASON: usize = 64;
/// The limit of an assertion that names none, in milliseconds.
pub const DEFAULT_LIMIT_MS: u64 = 60_000;
/// The longest limit an assertion may ask for, in milliseconds. A larger request is cut to this.
pub const MAX_LIMIT_MS: u64 = 3_600_000;

/// Why [`AssertionBook::take`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The reason is empty, longer than [`MAX_REASON`] or holds a control character.
    BadReason,
    /// The app already holds [`MAX_PER_APP`] assertions.
    TooMany,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::BadReason => write!(f, "the reason must be 1 to {MAX_REASON} characters with no control characters"),
            Refused::TooMany => write!(f, "an app may hold {MAX_PER_APP} process assertions at most"),
        }
    }
}

/// One held assertion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assertion {
    pub id: u32,
    pub reason: String,
    /// The millisecond at which the assertion lapses.
    pub expires_ms: u64,
}

/// The assertions of every app, by slot. Slots are the shell's external app slots.
#[derive(Clone, Debug, Default)]
pub struct AssertionBook {
    next_id: u32,
    held: BTreeMap<u8, Vec<Assertion>>,
}

impl AssertionBook {
    /// Takes an assertion for `slot` at `now_ms`. `limit_ms` of `None` means [`DEFAULT_LIMIT_MS`]; a limit of zero is
    /// raised to one millisecond and a limit above [`MAX_LIMIT_MS`] is cut to it. Returns the assertion id.
    pub fn take(&mut self, slot: u8, reason: &str, limit_ms: Option<u64>, now_ms: u64) -> Result<u32, Refused> {
        if reason.is_empty() || reason.len() > MAX_REASON || reason.chars().any(char::is_control) {
            return Err(Refused::BadReason);
        }
        self.expire(now_ms);
        let list = self.held.entry(slot).or_default();
        if list.len() >= MAX_PER_APP {
            return Err(Refused::TooMany);
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let limit = limit_ms.unwrap_or(DEFAULT_LIMIT_MS).clamp(1, MAX_LIMIT_MS);
        list.push(Assertion { id: self.next_id, reason: reason.to_string(), expires_ms: now_ms.saturating_add(limit) });
        Ok(self.next_id)
    }

    /// Drops the assertion `id` of `slot`. Returns whether it was held.
    pub fn release(&mut self, slot: u8, id: u32) -> bool {
        let Some(list) = self.held.get_mut(&slot) else { return false };
        let before = list.len();
        list.retain(|a| a.id != id);
        let released = list.len() != before;
        if list.is_empty() {
            self.held.remove(&slot);
        }
        released
    }

    /// Drops every assertion that has lapsed by `now_ms`. Returns the lapsed ones as `(slot, id)`.
    pub fn expire(&mut self, now_ms: u64) -> Vec<(u8, u32)> {
        let mut lapsed = Vec::new();
        for (slot, list) in self.held.iter_mut() {
            list.retain(|a| {
                let live = a.expires_ms > now_ms;
                if !live {
                    lapsed.push((*slot, a.id));
                }
                live
            });
        }
        self.held.retain(|_, list| !list.is_empty());
        lapsed
    }

    /// Drops everything `slot` held: the app quit.
    pub fn forget(&mut self, slot: u8) {
        self.held.remove(&slot);
    }

    /// Whether `slot` holds an assertion (call [`expire`](Self::expire) first so a lapsed one does not count).
    pub fn is_held(&self, slot: u8) -> bool {
        self.held.get(&slot).is_some_and(|l| !l.is_empty())
    }

    /// The assertions `slot` holds, oldest first.
    pub fn of(&self, slot: u8) -> &[Assertion] {
        self.held.get(&slot).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_assertion_is_held_until_released() {
        let mut b = AssertionBook::default();
        let id = b.take(1, "Exporting video", None, 0).unwrap();
        assert!(b.is_held(1));
        assert!(!b.is_held(2));
        assert!(b.release(1, id));
        assert!(!b.is_held(1));
        assert!(!b.release(1, id), "released once");
    }

    #[test]
    fn an_assertion_lapses_at_its_limit() {
        let mut b = AssertionBook::default();
        b.take(3, "Download", Some(1_000), 500).unwrap();
        assert!(b.expire(1_499).is_empty());
        assert!(b.is_held(3));
        let lapsed = b.expire(1_500);
        assert_eq!(lapsed.len(), 1);
        assert_eq!(lapsed[0].0, 3);
        assert!(!b.is_held(3));
    }

    #[test]
    fn the_limit_defaults_and_is_capped() {
        let mut b = AssertionBook::default();
        b.take(0, "a", None, 10).unwrap();
        b.take(0, "b", Some(u64::MAX), 10).unwrap();
        b.take(0, "c", Some(0), 10).unwrap();
        let list = b.of(0);
        assert_eq!(list[0].expires_ms, 10 + DEFAULT_LIMIT_MS);
        assert_eq!(list[1].expires_ms, 10 + MAX_LIMIT_MS);
        assert_eq!(list[2].expires_ms, 11);
    }

    #[test]
    fn reasons_are_checked() {
        let mut b = AssertionBook::default();
        assert_eq!(b.take(0, "", None, 0), Err(Refused::BadReason));
        assert_eq!(b.take(0, "line\nbreak", None, 0), Err(Refused::BadReason));
        assert_eq!(b.take(0, &"x".repeat(MAX_REASON + 1), None, 0), Err(Refused::BadReason));
        assert!(b.take(0, &"x".repeat(MAX_REASON), None, 0).is_ok());
    }

    #[test]
    fn an_app_holds_a_limited_number() {
        let mut b = AssertionBook::default();
        for _ in 0..MAX_PER_APP {
            b.take(5, "work", None, 0).unwrap();
        }
        assert_eq!(b.take(5, "one more", None, 0), Err(Refused::TooMany));
        assert!(b.take(6, "another app", None, 0).is_ok(), "the cap is per app");
    }

    #[test]
    fn a_lapsed_assertion_frees_room_for_a_new_one() {
        let mut b = AssertionBook::default();
        for _ in 0..MAX_PER_APP {
            b.take(5, "work", Some(10), 0).unwrap();
        }
        assert!(b.take(5, "renewed", Some(10), 10).is_ok());
        assert_eq!(b.of(5).len(), 1);
    }

    #[test]
    fn ids_are_unique_and_forget_drops_an_app() {
        let mut b = AssertionBook::default();
        let a = b.take(1, "a", None, 0).unwrap();
        let c = b.take(2, "c", None, 0).unwrap();
        assert_ne!(a, c);
        b.forget(1);
        assert!(!b.is_held(1));
        assert!(b.is_held(2));
        assert!(!b.release(2, a), "an id of another app is not released");
    }
}
