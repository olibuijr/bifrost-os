//! Audio sessions (`AVAudioSession` parity): each app declares a category, a mode and options, activates its session,
//! and is told when another session takes the audio (`interruptionNotification`: began, then ended with
//! `shouldResume`) or when the output route changes (`routeChangeNotification`).
//!
//! Rules of this model:
//! - Nothing is active until the app calls `set_active(client, true)` (apps never take audio on launch or focus).
//! - A session mixes when its category is `Ambient`, or it set `mix_with_others` or `duck_others`. Mixing sessions
//!   never interrupt and are never interrupted. `duck_others` additionally marks the others as ducked.
//! - Activating a non-mixing session interrupts every other active non-mixing session (event `Began`).
//! - Deactivating a session ends the interruptions it caused (event `Ended { should_resume: true }`). A resumed app
//!   must call `set_active(client, true)` again; the model does not reactivate it.
//! - `set_route` posts a route-change event to every client, with the previous route.
//!
//! This is the pure model. The arbitration is meant to live in `audiod` and is not wired to it yet, so the matrix rows
//! that use it are partial.

use std::collections::BTreeMap;

/// `AVAudioSession.Category`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Ambient,
    SoloAmbient,
    Playback,
    Record,
    PlayAndRecord,
}

/// `AVAudioSession.Mode` (the subset that matters for arbitration and routing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Default,
    VoiceChat,
    MoviePlayback,
    SpokenAudio,
    Measurement,
}

/// `AVAudioSession.CategoryOptions` (the subset modelled).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub mix_with_others: bool,
    pub duck_others: bool,
}

/// `AVAudioSession.InterruptionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptionType {
    Began,
    Ended,
}

/// `AVAudioSession.RouteChangeReason` (raw values as in the framework).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteChangeReason {
    Unknown = 0,
    NewDeviceAvailable = 1,
    OldDeviceUnavailable = 2,
    CategoryChange = 3,
    Override = 4,
    WakeFromSleep = 6,
    NoSuitableRouteForCategory = 7,
    RouteConfigurationChange = 8,
}

/// The output route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Speaker,
    Headphones,
}

/// A notification waiting for one client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// `interruptionNotification`; `should_resume` is only meaningful for `Ended`.
    Interruption { kind: InterruptionType, should_resume: bool },
    /// `routeChangeNotification` with the route before the change.
    RouteChange { reason: RouteChangeReason, previous: Route },
    /// `mediaServicesWereResetNotification`: the audio service restarted; the app must rebuild its audio objects and
    /// activate its session again.
    MediaServicesWereReset,
}

/// `AVAudioSession.Port` (the kinds of port modelled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortType {
    BuiltInMic,
    HeadsetMic,
    BuiltInSpeaker,
    Headphones,
    UsbAudio,
}

/// `AVAudioSessionPortDescription`: one input or output port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortDescription {
    pub port_type: PortType,
    pub port_name: String,
    pub uid: String,
}

impl PortDescription {
    pub fn new(port_type: PortType, port_name: &str, uid: &str) -> Self {
        PortDescription { port_type, port_name: port_name.to_string(), uid: uid.to_string() }
    }
}

/// `AVAudioSessionRouteDescription`: the active input and output ports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteDescription {
    pub inputs: Vec<PortDescription>,
    pub outputs: Vec<PortDescription>,
}

/// `AVAudioSession.PortOverride`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortOverride {
    None,
    Speaker,
}

/// Why a call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    UnknownClient,
    /// `setPreferredInput` named a port that is not in `availableInputs`.
    UnknownPort,
}

#[derive(Clone, Debug)]
struct Session {
    category: Category,
    mode: Mode,
    options: Options,
    active: bool,
    ducked: bool,
    /// The client whose activation interrupted this session, while it is interrupted.
    interrupted_by: Option<String>,
    output_override: PortOverride,
    events: Vec<SessionEvent>,
}

