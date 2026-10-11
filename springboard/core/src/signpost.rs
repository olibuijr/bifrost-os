//! Signposts: named, id-keyed intervals and events that a profiler pairs up and measures. This module is the pure
//! bookkeeping. A `Signposter` hands out ids and makes sure each interval ends once, under the name it began with and
//! from the signposter that began it. It returns the finished `Interval`. Times are nanoseconds from any fixed origin
//! the caller supplies, so nothing here reads a clock. `Summary` folds finished intervals into per-name totals, the
//! shape of a daily metric report.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Identifies one interval among those that share a name. Ids are unique within one signposter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub u64);

impl Id {
    /// A stable id for a text key (FNV-1a), so the same key gives the same id in every process.
    pub fn from_key(key: &str) -> Id {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in key.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Id(hash)
    }
}

/// Why a signpost call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The interval was begun by a different signposter.
    ForeignInterval,
    /// `end` named a different interval than `begin` did.
    NameMismatch,
    /// The end time is earlier than the begin time.
    TimeReversed,
}

/// An interval that has begun and not yet ended. `Signposter::end` consumes it, so it can end only once.
#[derive(Debug, PartialEq, Eq)]
pub struct Open {
    owner: u64,
    name: String,
    id: Id,
    begun_ns: u64,
}

impl Open {
    pub fn id(&self) -> Id {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A finished interval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    pub name: String,
    pub id: Id,
    pub begun_ns: u64,
    pub ended_ns: u64,
}

impl Interval {
    pub fn duration_ns(&self) -> u64 {
        self.ended_ns.saturating_sub(self.begun_ns)
    }
}

/// A point of interest in time, with no duration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub id: Id,
    pub at_ns: u64,
}

/// Begins and ends intervals and emits events. A disabled signposter accepts the same calls and records nothing,
/// so callers need no checks of their own.
#[derive(Debug)]
pub struct Signposter {
    owner: u64,
    enabled: bool,
    last_id: u64,
}

impl Signposter {
    pub fn new() -> Self {
        Self { owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed), enabled: true, last_id: 0 }
    }

    pub fn disabled() -> Self {
        Self { enabled: false, ..Self::new() }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// A new id, unique within this signposter.
    pub fn make_id(&mut self) -> Id {
        self.last_id += 1;
        Id(self.last_id)
    }

    /// Begins an interval at `begun_ns`. Pass the returned `Open` to `end` when the work finishes.
    pub fn begin(&self, name: &str, id: Id, begun_ns: u64) -> Open {
        Open { owner: self.owner, name: name.to_string(), id, begun_ns }
    }

    /// Ends `open` at `ended_ns`. `name` must match the name it began with. Returns the finished interval, or None
    /// when this signposter is disabled.
    pub fn end(&self, open: Open, name: &str, ended_ns: u64) -> Result<Option<Interval>, Error> {
        if open.owner != self.owner {
            return Err(Error::ForeignInterval);
        }
        if open.name != name {
            return Err(Error::NameMismatch);
        }
        if ended_ns < open.begun_ns {
            return Err(Error::TimeReversed);
        }
        if !self.enabled {
            return Ok(None);
        }
        Ok(Some(Interval { name: open.name, id: open.id, begun_ns: open.begun_ns, ended_ns }))
    }

    /// A point of interest at `at_ns`, or None when disabled.
    pub fn event(&self, name: &str, id: Id, at_ns: u64) -> Option<Event> {
        self.enabled.then(|| Event { name: name.to_string(), id, at_ns })
    }
}

impl Default for Signposter {
    fn default() -> Self {
        Self::new()
    }
}

/// Totals for one interval name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub count: u64,
    pub total_ns: u64,
    pub max_ns: u64,
}

/// Per-name totals of finished intervals: how many, how long in all, and the longest one.
#[derive(Debug, Default)]
pub struct Summary {
    by_name: BTreeMap<String, Stats>,
}

