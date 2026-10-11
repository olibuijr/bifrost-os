//! Signposts: named intervals and events that mark where work starts and ends, so that a trace or the log can show
//! how long a task took. This is a pure model. The caller passes the time (a `Duration` from a fixed origin, such
//! as a process-start `Instant`), and finished records wait in memory until `drain` takes them. Nothing is written
//! anywhere. A disabled signposter emits nothing at all.

use std::time::Duration;

/// Tells apart intervals that share a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignpostId(pub u64);

/// An open interval, returned by `begin_interval`. Hand it back to `end_interval` with the same name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalState {
    name: String,
    id: SignpostId,
    begin: Duration,
}

impl IntervalState {
    pub fn id(&self) -> SignpostId {
        self.id
    }
}

/// A finished record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Interval { name: String, id: SignpostId, begin: Duration, end: Duration, message: Option<String> },
    Event { name: String, id: SignpostId, at: Duration, message: Option<String> },
}

#[derive(Debug)]
pub struct Signposter {
    subsystem: String,
    category: String,
    enabled: bool,
    next_id: u64,
    records: Vec<Record>,
}

impl Signposter {
    pub fn new(subsystem: &str, category: &str) -> Self {
        Signposter {
            subsystem: subsystem.to_string(),
            category: category.to_string(),
            enabled: true,
            next_id: 1,
            records: Vec::new(),
        }
    }

    /// A signposter that emits nothing, so call sites compile the same whether signposts are on or off.
    pub fn disabled() -> Self {
        Signposter { enabled: false, ..Signposter::new("", "") }
    }

    pub fn subsystem(&self) -> &str {
        &self.subsystem
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// A new identifier, unique within this signposter.
    pub fn make_id(&mut self) -> SignpostId {
        let id = SignpostId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Opens an interval at time `at`. Returns None when the signposter is disabled.
    pub fn begin_interval(&mut self, name: &str, id: SignpostId, at: Duration) -> Option<IntervalState> {
        if !self.enabled {
            return None;
        }
        Some(IntervalState { name: name.to_string(), id, begin: at })
    }

    /// Closes the interval at time `at` and returns its length. A name that does not match the state returns None
    /// and records nothing. An end before the begin gives a zero length.
    pub fn end_interval(
        &mut self,
        name: &str,
        state: IntervalState,
        at: Duration,
        message: Option<&str>,
    ) -> Option<Duration> {
        if !self.enabled || state.name != name {
            return None;
        }
        let end = at.max(state.begin);
        self.records.push(Record::Interval {
            name: state.name,
            id: state.id,
            begin: state.begin,
            end,
            message: message.map(str::to_string),
        });
        Some(end - state.begin)
    }

    /// Records a point in time.
    pub fn event(&mut self, name: &str, id: SignpostId, at: Duration, message: Option<&str>) {
        if !self.enabled {
            return;
        }
        self.records.push(Record::Event {
            name: name.to_string(),
            id,
            at,
            message: message.map(str::to_string),
        });
    }

    /// Takes every finished record, oldest first.
    pub fn drain(&mut self) -> Vec<Record> {
        std::mem::take(&mut self.records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn interval_reports_its_length_and_bounds() {
        let mut sp = Signposter::new("is.olibuijr.app", "load");
        let id = sp.make_id();
        let state = sp.begin_interval("decode", id, secs(2)).unwrap();
        assert_eq!(state.id(), id);
        assert_eq!(sp.end_interval("decode", state, secs(5), Some("ok")), Some(secs(3)));
        assert_eq!(
            sp.drain(),
            vec![Record::Interval {
                name: "decode".into(),
                id,
                begin: secs(2),
                end: secs(5),
                message: Some("ok".into()),
            }]
        );
    }

    #[test]
    fn end_with_a_different_name_is_ignored() {
        let mut sp = Signposter::new("is.olibuijr.app", "load");
        let id = sp.make_id();
        let state = sp.begin_interval("load", id, secs(1)).unwrap();
        assert_eq!(sp.end_interval("save", state.clone(), secs(4), None), None);
        assert!(sp.drain().is_empty());
        assert_eq!(sp.end_interval("load", state, secs(4), None), Some(secs(3)));
    }

    #[test]
    fn end_before_begin_gives_zero_length() {
        let mut sp = Signposter::new("is.olibuijr.app", "load");
        let id = sp.make_id();
        let state = sp.begin_interval("x", id, secs(5)).unwrap();
        assert_eq!(sp.end_interval("x", state, secs(1), None), Some(Duration::ZERO));
    }

    #[test]
    fn disabled_signposter_emits_nothing() {
        let mut sp = Signposter::disabled();
        assert!(!sp.is_enabled());
        let id = sp.make_id();
        assert!(sp.begin_interval("x", id, secs(1)).is_none());
        sp.event("hit", id, secs(1), None);
        assert!(sp.drain().is_empty());
    }

    #[test]
    fn ids_are_unique_within_a_signposter() {
        let mut sp = Signposter::new("is.olibuijr.app", "load");
        let a = sp.make_id();
        let b = sp.make_id();
        assert_ne!(a, b);
    }

    #[test]
    fn events_keep_their_message_and_drain_empties_the_queue() {
        let mut sp = Signposter::new("is.olibuijr.app", "cache");
        let id = sp.make_id();
        sp.event("hit", id, secs(7), Some("page 3"));
        assert_eq!(
            sp.drain(),
            vec![Record::Event { name: "hit".into(), id, at: secs(7), message: Some("page 3".into()) }]
        );
        assert!(sp.drain().is_empty());
    }
}
