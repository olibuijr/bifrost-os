//! Playback arbitration between participants: which registered app may play on an external output, and which may
//! play when the route cannot carry two audio sources at once. A participant is an app identifier. The arbiter
//! only answers "may this participant play now"; it never starts or stops playback itself.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArbiterError {
    /// The participant is not registered with the arbiter.
    Unknown,
}

#[derive(Clone, Debug, Default)]
pub struct Arbiter {
    participants: Vec<String>,
    preferred_external: Option<String>,
    preferred_non_mixable: Option<String>,
}

impl Arbiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a participant. Returns false when the identifier is empty or already registered.
    pub fn register(&mut self, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() || self.is_registered(id) {
            return false;
        }
        self.participants.push(id.to_string());
        true
    }

    /// Removes a participant and any preference that named it. Returns false when it was not registered.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.participants.len();
        self.participants.retain(|p| p != id);
        if self.preferred_external.as_deref() == Some(id) {
            self.preferred_external = None;
        }
        if self.preferred_non_mixable.as_deref() == Some(id) {
            self.preferred_non_mixable = None;
        }
        self.participants.len() != before
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.participants.iter().any(|p| p == id)
    }

    /// Participants in registration order.
    pub fn participants(&self) -> &[String] {
        &self.participants
    }

    /// Sets the participant with priority on external outputs. `None` clears the preference.
    pub fn set_preferred_external(&mut self, id: Option<&str>) -> Result<(), ArbiterError> {
        self.preferred_external = checked(self, id)?;
        Ok(())
    }

    /// Sets the participant with priority when the route cannot mix. `None` clears the preference.
    pub fn set_preferred_non_mixable(&mut self, id: Option<&str>) -> Result<(), ArbiterError> {
        self.preferred_non_mixable = checked(self, id)?;
        Ok(())
    }

    pub fn preferred_external(&self) -> Option<&str> {
        self.preferred_external.as_deref()
    }

    pub fn preferred_non_mixable(&self) -> Option<&str> {
        self.preferred_non_mixable.as_deref()
    }

    /// True when `id` may play on an external output: no preference is set, or `id` is the preference.
    /// An unregistered identifier is never allowed.
    pub fn may_play_external(&self, id: &str) -> bool {
        self.is_registered(id) && self.preferred_external.as_deref().map_or(true, |p| p == id)
    }

    /// True when `id` may play over a route that cannot mix sources. Same rule as `may_play_external`.
    pub fn may_play_non_mixable(&self, id: &str) -> bool {
        self.is_registered(id) && self.preferred_non_mixable.as_deref().map_or(true, |p| p == id)
    }
}

/// Validates an optional identifier against the registered participants. `None` passes through.
fn checked(arbiter: &Arbiter, id: Option<&str>) -> Result<Option<String>, ArbiterError> {
    match id {
        None => Ok(None),
        Some(id) if arbiter.is_registered(id) => Ok(Some(id.to_string())),
        Some(_) => Err(ArbiterError::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_rejects_empty_and_duplicate_identifiers() {
        let mut a = Arbiter::new();
        assert!(a.register("is.olibuijr.music"));
        assert!(!a.register("is.olibuijr.music"));
        assert!(!a.register("   "));
        assert_eq!(a.participants(), &["is.olibuijr.music".to_string()]);
    }

    #[test]
    fn without_a_preference_every_registered_participant_may_play() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        assert!(a.may_play_external("music") && a.may_play_external("podcasts"));
        assert!(a.may_play_non_mixable("podcasts"));
        assert!(!a.may_play_external("unknown"), "unregistered identifiers never play");
    }

    #[test]
    fn a_preferred_participant_wins_the_external_output() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        a.set_preferred_external(Some("podcasts")).unwrap();
        assert!(a.may_play_external("podcasts"));
        assert!(!a.may_play_external("music"));
        assert_eq!(a.preferred_external(), Some("podcasts"));
        assert!(a.may_play_non_mixable("music"), "the non-mixable preference is separate");
    }

    #[test]
    fn preferences_must_name_a_registered_participant() {
        let mut a = Arbiter::new();
        a.register("music");
        assert_eq!(a.set_preferred_non_mixable(Some("ghost")), Err(ArbiterError::Unknown));
        assert_eq!(a.preferred_non_mixable(), None);
        a.set_preferred_non_mixable(Some("music")).unwrap();
        assert!(a.may_play_non_mixable("music"));
        assert!(!a.may_play_non_mixable("other"), "unregistered identifiers never play");
        a.set_preferred_non_mixable(None).unwrap();
        assert_eq!(a.preferred_non_mixable(), None);
    }

    #[test]
    fn unregistering_clears_the_preferences_that_named_it() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        a.set_preferred_external(Some("music")).unwrap();
        a.set_preferred_non_mixable(Some("music")).unwrap();
        assert!(a.unregister("music"));
        assert!(!a.unregister("music"));
        assert_eq!((a.preferred_external(), a.preferred_non_mixable()), (None, None));
        assert!(a.may_play_external("podcasts"));
    }
}