impl Session {
    fn mixes(&self) -> bool {
        self.category == Category::Ambient || self.options.mix_with_others || self.options.duck_others
    }
}

/// All sessions on the machine plus the current route.
#[derive(Clone, Debug)]
pub struct SessionHub {
    sessions: BTreeMap<String, Session>,
    route: Route,
    inputs: Vec<PortDescription>,
    preferred_input: Option<String>,
}

impl SessionHub {
    pub fn new() -> Self {
        SessionHub {
            sessions: BTreeMap::new(),
            route: Route::Speaker,
            inputs: vec![PortDescription::new(PortType::BuiltInMic, "Microphone", "builtin-mic")],
            preferred_input: None,
        }
    }

    /// Registers a client with `SoloAmbient`, the default mode and no options, inactive.
    pub fn register(&mut self, client: &str) {
        self.sessions.entry(client.to_string()).or_insert(Session {
            category: Category::SoloAmbient,
            mode: Mode::Default,
            options: Options::default(),
            active: false,
            ducked: false,
            interrupted_by: None,
            output_override: PortOverride::None,
            events: Vec::new(),
        });
    }

    /// `setCategory(_:mode:options:)`. The client gets a `CategoryChange` route event when its category changed.
    pub fn set_category(&mut self, client: &str, category: Category, mode: Mode, options: Options) -> Result<(), SessionError> {
        let route = self.route;
        let s = self.sessions.get_mut(client).ok_or(SessionError::UnknownClient)?;
        let changed = s.category != category;
        s.category = category;
        s.mode = mode;
        s.options = options;
        if changed {
            s.events.push(SessionEvent::RouteChange { reason: RouteChangeReason::CategoryChange, previous: route });
        }
        Ok(())
    }

    pub fn category(&self, client: &str) -> Option<Category> {
        self.sessions.get(client).map(|s| s.category)
    }

    pub fn mode(&self, client: &str) -> Option<Mode> {
        self.sessions.get(client).map(|s| s.mode)
    }

    pub fn options(&self, client: &str) -> Option<Options> {
        self.sessions.get(client).map(|s| s.options)
    }

    /// `setActive(_:)`. Activating a non-mixing session interrupts the other active non-mixing sessions; deactivating
    /// ends the interruptions this client caused.
    pub fn set_active(&mut self, client: &str, active: bool) -> Result<(), SessionError> {
        if !self.sessions.contains_key(client) {
            return Err(SessionError::UnknownClient);
        }
        if active {
            self.activate(client);
        } else {
            self.deactivate(client);
        }
        Ok(())
    }

    fn activate(&mut self, client: &str) {
        let (mixes, ducks) = match self.sessions.get_mut(client) {
            Some(s) => {
                s.active = true;
                s.interrupted_by = None;
                (s.mixes(), s.options.duck_others)
            }
            None => return,
        };
        for (id, other) in self.sessions.iter_mut() {
            if id == client || !other.active {
                continue;
            }
            if ducks {
                other.ducked = true;
            }
            if !mixes && !other.mixes() {
                other.active = false;
                other.interrupted_by = Some(client.to_string());
                other.events.push(SessionEvent::Interruption { kind: InterruptionType::Began, should_resume: false });
            }
        }
    }

    fn deactivate(&mut self, client: &str) {
        let (was_ducking, own_interrupter) = match self.sessions.get_mut(client) {
            Some(s) => {
                s.active = false;
                (s.options.duck_others, s.interrupted_by.take())
            }
            None => return,
        };
        let still_ducking = self.sessions.iter().any(|(id, o)| id != client && o.active && o.options.duck_others);
        if was_ducking && !still_ducking {
            for s in self.sessions.values_mut() {
                s.ducked = false;
            }
        }
        for (id, other) in self.sessions.iter_mut() {
            if id == client || other.interrupted_by.as_deref() != Some(client) {
                continue;
            }
            match &own_interrupter {
                // The client was itself interrupted: its victims now wait for whoever holds the audio.
                Some(holder) => other.interrupted_by = Some(holder.clone()),
                None => {
                    other.interrupted_by = None;
                    other.events.push(SessionEvent::Interruption { kind: InterruptionType::Ended, should_resume: true });
                }
            }
        }
    }

