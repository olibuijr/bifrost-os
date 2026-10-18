//! Audio session enforcement: what the server does with an application's streams because of the session category the
//! application declared. The decisions are pure; the daemon applies the returned [`Action`]s to the application's
//! streams and asks [`Arbiter::check_stream`] before it creates one.
//!
//! Rules (the same arbitration the SDK's `SessionHub` models for apps, here deciding for the server):
//! - A category says which directions a stream may use: `Playback` and `SoloAmbient` and `Ambient` output only,
//!   `Record` input only, `PlayAndRecord` both. A stream the category forbids is refused.
//! - Nothing is active until the application activates its session.
//! - A session *mixes* when its category is `Ambient`, or it asked to mix with others or to duck them. Mixing
//!   sessions never interrupt anyone and are never interrupted.
//! - Activating a session that does not mix interrupts every other active session that does not mix: its output is
//!   paused (`Action::Pause`). Deactivating it (or removing the application) resumes them (`Action::Resume`).
//! - A session that asks to duck others lowers every other active session to `DUCK_GAIN` while it is active.
//! - An application that never declared a session is not in the arbiter: its streams are allowed and unmanaged.

use std::collections::BTreeMap;

/// Which way a stream carries audio, from the application's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Output,
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Ambient,
    SoloAmbient,
    Playback,
    Record,
    PlayAndRecord,
}

impl Category {
    pub fn allows(self, direction: Direction) -> bool {
        match direction {
            Direction::Output => !matches!(self, Category::Record),
            Direction::Input => matches!(self, Category::Record | Category::PlayAndRecord),
        }
    }

    /// The name used in the control protocol and the prefs.
    pub fn name(self) -> &'static str {
        match self {
            Category::Ambient => "ambient",
            Category::SoloAmbient => "solo-ambient",
            Category::Playback => "playback",
            Category::Record => "record",
            Category::PlayAndRecord => "play-and-record",
        }
    }

    pub fn from_name(name: &str) -> Option<Category> {
        [Category::Ambient, Category::SoloAmbient, Category::Playback, Category::Record, Category::PlayAndRecord].into_iter().find(|c| c.name() == name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub mix_with_others: bool,
    pub duck_others: bool,
}

/// The gain other sessions get while a session that ducks them is active.
pub const DUCK_GAIN: f32 = 0.3;

/// What the daemon does to an application's streams.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Stop delivering the application's output (an interruption began).
    Pause(String),
    /// Deliver it again (the interruption ended).
    Resume(String),
    /// Lower the application's output to this gain.
    Duck(String, f32),
    /// Return the application's output to full gain.
    Unduck(String),
}

/// Why a stream was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The session category does not allow this direction.
    Category(Category),
    /// Another session has the audio; the application may stream again when it ends.
    Interrupted,
}

#[derive(Clone, Debug)]
struct Session {
    category: Category,
    options: Options,
    active: bool,
    /// The session whose activation interrupted this one.
    interrupted_by: Option<String>,
    ducked_by: Option<String>,
}

impl Session {
    fn mixes(&self) -> bool {
        self.category == Category::Ambient || self.options.mix_with_others || self.options.duck_others
    }
}

#[derive(Clone, Debug, Default)]
pub struct Arbiter {
    sessions: BTreeMap<String, Session>,
}

impl Arbiter {
    pub fn new() -> Arbiter {
        Arbiter::default()
    }

    /// Declares (or changes) an application's category and options. An active session is arbitrated again.
    pub fn set_category(&mut self, app: &str, category: Category, options: Options) -> Vec<Action> {
        let was_active = match self.sessions.get_mut(app) {
            Some(s) => {
                let was = s.active;
                s.category = category;
                s.options = options;
                was
            }
            None => {
                self.sessions.insert(app.to_string(), Session { category, options, active: false, interrupted_by: None, ducked_by: None });
                false
            }
        };
        if was_active {
            self.rearbitrate(app)
        } else {
            Vec::new()
        }
    }

    pub fn set_active(&mut self, app: &str, active: bool) -> Vec<Action> {
        match (self.sessions.get(app).map(|s| s.active), active) {
            (None, _) => Vec::new(),
            (Some(false), true) => self.activate(app),
            (Some(true), false) => self.deactivate(app),
            _ => Vec::new(),
        }
    }

