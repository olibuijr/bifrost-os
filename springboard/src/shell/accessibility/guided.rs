//! Guided Access (single-app mode): a supervised app asks the shell for a session, the user turns the feature on in
//! Settings > Accessibility, and the session holds one app with the restrictions that app declared. The app hears the answer
//! and every later change as `ToApp::GuidedAccessState` and `ToApp::GuidedRestriction`.
//!
//! [`Guided`] is the session model: it refuses a second app, cleans what an app declares, and records each restriction's
//! state. The `Shell` glue at the bottom reads the requests the apps posted (`AppHost::guided_requests`) on each tick, answers
//! them, and ends a session whose app has exited. Keeping Home, the Dock and other windows out of the way while a session runs
//! is left to the desk and the session panel; this module is the state those parts read.

use super::AccessibilitySettings;
use crate::shell::Shell;
use bifrost_proto::guided::MAX_RESTRICTIONS;
use bifrost_proto::{GuidedRestriction, ToApp};

/// A running session: the app that holds it and the restrictions it declared, with the states the user has set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub app: String,
    pub restrictions: Vec<GuidedRestriction>,
}

/// The Guided Access state the shell keeps. At most one session runs at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Guided {
    session: Option<Session>,
}

/// Cleans the restrictions an app declares: an empty identifier is dropped, a repeated identifier keeps its first entry, and
/// the list is capped at `MAX_RESTRICTIONS`.
pub fn clean(list: Vec<GuidedRestriction>) -> Vec<GuidedRestriction> {
    let mut out: Vec<GuidedRestriction> = Vec::new();
    for g in list {
        if out.len() >= MAX_RESTRICTIONS {
            break;
        }
        if g.id.is_empty() || out.iter().any(|o| o.id == g.id) {
            continue;
        }
        out.push(g);
    }
    out
}

impl Guided {
    /// The app that holds the session, if one runs.
    pub fn app(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.app.as_str())
    }

    #[allow(dead_code)] // read by the session options, which are not built yet
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// Starts a session for `app` with these restrictions. Refused when Guided Access is off in Settings, or when another app
    /// holds the session. The app that already holds it replaces its restrictions.
    pub fn start(&mut self, app: &str, restrictions: Vec<GuidedRestriction>, enabled: bool) -> Result<(), &'static str> {
        if !enabled {
            return Err("Guided Access is off");
        }
        if self.app().is_some_and(|holder| holder != app) {
            return Err("another app is in Guided Access");
        }
        self.session = Some(Session { app: app.to_string(), restrictions: clean(restrictions) });
        Ok(())
    }

    /// Ends the session `app` holds. False when `app` holds none.
    pub fn end(&mut self, app: &str) -> bool {
        if self.app() == Some(app) {
            self.session = None;
            return true;
        }
        false
    }

    /// Sets one restriction's state. Answers the app to tell when the state changed; None for no session, an unknown
    /// identifier or an unchanged state.
    #[allow(dead_code)] // called by the session options, which are not built yet
    pub fn set_restriction(&mut self, id: &str, allowed: bool) -> Option<String> {
        let s = self.session.as_mut()?;
        let r = s.restrictions.iter_mut().find(|r| r.id == id)?;
        if r.allowed == allowed {
            return None;
        }
        r.allowed = allowed;
        Some(s.app.clone())
    }

    /// Whether the restriction `id` permits what it guards. With no session, or an identifier the session does not declare,
    /// everything is permitted.
    #[allow(dead_code)] // read by the desk and the Dock once they honour a session
    pub fn permitted(&self, id: &str) -> bool {
        self.session.as_ref().and_then(|s| s.restrictions.iter().find(|r| r.id == id)).map_or(true, |r| r.allowed)
    }
}