    /// Whether the client's session is active now (an interrupted session is not).
    pub fn is_active(&self, client: &str) -> bool {
        self.sessions.get(client).map_or(false, |s| s.active)
    }

    /// Whether the session is interrupted and waiting for the interruption to end.
    pub fn is_interrupted(&self, client: &str) -> bool {
        self.sessions.get(client).map_or(false, |s| s.interrupted_by.is_some())
    }

    /// Whether another session asked to duck this client's audio.
    pub fn is_ducked(&self, client: &str) -> bool {
        self.sessions.get(client).map_or(false, |s| s.ducked)
    }

    pub fn route(&self) -> Route {
        self.route
    }

    /// Changes the output route and tells every client. Setting the current route does nothing.
    pub fn set_route(&mut self, route: Route) {
        if route == self.route {
            return;
        }
        let previous = self.route;
        let reason = match route {
            Route::Headphones => RouteChangeReason::NewDeviceAvailable,
            Route::Speaker => RouteChangeReason::OldDeviceUnavailable,
        };
        self.route = route;
        for s in self.sessions.values_mut() {
            s.events.push(SessionEvent::RouteChange { reason, previous });
        }
    }

    /// `availableInputs`.
    pub fn available_inputs(&self) -> &[PortDescription] {
        &self.inputs
    }

    /// Replaces the input ports (a device was plugged or unplugged). A preferred input that disappeared is cleared.
    pub fn set_available_inputs(&mut self, inputs: Vec<PortDescription>) {
        self.inputs = inputs;
        if let Some(uid) = &self.preferred_input {
            if !self.inputs.iter().any(|p| &p.uid == uid) {
                self.preferred_input = None;
            }
        }
    }

    /// `setPreferredInput(_:)`. `None` returns to the default input. The port must be in `availableInputs`.
    pub fn set_preferred_input(&mut self, uid: Option<&str>) -> Result<(), SessionError> {
        if let Some(uid) = uid {
            if !self.inputs.iter().any(|p| p.uid == uid) {
                return Err(SessionError::UnknownPort);
            }
        }
        self.preferred_input = uid.map(str::to_string);
        Ok(())
    }

    /// `preferredInput`.
    pub fn preferred_input(&self) -> Option<&PortDescription> {
        let uid = self.preferred_input.as_deref()?;
        self.inputs.iter().find(|p| p.uid == uid)
    }

    /// `overrideOutputAudioPort(_:)`. Forcing the speaker posts an `Override` route change to the client; clearing the
    /// override posts `RouteConfigurationChange`.
    pub fn override_output_port(&mut self, client: &str, port: PortOverride) -> Result<(), SessionError> {
        let previous = self.effective_route(client)?;
        let s = self.sessions.get_mut(client).ok_or(SessionError::UnknownClient)?;
        if s.output_override == port {
            return Ok(());
        }
        s.output_override = port;
        let reason = if port == PortOverride::Speaker { RouteChangeReason::Override } else { RouteChangeReason::RouteConfigurationChange };
        s.events.push(SessionEvent::RouteChange { reason, previous });
        Ok(())
    }

    fn effective_route(&self, client: &str) -> Result<Route, SessionError> {
        let s = self.sessions.get(client).ok_or(SessionError::UnknownClient)?;
        Ok(if s.output_override == PortOverride::Speaker { Route::Speaker } else { self.route })
    }