    /// Forgets an application (it disconnected). What it held is released.
    pub fn remove(&mut self, app: &str) -> Vec<Action> {
        let actions = if self.sessions.get(app).map_or(false, |s| s.active) { self.deactivate(app) } else { Vec::new() };
        self.sessions.remove(app);
        actions
    }

    fn activate(&mut self, app: &str) -> Vec<Action> {
        if let Some(s) = self.sessions.get_mut(app) {
            s.active = true;
        }
        self.rearbitrate(app)
    }

    /// Settles what the active session `app` does to the others, from its present category and options: it
    /// interrupts the active sessions that do not mix (unless it mixes itself), ducks the others if it asked to, and
    /// lets go of whatever it held that its options no longer justify. A session that now mixes is itself resumed.
    fn rearbitrate(&mut self, app: &str) -> Vec<Action> {
        let (mixes, ducks) = match self.sessions.get(app) {
            Some(s) => (s.mixes(), s.options.duck_others),
            None => return Vec::new(),
        };
        let mut actions = Vec::new();
        for (other, s) in self.sessions.iter_mut() {
            if other == app {
                if mixes && s.interrupted_by.take().is_some() {
                    actions.push(Action::Resume(other.clone()));
                }
                continue;
            }
            if !s.active {
                continue;
            }
            if mixes {
                if s.interrupted_by.as_deref() == Some(app) {
                    s.interrupted_by = None;
                    actions.push(Action::Resume(other.clone()));
                }
            } else if !s.mixes() && s.interrupted_by.is_none() {
                s.interrupted_by = Some(app.to_string());
                actions.push(Action::Pause(other.clone()));
            }
            if ducks {
                if s.ducked_by.is_none() {
                    s.ducked_by = Some(app.to_string());
                    actions.push(Action::Duck(other.clone(), DUCK_GAIN));
                }
            } else if s.ducked_by.as_deref() == Some(app) {
                s.ducked_by = None;
                actions.push(Action::Unduck(other.clone()));
            }
        }
        actions
    }

    fn deactivate(&mut self, app: &str) -> Vec<Action> {
        if let Some(s) = self.sessions.get_mut(app) {
            s.active = false;
            s.interrupted_by = None;
            s.ducked_by = None;
        }
        let mut actions = Vec::new();
        for (other, s) in self.sessions.iter_mut() {
            if s.interrupted_by.as_deref() == Some(app) {
                s.interrupted_by = None;
                actions.push(Action::Resume(other.clone()));
            }
            if s.ducked_by.as_deref() == Some(app) {
                s.ducked_by = None;
                actions.push(Action::Unduck(other.clone()));
            }
        }
        actions
    }

    /// Whether a stream of `direction` may be created for `app` now.
    pub fn check_stream(&self, app: &str, direction: Direction) -> Result<(), Denied> {
        let Some(s) = self.sessions.get(app) else { return Ok(()) };
        if !s.category.allows(direction) {
            return Err(Denied::Category(s.category));
        }
        if direction == Direction::Output && s.interrupted_by.is_some() {
            return Err(Denied::Interrupted);
        }
        Ok(())
    }

    /// The gain the mixer applies to the application's output: 0 while interrupted, `DUCK_GAIN` while ducked.
    pub fn gain(&self, app: &str) -> f32 {
        match self.sessions.get(app) {
            Some(s) if s.interrupted_by.is_some() => 0.0,
            Some(s) if s.ducked_by.is_some() => DUCK_GAIN,
            _ => 1.0,
        }
    }

    pub fn is_active(&self, app: &str) -> bool {
        self.sessions.get(app).map_or(false, |s| s.active)
    }

