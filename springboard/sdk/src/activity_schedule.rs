//! Background activity scheduling: a unit of work that the system runs no sooner than an interval after its last run,
//! and at the latest within a tolerance after that point. An activity is either one-shot (it runs once) or repeating.
//!
//! The model takes time as whole seconds on a clock the caller supplies, so it is deterministic and needs no device.
//! `ActivitySchedule::decide` says what the system should do at a given time, and `run_if_due` runs the work and records
//! the run when the decision allows it. Process activities (`ActivityOptions`) keep an app alive and hint throttling;
//! this module decides when a piece of background work may run.

/// When a scheduled activity may run: a minimum spacing between runs, a tolerance, and whether it repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivityCriteria {
    interval_secs: u64,
    tolerance_secs: u64,
    repeats: bool,
}

impl ActivityCriteria {
    /// Criteria for a one-shot activity with a minimum spacing of `interval_secs` and a tolerance of
    /// `tolerance_secs`. None when the interval is zero or the tolerance is larger than the interval.
    pub fn new(interval_secs: u64, tolerance_secs: u64) -> Option<Self> {
        (interval_secs > 0 && tolerance_secs <= interval_secs).then_some(ActivityCriteria {
            interval_secs,
            tolerance_secs,
            repeats: false,
        })
    }

    /// The same criteria, repeating: the activity may run again once its interval has passed since the last run.
    pub fn repeating(self) -> Self {
        ActivityCriteria { repeats: true, ..self }
    }

    /// The minimum spacing between two runs, in seconds.
    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    /// How far past its due time a run may be deferred, in seconds.
    pub fn tolerance_secs(&self) -> u64 {
        self.tolerance_secs
    }

    /// Whether the activity runs again after its first run.
    pub fn repeats(&self) -> bool {
        self.repeats
    }
}

/// What the system should do with an activity at a given time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Not yet. The activity may run from `until` on.
    Wait { until: u64 },
    /// The activity may run now. It must have run by `deadline`, and a deadline already past means the run is late.
    Run { deadline: u64 },
    /// A one-shot activity that has already run.
    Finished,
}

/// One scheduled activity and the time of its last run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivitySchedule {
    criteria: ActivityCriteria,
    last_run: Option<u64>,
}

impl ActivitySchedule {
    /// A schedule that has not run yet.
    pub fn new(criteria: ActivityCriteria) -> Self {
        ActivitySchedule { criteria, last_run: None }
    }

    /// The criteria of the activity.
    pub fn criteria(&self) -> ActivityCriteria {
        self.criteria
    }

    /// The time of the last run, or None when the activity has not run.
    pub fn last_run(&self) -> Option<u64> {
        self.last_run
    }

    /// The decision at time `now`. The first run is due at once, with the tolerance as its deadline. A later run is due
    /// at the last run plus the interval, and its deadline is that time plus the tolerance.
    pub fn decide(&self, now: u64) -> Decision {
        match self.last_run {
            None => Decision::Run { deadline: now + self.criteria.tolerance_secs },
            Some(_) if !self.criteria.repeats => Decision::Finished,
            Some(last) => {
                let due = last + self.criteria.interval_secs;
                if now < due {
                    Decision::Wait { until: due }
                } else {
                    Decision::Run { deadline: (due + self.criteria.tolerance_secs).max(now) }
                }
            }
        }
    }

    /// Runs `work` when the activity is due at `now`, records `now` as the last run and returns true. Otherwise `work`
    /// does not run and the schedule is unchanged.
    pub fn run_if_due(&mut self, now: u64, work: impl FnOnce()) -> bool {
        match self.decide(now) {
            Decision::Run { .. } => {
                work();
                self.last_run = Some(now);
                true
            }
            Decision::Wait { .. } | Decision::Finished => false,
        }
    }
}
