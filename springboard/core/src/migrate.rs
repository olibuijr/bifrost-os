//! Versioned data migrations: an ordered chain of steps, one per version, that brings data written by an older build up
//! to the version this build writes. Each step is a plain function over the data. The chain refuses data written by a
//! newer build, rather than reading fields it does not know and writing them back without them.

use std::fmt;

/// Why a migration did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationError {
    /// The data was written by a newer build. Reading it could drop fields this build does not know.
    NewerThanBuild { found: u32, current: u32 },
    /// No step starts at this version, so the chain has a gap.
    MissingStep { from: u32 },
    /// A step refused the data.
    StepFailed { from: u32, name: &'static str, reason: String },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationError::NewerThanBuild { found, current } => {
                write!(f, "data version {found} is newer than this build ({current})")
            }
            MigrationError::MissingStep { from } => write!(f, "no migration step from version {from}"),
            MigrationError::StepFailed { from, name, reason } => {
                write!(f, "migration {name} from version {from} failed: {reason}")
            }
        }
    }
}

impl std::error::Error for MigrationError {}

type StepFn<T> = Box<dyn Fn(T) -> Result<T, String>>;

/// The migration chain for one kind of data. `current` is the version this build writes.
pub struct Plan<T> {
    current: u32,
    steps: Vec<(u32, &'static str, StepFn<T>)>,
}

impl<T: 'static> Plan<T> {
    pub fn new(current: u32) -> Self {
        Plan { current, steps: Vec::new() }
    }

    /// Adds the step that reads version `from` and produces `from + 1`. `name` appears in errors.
    pub fn step(mut self, from: u32, name: &'static str, run: impl Fn(T) -> Result<T, String> + 'static) -> Self {
        self.steps.push((from, name, Box::new(run)));
        self
    }

    /// Checks that a step exists for every version below `current`. Call it once, when the plan is built.
    pub fn check(&self) -> Result<(), MigrationError> {
        for from in 0..self.current {
            if !self.steps.iter().any(|(v, _, _)| *v == from) {
                return Err(MigrationError::MissingStep { from });
            }
        }
        Ok(())
    }

    /// Brings `data`, written at version `found`, up to the current version by running each step in turn.
    /// Data already at the current version comes back unchanged.
    pub fn migrate(&self, found: u32, data: T) -> Result<T, MigrationError> {
        if found > self.current {
            return Err(MigrationError::NewerThanBuild { found, current: self.current });
        }
        let mut data = data;
        for version in found..self.current {
            let step = self
                .steps
                .iter()
                .find(|(v, _, _)| *v == version)
                .ok_or(MigrationError::MissingStep { from: version })?;
            let (_, name, run) = step;
            data = run(data).map_err(|reason| MigrationError::StepFailed { from: version, name: *name, reason })?;
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Version 0 is a bare list. Version 1 adds "one", version 2 adds "two", and the step to version 3 refuses
    /// data marked "bad".
    fn plan() -> Plan<Vec<String>> {
        Plan::new(3)
            .step(0, "add one", |mut d: Vec<String>| {
                d.push("one".into());
                Ok(d)
            })
            .step(1, "add two", |mut d: Vec<String>| {
                d.push("two".into());
                Ok(d)
            })
            .step(2, "check", |d: Vec<String>| {
                if d.iter().any(|s| s == "bad") {
                    Err("marked bad".into())
                } else {
                    Ok(d)
                }
            })
    }

    #[test]
    fn runs_every_step_in_order_from_the_oldest_version() {
        assert_eq!(plan().check(), Ok(()));
        assert_eq!(plan().migrate(0, vec![]), Ok(vec!["one".to_string(), "two".to_string()]));
    }

    #[test]
    fn starts_from_the_stored_version_and_skips_earlier_steps() {
        assert_eq!(plan().migrate(1, vec!["kept".into()]), Ok(vec!["kept".to_string(), "two".to_string()]));
    }

    #[test]
    fn data_at_the_current_version_is_returned_unchanged() {
        assert_eq!(plan().migrate(3, vec!["x".into()]), Ok(vec!["x".to_string()]));
    }

    #[test]
    fn data_from_a_newer_build_is_refused() {
        assert_eq!(
            plan().migrate(4, vec![]),
            Err(MigrationError::NewerThanBuild { found: 4, current: 3 })
        );
    }

    #[test]
    fn a_refusing_step_reports_its_version_and_name() {
        assert_eq!(
            plan().migrate(2, vec!["bad".into()]),
            Err(MigrationError::StepFailed { from: 2, name: "check", reason: "marked bad".into() })
        );
    }

    #[test]
    fn the_check_finds_a_gap_in_the_chain() {
        let gapped: Plan<Vec<String>> = Plan::new(3).step(0, "a", |d| Ok(d)).step(2, "c", |d| Ok(d));
        assert_eq!(gapped.check(), Err(MigrationError::MissingStep { from: 1 }));
        assert_eq!(
            gapped.migrate(0, vec![]),
            Err(MigrationError::MissingStep { from: 1 })
        );
    }

    #[test]
    fn errors_have_readable_text() {
        let text = MigrationError::NewerThanBuild { found: 4, current: 3 }.to_string();
        assert_eq!(text, "data version 4 is newer than this build (3)");
    }
}
