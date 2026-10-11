//! Signposts for apps: named intervals and events that a profiler pairs up and measures. The bookkeeping is
//! `bifrost_core::signpost`. Finished intervals and events also go to the app's unified log (see `log`), so the
//! Console app shows them. Times come from a monotonic clock that starts when the signposter is made.
use std::time::Instant;

use bifrost_core::log::Logger;
use bifrost_core::signpost::Signposter as Core;
pub use bifrost_core::signpost::{Error, Event, Id, Interval, Open, Stats, Summary};

pub struct Signposter {
    core: Core,
    log: Logger,
    origin: Instant,
}

impl Signposter {
    /// A signposter that logs under `subsystem` and `category`, the same pair a unified-log `Logger` takes.
    pub fn new(subsystem: &str, category: &str) -> Self {
        Self { core: Core::new(), log: Logger::new(subsystem, category), origin: Instant::now() }
    }

    /// A signposter that accepts every call and logs nothing.
    pub fn disabled(subsystem: &str, category: &str) -> Self {
        Self { core: Core::disabled(), log: Logger::new(subsystem, category), origin: Instant::now() }
    }

    pub fn is_enabled(&self) -> bool {
        self.core.is_enabled()
    }

    fn now(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// A new id, unique within this signposter.
    pub fn make_id(&mut self) -> Id {
        self.core.make_id()
    }

    /// Begins an interval. Keep the returned `Open` and pass it to `end` when the work finishes.
    pub fn begin(&self, name: &str, id: Id) -> Open {
        self.core.begin(name, id, self.now())
    }

    /// Ends an interval begun under `name`, logs it at info level and returns it (None when disabled).
    pub fn end(&self, open: Open, name: &str) -> Result<Option<Interval>, Error> {
        let done = self.core.end(open, name, self.now())?;
        if let Some(interval) = &done {
            self.log.info(format!("interval {} id={} {} us", interval.name, interval.id.0, interval.duration_ns() / 1000));
        }
        Ok(done)
    }

    /// Marks a point of interest now and logs it at info level.
    pub fn event(&self, name: &str, id: Id) {
        if let Some(event) = self.core.event(name, id, self.now()) {
            self.log.info(format!("event {} id={}", event.name, event.id.0));
        }
    }

    /// Measures the closure as one interval named `name` and returns its value.
    pub fn measure<T>(&mut self, name: &str, work: impl FnOnce() -> T) -> T {
        let id = self.make_id();
        let open = self.begin(name, id);
        let out = work();
        // The name is the one begin used, so this end cannot be refused.
        let _ = self.end(open, name);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_returns_the_value_of_the_closure() {
        let mut sp = Signposter::disabled("is.olibuijr.test", "signpost");
        assert_eq!(sp.measure("sum", || 2 + 3), 5);
    }

    #[test]
    fn a_disabled_signposter_ends_with_no_interval() {
        let mut sp = Signposter::disabled("is.olibuijr.test", "signpost");
        let id = sp.make_id();
        let open = sp.begin("draw", id);
        assert_eq!(sp.end(open, "draw").unwrap(), None);
        assert!(!sp.is_enabled());
    }

    #[test]
    fn ending_under_another_name_is_refused() {
        let sp = Signposter::disabled("is.olibuijr.test", "signpost");
        let open = sp.begin("draw", Id(1));
        assert_eq!(sp.end(open, "paint"), Err(Error::NameMismatch));
    }

    #[test]
    fn ids_from_make_id_are_distinct() {
        let mut sp = Signposter::disabled("is.olibuijr.test", "signpost");
        assert_ne!(sp.make_id(), sp.make_id());
    }

    #[test]
    fn an_enabled_signposter_returns_the_finished_interval() {
        let sp = Signposter::new("is.olibuijr.test", "signpost");
        assert!(sp.is_enabled());
        let open = sp.begin("parse", Id::from_key("doc-1"));
        let done = sp.end(open, "parse").unwrap().expect("an enabled signposter returns the interval");
        assert_eq!(done.name, "parse");
        assert_eq!(done.id, Id::from_key("doc-1"));
        assert!(done.ended_ns >= done.begun_ns, "the monotonic clock never runs backwards");
    }
}