impl Summary {
    pub fn record(&mut self, interval: &Interval) {
        let duration = interval.duration_ns();
        let stats = self.by_name.entry(interval.name.clone()).or_default();
        stats.count += 1;
        stats.total_ns = stats.total_ns.saturating_add(duration);
        stats.max_ns = stats.max_ns.max(duration);
    }

    pub fn stats(&self, name: &str) -> Option<Stats> {
        self.by_name.get(name).copied()
    }

    /// The recorded names, in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(|k| k.as_str())
    }

    /// Takes the totals and clears the summary, as a metric report is delivered once.
    pub fn take(&mut self) -> BTreeMap<String, Stats> {
        std::mem::take(&mut self.by_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_keys_are_stable_and_distinct() {
        assert_eq!(Id::from_key("request-7"), Id::from_key("request-7"));
        assert_ne!(Id::from_key("request-7"), Id::from_key("request-8"));
        assert_eq!(Id::from_key(""), Id(0xcbf2_9ce4_8422_2325), "an empty key keeps the FNV offset basis");
    }

    #[test]
    fn make_id_is_unique_within_a_signposter() {
        let mut sp = Signposter::new();
        let a = sp.make_id();
        let b = sp.make_id();
        assert_ne!(a, b);
        assert_ne!(a, Id(0));
    }

    #[test]
    fn an_interval_ends_once_under_its_own_name() {
        let sp = Signposter::new();
        let open = sp.begin("load", Id(1), 100);
        assert_eq!(open.name(), "load");
        assert_eq!(open.id(), Id(1));
        let done = sp.end(open, "load", 350).unwrap().unwrap();
        assert_eq!(done.duration_ns(), 250);
        assert_eq!(done.name, "load");
        let wrong = sp.begin("load", Id(2), 0);
        assert_eq!(sp.end(wrong, "save", 5), Err(Error::NameMismatch));
    }

    #[test]
    fn a_foreign_interval_is_refused() {
        let a = Signposter::new();
        let b = Signposter::new();
        let open = a.begin("draw", Id(1), 0);
        assert_eq!(b.end(open, "draw", 10), Err(Error::ForeignInterval));
    }

    #[test]
    fn a_reversed_clock_is_refused() {
        let sp = Signposter::new();
        let open = sp.begin("draw", Id(1), 500);
        assert_eq!(sp.end(open, "draw", 400), Err(Error::TimeReversed));
    }

    #[test]
    fn a_disabled_signposter_records_nothing() {
        let mut sp = Signposter::disabled();
        assert!(!sp.is_enabled());
        let id = sp.make_id();
        let open = sp.begin("draw", id, 0);
        assert_eq!(sp.end(open, "draw", 10), Ok(None), "the call is accepted, with no interval kept");
        assert!(sp.event("tap", id, 3).is_none());
    }

    #[test]
    fn events_carry_their_time_and_id() {
        let sp = Signposter::new();
        let e = sp.event("tap", Id(9), 42).unwrap();
        assert_eq!((e.name.as_str(), e.id, e.at_ns), ("tap", Id(9), 42));
    }

    #[test]
    fn summary_totals_each_name() {
        let sp = Signposter::new();
        let mut summary = Summary::default();
        for (name, begun, ended) in [("load", 0, 10), ("load", 100, 130), ("save", 0, 5)] {
            let open = sp.begin(name, Id(begun), begun);
            let done = sp.end(open, name, ended).unwrap().unwrap();
            summary.record(&done);
        }
        assert_eq!(summary.stats("load"), Some(Stats { count: 2, total_ns: 40, max_ns: 30 }));
        assert_eq!(summary.names().collect::<Vec<_>>(), vec!["load", "save"], "names come out sorted");
        assert_eq!(summary.stats("missing"), None);
        let taken = summary.take();
        assert_eq!(taken.len(), 2);
        assert!(summary.names().next().is_none(), "take clears the summary");
    }
}
