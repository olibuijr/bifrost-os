//! Containers and probing (`probe and container` parity): the result a probe reports for a resource, the grade of
//! the match, the identifier of the container a probe found, and the state of a container. Pure model: no resource
//! is read and no module runs. The probe runner and the mount step belong to the system layer.
//!
//! Mirrors the match result (not recognized, recognized, usable but limited, usable), the probe result (name,
//! container identifier, match result), the container state (not ready, ready, active, blocked) and the container
//! status (state with an optional error). `best_mountable` chooses the probe to mount when several resources were
//! probed.
//!
//! Gaps: the errors are text, not a typed error domain; no resource kinds (block device, path, URL); no probe
//! scheduling and no container registry.

use std::fmt;

/// How well a probe recognized a resource. The order is the grade: `NotRecognized` is the lowest and `Usable`
/// the highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchResult {
    NotRecognized,
    /// Recognized, but the module cannot use it.
    Recognized,
    /// Recognized and usable, with limited capabilities.
    UsableButLimited,
    /// Recognized and ready to use.
    Usable,
}

impl MatchResult {
    /// True when a mount can be attempted: usable, or usable with limits.
    pub fn can_mount(self) -> bool {
        matches!(self, MatchResult::Usable | MatchResult::UsableButLimited)
    }

    /// True when the probe recognized the resource at all.
    pub fn is_recognized(self) -> bool {
        self != MatchResult::NotRecognized
    }
}

impl fmt::Display for MatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            MatchResult::NotRecognized => "not recognized",
            MatchResult::Recognized => "recognized",
            MatchResult::UsableButLimited => "usable but limited",
            MatchResult::Usable => "usable",
        };
        f.write_str(text)
    }
}

/// The identifier of a container: a non-empty name the module chose.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerIdentifier(String);

impl ContainerIdentifier {
    /// Wraps an identifier. `None` when it is empty.
    pub fn new(raw: &str) -> Option<Self> {
        (!raw.is_empty()).then(|| Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What one probe found: the resource name, the container it belongs to when one was found, and the match grade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub name: String,
    pub container: Option<ContainerIdentifier>,
    pub result: MatchResult,
}

impl ProbeResult {
    /// A resource the probe recognized but cannot use.
    pub fn recognized(name: &str, container: ContainerIdentifier) -> Self {
        Self::with(name, Some(container), MatchResult::Recognized)
    }

    /// A resource the probe recognized and can use.
    pub fn usable(name: &str, container: ContainerIdentifier) -> Self {
        Self::with(name, Some(container), MatchResult::Usable)
    }

    /// A resource the probe recognized and can use with limited capabilities.
    pub fn usable_but_limited(name: &str, container: ContainerIdentifier) -> Self {
        Self::with(name, Some(container), MatchResult::UsableButLimited)
    }

    /// A resource the probe did not recognize. No container is reported.
    pub fn not_recognized(name: &str) -> Self {
        Self::with(name, None, MatchResult::NotRecognized)
    }

    fn with(name: &str, container: Option<ContainerIdentifier>, result: MatchResult) -> Self {
        Self {
            name: name.to_string(),
            container,
            result,
        }
    }
}

/// Picks the probe to mount: the highest grade that can mount. Among probes of the same grade the first one wins.
/// `None` when no probe can mount.
pub fn best_mountable(results: &[ProbeResult]) -> Option<&ProbeResult> {
    results
        .iter()
        .filter(|r| r.result.can_mount())
        .fold(None, |best: Option<&ProbeResult>, r| match best {
            Some(b) if b.result >= r.result => Some(b),
            _ => Some(r),
        })
}

/// The state of a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerState {
    NotReady,
    Ready,
    Active,
    Blocked,
}

/// The state of a container with an optional error text that explains it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerStatus {
    state: ContainerState,
    error: Option<String>,
}

impl ContainerStatus {
    pub fn new(state: ContainerState, error: Option<&str>) -> Self {
        Self {
            state,
            error: error.map(|e| e.to_string()),
        }
    }

    /// A ready container with no error.
    pub fn ready() -> Self {
        Self::new(ContainerState::Ready, None)
    }

    /// An active container with no error.
    pub fn active() -> Self {
        Self::new(ContainerState::Active, None)
    }

    /// A container that is not ready yet, with the reason.
    pub fn not_ready(error: &str) -> Self {
        Self::new(ContainerState::NotReady, Some(error))
    }

    /// A blocked container, with the reason.
    pub fn blocked(error: &str) -> Self {
        Self::new(ContainerState::Blocked, Some(error))
    }

    pub fn state(&self) -> ContainerState {
        self.state
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// True when the container is ready or active.
    pub fn is_usable(&self) -> bool {
        matches!(self.state, ContainerState::Ready | ContainerState::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(s: &str) -> ContainerIdentifier {
        ContainerIdentifier::new(s).unwrap()
    }

    #[test]
    fn match_grades_are_ordered() {
        assert!(MatchResult::Usable > MatchResult::UsableButLimited);
        assert!(MatchResult::UsableButLimited > MatchResult::Recognized);
        assert!(MatchResult::Recognized > MatchResult::NotRecognized);
        assert!(MatchResult::UsableButLimited.can_mount());
        assert!(!MatchResult::Recognized.can_mount());
        assert!(!MatchResult::NotRecognized.is_recognized());
        assert!(MatchResult::Recognized.is_recognized());
        assert_eq!(MatchResult::UsableButLimited.to_string(), "usable but limited");
    }

    #[test]
    fn empty_container_identifier_is_refused() {
        assert!(ContainerIdentifier::new("").is_none());
        assert_eq!(cid("disk2s1").as_str(), "disk2s1");
    }

    #[test]
    fn unrecognized_probe_has_no_container() {
        let p = ProbeResult::not_recognized("sdb1");
        assert_eq!(p.container, None);
        assert_eq!(p.result, MatchResult::NotRecognized);
        let u = ProbeResult::usable("sda2", cid("c1"));
        assert_eq!(u.container.as_ref().map(|c| c.as_str()), Some("c1"));
    }

    #[test]
    fn best_mountable_prefers_the_higher_grade() {
        let results = vec![
            ProbeResult::usable_but_limited("a", cid("c1")),
            ProbeResult::usable("b", cid("c2")),
            ProbeResult::recognized("c", cid("c3")),
        ];
        let best = best_mountable(&results).unwrap();
        assert_eq!(best.name, "b");
    }

    #[test]
    fn best_mountable_keeps_the_first_of_equal_grade() {
        let results = vec![
            ProbeResult::usable("first", cid("c1")),
            ProbeResult::usable("second", cid("c2")),
        ];
        assert_eq!(best_mountable(&results).unwrap().name, "first");
    }

    #[test]
    fn best_mountable_is_none_when_nothing_can_mount() {
        let results = vec![
            ProbeResult::not_recognized("x"),
            ProbeResult::recognized("y", cid("c1")),
        ];
        assert!(best_mountable(&results).is_none());
        assert!(best_mountable(&[]).is_none());
    }

    #[test]
    fn container_status_usable_states_and_errors() {
        assert!(ContainerStatus::ready().is_usable());
        assert!(ContainerStatus::active().is_usable());
        assert!(!ContainerStatus::not_ready("still formatting").is_usable());
        assert!(!ContainerStatus::blocked("locked").is_usable());
        assert_eq!(ContainerStatus::blocked("locked").error(), Some("locked"));
        assert_eq!(ContainerStatus::ready().error(), None);
        assert_eq!(
            ContainerStatus::not_ready("x").state(),
            ContainerState::NotReady
        );
    }
}