    /// `currentRoute` for a client: the output follows the hub route unless the client forced the speaker; the input is
    /// the preferred input when set, else the first available one.
    pub fn current_route(&self, client: &str) -> Result<RouteDescription, SessionError> {
        let output = match self.effective_route(client)? {
            Route::Speaker => PortDescription::new(PortType::BuiltInSpeaker, "Speaker", "builtin-speaker"),
            Route::Headphones => PortDescription::new(PortType::Headphones, "Headphones", "headphones"),
        };
        let input = self.preferred_input().or_else(|| self.inputs.first()).cloned();
        Ok(RouteDescription { inputs: input.into_iter().collect(), outputs: vec![output] })
    }

    /// The audio service restarted: every client is told to rebuild its audio state, and every session becomes inactive.
    pub fn media_services_were_reset(&mut self) {
        for s in self.sessions.values_mut() {
            s.active = false;
            s.ducked = false;
            s.interrupted_by = None;
            s.events.push(SessionEvent::MediaServicesWereReset);
        }
    }

    /// Takes the client's waiting notifications, oldest first.
    pub fn take_events(&mut self, client: &str) -> Vec<SessionEvent> {
        self.sessions.get_mut(client).map(|s| std::mem::take(&mut s.events)).unwrap_or_default()
    }

    /// Removes a client (the app quit). Its interruptions end as if it deactivated.
    pub fn unregister(&mut self, client: &str) {
        if self.sessions.contains_key(client) {
            self.deactivate(client);
            self.sessions.remove(client);
        }
    }
}