    pub fn category(&self, app: &str) -> Option<Category> {
        self.sessions.get(app).map(|s| s.category)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixing() -> Options {
        Options { mix_with_others: true, duck_others: false }
    }

    #[test]
    fn categories_allow_only_their_directions() {
        let mut a = Arbiter::new();
        a.set_category("player", Category::Playback, Options::default());
        a.set_category("recorder", Category::Record, Options::default());
        a.set_category("call", Category::PlayAndRecord, Options::default());
        assert_eq!(a.check_stream("player", Direction::Output), Ok(()));
        assert_eq!(a.check_stream("player", Direction::Input), Err(Denied::Category(Category::Playback)));
        assert_eq!(a.check_stream("recorder", Direction::Output), Err(Denied::Category(Category::Record)));
        assert_eq!(a.check_stream("recorder", Direction::Input), Ok(()));
        assert_eq!(a.check_stream("call", Direction::Input), Ok(()));
        assert_eq!(a.check_stream("call", Direction::Output), Ok(()));
        assert_eq!(a.check_stream("legacy", Direction::Input), Ok(()), "an application with no session is unmanaged");
    }

    #[test]
    fn a_new_session_interrupts_the_others_that_do_not_mix_and_resumes_them_after() {
        let mut a = Arbiter::new();
        a.set_category("music", Category::Playback, Options::default());
        a.set_category("video", Category::Playback, Options::default());
        a.set_category("game", Category::Ambient, Options::default());
        assert!(a.set_active("music", true).is_empty());
        assert!(a.set_active("game", true).is_empty(), "an ambient session interrupts nobody");
        assert_eq!(a.set_active("video", true), vec![Action::Pause("music".into())]);
        assert_eq!(a.gain("music"), 0.0);
        assert_eq!(a.gain("game"), 1.0, "a mixing session is never interrupted");
        assert_eq!(a.check_stream("music", Direction::Output), Err(Denied::Interrupted));
        assert_eq!(a.set_active("video", false), vec![Action::Resume("music".into())]);
        assert_eq!(a.gain("music"), 1.0);
        assert_eq!(a.check_stream("music", Direction::Output), Ok(()));
    }

    #[test]
    fn a_session_that_mixes_interrupts_nobody() {
        let mut a = Arbiter::new();
        a.set_category("music", Category::Playback, Options::default());
        a.set_category("nav", Category::Playback, mixing());
        a.set_active("music", true);
        assert!(a.set_active("nav", true).is_empty());
        assert_eq!(a.gain("music"), 1.0);
    }

    #[test]
    fn ducking_lowers_the_others_until_the_session_ends() {
        let mut a = Arbiter::new();
        a.set_category("music", Category::Playback, Options::default());
        a.set_category("nav", Category::Playback, Options { mix_with_others: false, duck_others: true });
        a.set_active("music", true);
        assert_eq!(a.set_active("nav", true), vec![Action::Duck("music".into(), DUCK_GAIN)]);
        assert_eq!(a.gain("music"), DUCK_GAIN);
        assert_eq!(a.set_active("nav", false), vec![Action::Unduck("music".into())]);
        assert_eq!(a.gain("music"), 1.0);
    }

    #[test]
    fn removing_an_application_releases_what_it_held() {
        let mut a = Arbiter::new();
        a.set_category("a", Category::Playback, Options::default());
        a.set_category("b", Category::Playback, Options::default());
        a.set_active("a", true);
        a.set_active("b", true);
        assert_eq!(a.gain("a"), 0.0);
        assert_eq!(a.remove("b"), vec![Action::Resume("a".into())]);
        assert_eq!(a.category("b"), None);
        assert_eq!(a.gain("a"), 1.0);
    }

    #[test]
    fn changing_the_category_of_an_active_session_arbitrates_again() {
        let mut a = Arbiter::new();
        a.set_category("a", Category::Playback, Options::default());
        a.set_category("b", Category::Playback, Options::default());
        a.set_active("a", true);
        a.set_active("b", true);
        assert_eq!(a.gain("a"), 0.0);
        // b turns ambient: it stops holding the audio, so a plays again
        let actions = a.set_category("b", Category::Ambient, Options::default());
        assert!(actions.contains(&Action::Resume("a".into())));
        assert_eq!(a.gain("a"), 1.0);
        assert!(a.is_active("b"));
    }

    #[test]
    fn activating_twice_and_unknown_applications_do_nothing() {
        let mut a = Arbiter::new();
        a.set_category("a", Category::Playback, Options::default());
        a.set_category("b", Category::Playback, Options::default());
        a.set_active("a", true);
        assert_eq!(a.set_active("b", true).len(), 1);
        assert!(a.set_active("b", true).is_empty());
        assert!(a.set_active("ghost", true).is_empty());
        assert!(a.set_active("a", false).is_empty());
        assert_eq!(a.gain("a"), 1.0, "a session that ended is no longer interrupted");
        assert_eq!(Category::from_name("play-and-record"), Some(Category::PlayAndRecord));
        assert_eq!(Category::from_name("loud"), None);
    }
}