impl Shell {
    /// Settings > Accessibility > Guided Access, read the way the Settings page reads it.
    fn guided_enabled(&self) -> bool {
        self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs), |st| st.accessibility().clone()).guided_access
    }

    /// Answers the Guided Access requests the apps posted since the last tick, and ends a session whose app has exited.
    pub(in crate::shell) fn guided_tick(&mut self) {
        let enabled = self.guided_enabled();
        for host in self.ext.iter_mut().flatten() {
            for (on, restrictions) in std::mem::take(&mut host.guided_requests) {
                let active = if on {
                    self.guided.start(&host.id, restrictions, enabled).is_ok()
                } else {
                    self.guided.end(&host.id);
                    false
                };
                host.send(&ToApp::GuidedAccessState { active });
            }
        }
        if let Some(app) = self.guided.app().map(str::to_string) {
            if !self.ext.iter().flatten().any(|h| h.id == app && h.running()) {
                self.guided.end(&app);
            }
        }
    }

    /// The user allowed or denied one restriction in the session options. The holding app hears it as `ToApp::GuidedRestriction`.
    #[allow(dead_code)] // called by the session options, which are not built yet
    pub fn guided_set_restriction(&mut self, id: &str, allowed: bool) {
        if let Some(app) = self.guided.set_restriction(id, allowed) {
            if let Some(host) = self.ext.iter_mut().flatten().find(|h| h.id == app) {
                host.send(&ToApp::GuidedRestriction { id: id.to_string(), allowed });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: &str) -> GuidedRestriction {
        GuidedRestriction::new(id, id)
    }

    #[test]
    fn clean_drops_blank_and_repeated_identifiers() {
        let out = clean(vec![g("touch"), g(""), g("sleep").set_allowed(false), g("touch").set_allowed(false)]);
        assert_eq!(out, vec![g("touch"), g("sleep").set_allowed(false)], "the first entry of a repeated identifier wins");
        assert_eq!(clean((0..40).map(|i| g(&format!("r{i}"))).collect()).len(), MAX_RESTRICTIONS);
    }

    #[test]
    fn a_session_needs_the_setting_on() {
        let mut m = Guided::default();
        assert_eq!(m.start("notes", vec![g("touch")], false), Err("Guided Access is off"));
        assert_eq!(m.app(), None);
        assert_eq!(m.start("notes", vec![g("touch")], true), Ok(()));
        assert_eq!(m.app(), Some("notes"));
    }

    #[test]
    fn one_app_at_a_time_and_the_holder_may_restart() {
        let mut m = Guided::default();
        m.start("notes", vec![g("touch")], true).unwrap();
        assert_eq!(m.start("mail", vec![], true), Err("another app is in Guided Access"));
        m.start("notes", vec![g("sleep")], true).unwrap();
        assert_eq!(m.session().unwrap().restrictions, vec![g("sleep")], "the holder's request replaces its restrictions");
    }

    #[test]
    fn only_the_holder_ends_its_session() {
        let mut m = Guided::default();
        m.start("notes", vec![], true).unwrap();
        assert!(!m.end("mail"));
        assert_eq!(m.app(), Some("notes"));
        assert!(m.end("notes"));
        assert_eq!(m.app(), None);
    }

    #[test]
    fn restriction_states_are_recorded_and_reported_once() {
        let mut m = Guided::default();
        m.start("notes", vec![g("touch"), g("sleep")], true).unwrap();
        assert!(m.permitted("touch") && m.permitted("unknown"));
        assert_eq!(m.set_restriction("sleep", false), Some("notes".to_string()));
        assert_eq!(m.set_restriction("sleep", false), None, "an unchanged state is not reported again");
        assert_eq!(m.set_restriction("missing", false), None);
        assert!(!m.permitted("sleep"));
        assert!(m.permitted("touch"));
    }

    #[test]
    fn no_session_permits_everything() {
        let mut m = Guided::default();
        assert!(m.permitted("touch"));
        assert_eq!(m.set_restriction("touch", false), None);
        m.start("notes", vec![g("touch")], true).unwrap();
        assert!(m.end("notes"));
        assert!(m.permitted("touch"));
    }
}
