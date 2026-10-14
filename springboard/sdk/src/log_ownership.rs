//! Subsystem ownership for log tools: which store app declared a subsystem, and whether a writing process may log as
//! it. The writer's identity is the executable path journald records for the process (set by the kernel, so an app
//! cannot fake it): a store app runs from `/var/lib/springboard/apps/<id>/...`, and may log only as the subsystems its
//! manifest declared. The log store applies this rule on every read (an entry that claims another app's subsystem
//! loses the subsystem and its category). This module gives the same decision to tools that read the registry, such
//! as a log viewer that explains why an entry was refused.

use crate::log::{may_log_as, valid_subsystem, Spec};

/// Why a process may not log as a subsystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// The text is not a subsystem id (letters, digits, `.`, `-` and `_`, 1 to 128 characters).
    Invalid(String),
    /// The subsystem was declared by another store app.
    OwnedByAnother { subsystem: String, owner: String },
    /// The shell's own subsystem, which no store app may use.
    Reserved(String),
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaimError::Invalid(id) => write!(f, "{id:?} is not a subsystem id"),
            ClaimError::OwnedByAnother { subsystem, owner } => write!(f, "subsystem {subsystem} belongs to {owner}"),
            ClaimError::Reserved(subsystem) => write!(f, "subsystem {subsystem} is reserved for the system"),
        }
    }
}

/// The store app that declared `subsystem` in its manifest, if one did.
pub fn owner_of<'a>(registry: &'a [Spec], subsystem: &str) -> Option<&'a str> {
    registry.iter().find(|s| s.id == subsystem).and_then(|s| s.owner.as_deref())
}

/// Checks that a process run from `exe` (`None` for a system binary) may write entries under `subsystem`.
pub fn check_claim(registry: &[Spec], exe: Option<&str>, subsystem: &str) -> Result<(), ClaimError> {
    if !valid_subsystem(subsystem) {
        return Err(ClaimError::Invalid(subsystem.to_string()));
    }
    if may_log_as(registry, exe, subsystem) {
        return Ok(());
    }
    match owner_of(registry, subsystem) {
        Some(owner) => Err(ClaimError::OwnedByAnother { subsystem: subsystem.to_string(), owner: owner.to_string() }),
        None => Err(ClaimError::Reserved(subsystem.to_string())),
    }
}