impl Default for SessionHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEGAN: SessionEvent = SessionEvent::Interruption { kind: InterruptionType::Began, should_resume: false };
    const ENDED: SessionEvent = SessionEvent::Interruption { kind: InterruptionType::Ended, should_resume: true };

    fn hub(ids: &[&str]) -> SessionHub {
        let mut h = SessionHub::new();
        for id in ids {
            h.register(id);
        }
        h
    }

    fn playback(h: &mut SessionHub, id: &str) {
        h.set_category(id, Category::Playback, Mode::Default, Options::default()).unwrap();
    }

    fn mixing(h: &mut SessionHub, id: &str) {
        h.set_category(id, Category::Playback, Mode::Default, Options { mix_with_others: true, duck_others: false }).unwrap();
    }

    #[test]
    fn nothing_is_active_until_asked() {
        let h = hub(&["a"]);
        assert!(!h.is_active("a"));
    }

    #[test]
    fn category_and_mode_are_kept() {
        let mut h = hub(&["a"]);
        h.set_category("a", Category::PlayAndRecord, Mode::VoiceChat, Options { mix_with_others: true, duck_others: false }).unwrap();
        assert_eq!(h.category("a"), Some(Category::PlayAndRecord));
        assert_eq!(h.mode("a"), Some(Mode::VoiceChat));
        assert!(h.options("a").unwrap().mix_with_others);
    }

    #[test]
    fn unknown_client_is_refused() {
        let mut h = hub(&[]);
        assert_eq!(h.set_active("x", true), Err(SessionError::UnknownClient));
        assert_eq!(h.set_category("x", Category::Playback, Mode::Default, Options::default()), Err(SessionError::UnknownClient));
    }

    #[test]
    fn second_exclusive_session_interrupts_the_first() {
        let mut h = hub(&["a", "b"]);
        playback(&mut h, "a");
        playback(&mut h, "b");
        h.take_events("a");
        h.set_active("a", true).unwrap();
        h.set_active("b", true).unwrap();
        assert!(!h.is_active("a") && h.is_interrupted("a"));
        assert!(h.is_active("b"));
        assert_eq!(h.take_events("a"), vec![BEGAN]);
    }

    #[test]
    fn deactivating_the_interrupter_ends_with_should_resume() {
        let mut h = hub(&["a", "b"]);
        playback(&mut h, "a");
        playback(&mut h, "b");
        h.set_active("a", true).unwrap();
        h.set_active("b", true).unwrap();
        h.take_events("a");
        h.set_active("b", false).unwrap();
        assert_eq!(h.take_events("a"), vec![ENDED]);
        assert!(!h.is_interrupted("a"));
        assert!(!h.is_active("a"), "the app must reactivate itself");
        h.set_active("a", true).unwrap();
        assert!(h.is_active("a"));
    }

    #[test]
    fn mix_with_others_never_interrupts_or_is_interrupted() {
        let mut h = hub(&["a", "m"]);
        playback(&mut h, "a");
        mixing(&mut h, "m");
        h.set_active("a", true).unwrap();
        h.set_active("m", true).unwrap();
        assert!(h.is_active("a") && h.is_active("m"));
        let mut h2 = hub(&["a", "m"]);
        playback(&mut h2, "a");
        mixing(&mut h2, "m");
        h2.set_active("m", true).unwrap();
        h2.set_active("a", true).unwrap();
        assert!(h2.is_active("m") && h2.is_active("a"));
    }

    #[test]
    fn ambient_mixes() {
        let mut h = hub(&["a", "x"]);
        playback(&mut h, "a");
        h.set_category("x", Category::Ambient, Mode::Default, Options::default()).unwrap();
        h.set_active("a", true).unwrap();
        h.set_active("x", true).unwrap();
        assert!(h.is_active("a") && h.is_active("x"));
    }

    #[test]
    fn duck_others_marks_and_clears_ducking_without_interrupting() {
        let mut h = hub(&["a", "d"]);
        playback(&mut h, "a");
        h.set_category("d", Category::Playback, Mode::SpokenAudio, Options { mix_with_others: false, duck_others: true }).unwrap();
        h.set_active("a", true).unwrap();
        h.set_active("d", true).unwrap();
        assert!(h.is_active("a") && h.is_ducked("a"));
        assert!(!h.take_events("a").contains(&BEGAN));
        h.set_active("d", false).unwrap();
        assert!(!h.is_ducked("a"));
    }

    #[test]
    fn chain_of_interruptions_resumes_in_order() {
        let mut h = hub(&["a", "b", "c"]);
        for id in ["a", "b", "c"] {
            playback(&mut h, id);
        }
        h.set_active("a", true).unwrap();
        h.set_active("b", true).unwrap();
        h.set_active("c", true).unwrap();
        h.take_events("a");
        h.take_events("b");
        h.set_active("c", false).unwrap();
        assert_eq!(h.take_events("b"), vec![ENDED]);
        assert!(h.take_events("a").is_empty(), "a still waits for b");
        h.set_active("b", true).unwrap();
        h.set_active("b", false).unwrap();
        assert_eq!(h.take_events("a"), vec![ENDED]);
    }

    #[test]
    fn interrupted_session_that_deactivates_hands_its_victims_to_the_holder() {
        let mut h = hub(&["a", "b", "c"]);
        for id in ["a", "b", "c"] {
            playback(&mut h, id);
        }
        h.set_active("a", true).unwrap();
        h.set_active("b", true).unwrap();
        h.set_active("c", true).unwrap();
        h.take_events("a");
        h.set_active("b", false).unwrap();
        assert!(h.take_events("a").is_empty());
        assert!(h.is_interrupted("a"));
        h.set_active("c", false).unwrap();
        assert_eq!(h.take_events("a"), vec![ENDED]);
    }

    #[test]
    fn unregister_ends_interruptions() {
        let mut h = hub(&["a", "b"]);
        playback(&mut h, "a");
        playback(&mut h, "b");
        h.set_active("a", true).unwrap();
        h.set_active("b", true).unwrap();
        h.take_events("a");
        h.unregister("b");
        assert_eq!(h.take_events("a"), vec![ENDED]);
    }

    #[test]
    fn route_change_reaches_every_client_with_the_previous_route() {
        let mut h = hub(&["a", "b"]);
        h.set_route(Route::Headphones);
        let want = SessionEvent::RouteChange { reason: RouteChangeReason::NewDeviceAvailable, previous: Route::Speaker };
        assert_eq!(h.take_events("a"), vec![want]);
        assert_eq!(h.take_events("b"), vec![want]);
        h.set_route(Route::Speaker);
        assert_eq!(
            h.take_events("a"),
            vec![SessionEvent::RouteChange { reason: RouteChangeReason::OldDeviceUnavailable, previous: Route::Headphones }]
        );
        h.set_route(Route::Speaker);
        assert!(h.take_events("a").is_empty());
    }

    #[test]
    fn category_change_posts_a_route_event_to_that_client_only() {
        let mut h = hub(&["a", "b"]);
        playback(&mut h, "a");
        assert_eq!(
            h.take_events("a"),
            vec![SessionEvent::RouteChange { reason: RouteChangeReason::CategoryChange, previous: Route::Speaker }]
        );
        assert!(h.take_events("b").is_empty());
        playback(&mut h, "a");
        assert!(h.take_events("a").is_empty());
    }

    #[test]
    fn route_change_raw_values_match_the_framework() {
        assert_eq!(RouteChangeReason::NewDeviceAvailable as u8, 1);
        assert_eq!(RouteChangeReason::NoSuitableRouteForCategory as u8, 7);
    }

    #[test]
    fn current_route_follows_the_hub_route() {
        let mut h = hub(&["a"]);
        assert_eq!(h.current_route("a").unwrap().outputs[0].port_type, PortType::BuiltInSpeaker);
        h.set_route(Route::Headphones);
        let r = h.current_route("a").unwrap();
        assert_eq!(r.outputs[0].port_type, PortType::Headphones);
        assert_eq!(r.inputs[0].port_type, PortType::BuiltInMic);
        assert_eq!(h.current_route("x"), Err(SessionError::UnknownClient));
    }

    #[test]
    fn preferred_input_must_be_available_and_is_dropped_when_it_goes() {
        let mut h = hub(&["a"]);
        let usb = PortDescription::new(PortType::UsbAudio, "USB", "usb-1");
        assert_eq!(h.set_preferred_input(Some("usb-1")), Err(SessionError::UnknownPort));
        let mic = h.available_inputs()[0].clone();
        h.set_available_inputs(vec![mic.clone(), usb.clone()]);
        h.set_preferred_input(Some("usb-1")).unwrap();
        assert_eq!(h.preferred_input(), Some(&usb));
        assert_eq!(h.current_route("a").unwrap().inputs, vec![usb]);
        h.set_available_inputs(vec![mic]);
        assert_eq!(h.preferred_input(), None);
        h.set_preferred_input(None).unwrap();
    }

    #[test]
    fn overriding_to_the_speaker_posts_an_override_event_for_that_client_only() {
        let mut h = hub(&["a", "b"]);
        h.set_route(Route::Headphones);
        h.take_events("a");
        h.take_events("b");
        h.override_output_port("a", PortOverride::Speaker).unwrap();
        assert_eq!(h.current_route("a").unwrap().outputs[0].port_type, PortType::BuiltInSpeaker);
        assert_eq!(h.current_route("b").unwrap().outputs[0].port_type, PortType::Headphones);
        assert_eq!(
            h.take_events("a"),
            vec![SessionEvent::RouteChange { reason: RouteChangeReason::Override, previous: Route::Headphones }]
        );
        assert!(h.take_events("b").is_empty());
        h.override_output_port("a", PortOverride::Speaker).unwrap();
        assert!(h.take_events("a").is_empty(), "repeating the override is not a change");
        h.override_output_port("a", PortOverride::None).unwrap();
        assert_eq!(h.current_route("a").unwrap().outputs[0].port_type, PortType::Headphones);
    }

    #[test]
    fn media_services_reset_notifies_everyone_and_deactivates() {
        let mut h = hub(&["a", "b"]);
        playback(&mut h, "a");
        h.set_active("a", true).unwrap();
        h.media_services_were_reset();
        assert!(!h.is_active("a"));
        assert_eq!(h.take_events("a").last(), Some(&SessionEvent::MediaServicesWereReset));
        assert_eq!(h.take_events("b"), vec![SessionEvent::MediaServicesWereReset]);
    }
}
