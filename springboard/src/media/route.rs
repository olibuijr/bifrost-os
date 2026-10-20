//! Audio routing: which output the sound goes to (speaker or headphones), the headset jack
//! state, each route's remembered volume and the headphone safety cap. The sound server does
//! the real routing on a desktop; `plan` names the card's own headphone and speaker output
//! switches, found by name at run time, for a caller that talks to the card's mixer (`bifrost_audio_alsa::ctl`).

use super::volume::Volume;

/// Which output the sound goes to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Route {
    #[default]
    Speaker,
    Headphones,
}

impl Route {
    /// The name the HUD and Control Center show.
    pub fn label(self) -> &'static str {
        match self {
            Route::Speaker => "Speaker",
            Route::Headphones => "Headphones",
        }
    }

    /// The settings.ini key holding this route's remembered volume.
    pub fn pref_key(self) -> &'static str {
        match self {
            Route::Speaker => PREF_SPEAKER,
            Route::Headphones => PREF_HEADPHONES,
        }
    }
}

/// The name of the connected Bluetooth audio device, for Now Playing's route
/// indicator ("Buds" instead of "Speaker"); None when nothing is connected.
/// Display only: the speaker path and the mixer routes are not touched.
pub fn bluetooth_sink(bt: &crate::bare::bluetooth::Status) -> Option<&str> {
    bt.audio_sink()
}

pub const PREF_SPEAKER: &str = "volume_speaker";
pub const PREF_HEADPHONES: &str = "volume_headphones";
/// Set once the user has raised the headphone volume above the safety cap.
pub const PREF_CAP_LIFTED: &str = "headphone_cap_lifted";
/// The "Headphone safety" ceiling applied on plug-in until lifted.
pub const HEADPHONE_CAP: f32 = 0.85;

/// input_event switch codes (linux/input-event-codes.h).
pub const SW_HEADPHONE_INSERT: u16 = 0x02;
pub const SW_MICROPHONE_INSERT: u16 = 0x04;

/// Name fragments of the headset input device, lowercase.
pub const JACK_DEVICE_HINTS: &[&str] = &["headset", "jack"];

// --- finding the codec's own output switches by name ---

/// What the runtime scan knows about a control; `items` is empty for a
/// plain switch and holds the item names of an enumerated one.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub items: Vec<String>,
}

/// The one table of name knowledge. A control is an output switch when it
/// has one of these tokens (names split at anything but letters and
/// digits, lowercase) and none of `NOT_A_SWITCH`.
const HEADPHONE_TOKENS: &[&str] = &["headphone", "headphones", "hp", "hpl", "hpr"];
const EARPIECE_TOKENS: &[&str] = &["earpiece", "ear"];
/// codec speaker amp switches; routed off for headphones
const CODEC_SPEAKER_TOKENS: &[&str] = &["speaker", "speaker1"];
/// gains, mutes, mixers and capture paths are never switched by a route
const NOT_A_SWITCH: &[&str] = &["volume", "gain", "dg", "mdg", "mute", "mixer", "mic", "capture", "record", "set", "sel"];
const SWITCH_WORDS: &[&str] = &["function", "switch", "enable"];

fn tokens(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase()).collect()
}

fn is_output_switch(name: &str, kinds: &[&str]) -> bool {
    let t = tokens(name);
    t.iter().any(|x| kinds.contains(&x.as_str())) && !t.iter().any(|x| NOT_A_SWITCH.contains(&x.as_str())) && t.iter().any(|x| SWITCH_WORDS.contains(&x.as_str()))
}

/// The value that turns `c` on or off, None when it has no such item.
fn switch_value(c: &Candidate, on: bool) -> Option<String> {
    if c.items.is_empty() {
        return Some(if on { "1" } else { "0" }.to_string());
    }
    let words: &[&str] = if on { &["on", "enable", "enabled"] } else { &["off", "disable", "disabled"] };
    c.items.iter().find(|i| words.contains(&i.to_ascii_lowercase().as_str())).cloned()
}

/// The (control, value) pairs that put the sound on `route`, in order,
/// given the controls the card really has: missing ones are skipped. The
/// other output goes off first so two paths never play at once; the
/// earpiece is off on both routes.
pub fn plan(route: Route, controls: &[Candidate]) -> Vec<(String, String)> {
    let switches = |kinds: &'static [&'static str], on: bool| -> Vec<(String, String)> {
        controls.iter().filter(|c| is_output_switch(&c.name, kinds)).filter_map(|c| switch_value(c, on).map(|v| (c.name.clone(), v))).collect()
    };
    let mut out = Vec::new();
    match route {
        Route::Speaker => {
            out.extend(switches(HEADPHONE_TOKENS, false));
            out.extend(switches(EARPIECE_TOKENS, false));
        }
        Route::Headphones => {
            out.extend(switches(CODEC_SPEAKER_TOKENS, false));
            out.extend(switches(EARPIECE_TOKENS, false));
            out.extend(switches(HEADPHONE_TOKENS, true));
        }
    }
    out
}

// --- the jack ---

/// The headset switch state; the jack counts as plugged when either the
/// headphone or the microphone-insert switch is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Jack {
    headphone: bool,
    microphone: bool,
}

impl Jack {
    /// From the EVIOCGSW bitmap read at start-up.
    pub fn from_bits(bits: u32) -> Jack {
        Jack { headphone: bits >> SW_HEADPHONE_INSERT & 1 != 0, microphone: bits >> SW_MICROPHONE_INSERT & 1 != 0 }
    }

    pub fn plugged(self) -> bool {
        self.headphone || self.microphone
    }

    /// Feeds a switch event; the new plugged state when it changed.
    pub fn feed(&mut self, code: u16, on: bool) -> Option<bool> {
        let before = self.plugged();
        match code {
            SW_HEADPHONE_INSERT => self.headphone = on,
            SW_MICROPHONE_INSERT => self.microphone = on,
            _ => {}
        }
        (self.plugged() != before).then(|| self.plugged())
    }
}

// --- volumes per route and the headphone safety cap ---

/// What a jack event asks the shell to do, in this order: pause first (an
/// unplug must never blast the speaker), switch the card, set the gain,
/// show the HUD, persist `prefs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub route: Route,
    pub volume: Volume,
    pub pause: bool,
    pub hud: bool,
    pub prefs: Vec<(&'static str, String)>,
}

/// The current route and each route's remembered volume.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Router {
    route: Route,
    speaker: Volume,
    headphones: Volume,
    lifted: bool,
    /// Whether the headset is in the jack now. Headphones can only be chosen while it is.
    plugged: bool,
    /// The route the user chose (the route picker). It holds against the jack until its device disappears: an
    /// unplug ends a Headphones choice, and a Speaker choice holds through plug and unplug.
    manual: Option<Route>,
}

impl Router {
    /// From settings.ini values (`get` reads one key). `legacy` is the old
    /// single `volume` key, taken for the speaker until it has its own.
    /// With the jack already plugged at start the cap applies at once.
    pub fn from_prefs(get: impl Fn(&str) -> Option<String>, plugged: bool) -> Router {
        let level = |k: &str| get(k).map(|v| Volume::from_pref(Some(&v)));
        let mut r = Router {
            route: Route::Speaker,
            speaker: level(PREF_SPEAKER).or_else(|| level(super::volume::PREF)).unwrap_or_default(),
            headphones: level(PREF_HEADPHONES).unwrap_or_default(),
            lifted: get(PREF_CAP_LIFTED).as_deref() == Some("1"),
            plugged,
            manual: None,
        };
        if plugged {
            r.route = Route::Headphones;
            r.headphones = r.capped(r.headphones);
        }
        r
    }

    pub fn route(&self) -> Route {
        self.route
    }

    /// The route the user chose, if one holds over the jack.
    pub fn manual(&self) -> Option<Route> {
        self.manual
    }

    /// The user picked a route. Headphones is refused while the headset is not in the jack. The change is what the
    /// binary applies (volume, HUD, saved prefs) and is None when the route does not change.
    pub fn choose(&mut self, to: Route) -> Option<Change> {
        if to == Route::Headphones && !self.plugged {
            return None;
        }
        self.manual = Some(to);
        self.switch(to, false)
    }

    /// The current route's volume.
    pub fn volume(&self) -> Volume {
        match self.route {
            Route::Speaker => self.speaker,
            Route::Headphones => self.headphones,
        }
    }

    fn capped(&self, v: Volume) -> Volume {
        if self.lifted {
            v
        } else {
            Volume::new(v.level().min(HEADPHONE_CAP))
        }
    }

    /// The jack changed. None when the route stays; else the change. A
    /// plug-in lowers a too-loud remembered headphone volume to the cap
    /// (until the user has lifted it); an unplug pauses playback.
    pub fn jack(&mut self, plugged: bool) -> Option<Change> {
        self.plugged = plugged;
        // the chosen headphones are gone with the headset
        if !plugged && self.manual == Some(Route::Headphones) {
            self.manual = None;
        }
        let to = self.manual.unwrap_or(if plugged { Route::Headphones } else { Route::Speaker });
        self.switch(to, !plugged)
    }

    /// Makes `to` the route, capping the headphone volume when it is headphones. None when it already is.
    fn switch(&mut self, to: Route, pause: bool) -> Option<Change> {
        if to == self.route {
            return None;
        }
        self.route = to;
        let mut prefs = Vec::new();
        if to == Route::Headphones {
            let capped = self.capped(self.headphones);
            if capped != self.headphones {
                self.headphones = capped;
                prefs.push((PREF_HEADPHONES, capped.pref()));
            }
        }
        Some(Change { route: to, volume: self.volume(), pause, hud: true, prefs })
    }

    /// The user set the current route's volume (slider or keys): what to
    /// persist. Going above the cap on headphones lifts it for good.
    pub fn set_level(&mut self, level: f32) -> Vec<(&'static str, String)> {
        let v = Volume::new(level);
        let mut prefs = vec![(self.route.pref_key(), v.pref())];
        match self.route {
            Route::Speaker => self.speaker = v,
            Route::Headphones => {
                self.headphones = v;
                if v.level() > HEADPHONE_CAP && !self.lifted {
                    self.lifted = true;
                    prefs.push((PREF_CAP_LIFTED, "1".to_string()));
                }
            }
        }
        prefs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_route_beats_the_jack_until_its_device_disappears() {
        let mut r = Router::from_prefs(|_| None, true);
        assert_eq!(r.route(), Route::Headphones);
        // the user picks the speaker while the headset is in: the plug stays ignored
        let change = r.choose(Route::Speaker).expect("speaker is a change");
        assert_eq!((change.route, change.pause), (Route::Speaker, false));
        assert!(r.jack(true).is_none(), "a plug-in does not steal a speaker choice");
        assert_eq!(r.route(), Route::Speaker);
        // unplugging does not change the speaker choice either
        assert!(r.jack(false).is_none());
        assert_eq!(r.manual(), Some(Route::Speaker));
        // headphones cannot be chosen without the headset
        assert!(r.choose(Route::Headphones).is_none());
        assert_eq!(r.route(), Route::Speaker);
    }

    #[test]
    fn a_headphones_choice_ends_with_the_headset() {
        let mut r = Router::from_prefs(|_| None, false);
        let change = r.jack(true).expect("plugging in switches");
        assert_eq!(change.route, Route::Headphones);
        assert!(r.choose(Route::Speaker).is_some());
        assert!(r.choose(Route::Headphones).is_some());
        assert_eq!(r.manual(), Some(Route::Headphones));
        let gone = r.jack(false).expect("unplugging ends the headphones choice");
        assert_eq!((gone.route, gone.pause), (Route::Speaker, true));
        assert_eq!(r.manual(), None);
    }

    #[test]
    fn choosing_the_current_route_changes_nothing() {
        let mut r = Router::from_prefs(|_| None, false);
        assert!(r.choose(Route::Speaker).is_none());
        assert_eq!(r.manual(), Some(Route::Speaker));
    }

    #[test]
    fn the_bluetooth_sink_is_the_connected_audio_device() {
        use crate::bare::bluetooth::{Device, State, Status};
        let dev = |name: &str, connected: bool, audio: bool| Device { addr: "AA:BB:CC:DD:EE:01".into(), kind: 0, name: name.into(), paired: true, connected, audio, rssi: None, class: None };
        let mut bt = Status { state: State::On, devices: vec![dev("Keyboard", true, false), dev("Buds", false, true)], ..Status::default() };
        assert_eq!(bluetooth_sink(&bt), None, "a keyboard or a disconnected headset is not a sink");
        bt.devices[1].connected = true;
        assert_eq!(bluetooth_sink(&bt), Some("Buds"));    }

    fn cand(name: &str, items: &[&str]) -> Candidate {
        Candidate { name: name.into(), items: items.iter().map(|s| s.to_string()).collect() }
    }

    fn card() -> Vec<Candidate> {
        vec![
            cand("Speaker Function", &[]),
            cand("Speaker1 Function", &[]),
            cand("HP Function", &["Off", "On"]),
            cand("Earpiece Function", &[]),
            cand("HP Playback Volume", &[]),
            cand("VBC DAC0 DG Set", &[]),
        ]
    }

    #[test]
    fn headphones_plan_turns_the_speaker_off_and_the_jack_path_on() {
        let p = plan(Route::Headphones, &card());
        let pos = |n: &str| p.iter().position(|(c, _)| c == n).unwrap();
        assert!(p.contains(&("HP Function".into(), "On".into())));
        assert!(p.contains(&("Speaker Function".into(), "0".into())));
        assert!(p.contains(&("Earpiece Function".into(), "0".into())));
        assert!(pos("Speaker Function") < pos("HP Function"), "the speaker goes off first");
        assert!(!p.iter().any(|(n, _)| n.contains("Volume") || n.contains("DG")), "gains are not route switches");
    }

    #[test]
    fn speaker_plan_turns_headphones_off_and_skips_missing_controls() {
        let p = plan(Route::Speaker, &card());
        assert!(p.contains(&("HP Function".into(), "Off".into())));
        assert!(plan(Route::Headphones, &[]).is_empty());
    }

    #[test]
    fn jack_is_plugged_by_either_switch() {
        let mut j = Jack::default();
        assert_eq!(j.feed(SW_HEADPHONE_INSERT, true), Some(true));
        assert_eq!(j.feed(SW_MICROPHONE_INSERT, true), None, "already plugged");
        assert_eq!(j.feed(SW_HEADPHONE_INSERT, false), None, "mic switch still on");
        assert_eq!(j.feed(SW_MICROPHONE_INSERT, false), Some(false));
        assert_eq!(j.feed(0x07, true), None);
        assert!(Jack::from_bits(1 << SW_MICROPHONE_INSERT).plugged());
        assert!(!Jack::from_bits(0).plugged());
    }

    fn prefs(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn each_route_remembers_its_volume_and_legacy_feeds_the_speaker() {
        let r = Router::from_prefs(prefs(&[("volume", "0.5"), (PREF_HEADPHONES, "0.25")]), false);
        assert_eq!((r.route(), r.volume().level()), (Route::Speaker, 0.5));
        let mut r = Router::from_prefs(prefs(&[]), false);
        assert_eq!(r.volume().level(), 0.0, "silent until turned up");
        r.set_level(0.4);
        let c = r.jack(true).unwrap();
        assert_eq!((c.route, c.volume.level(), c.hud, c.pause), (Route::Headphones, 0.0, true, false));
        r.set_level(0.3);
        let c = r.jack(false).unwrap();
        assert_eq!((c.route, c.volume.level(), c.pause), (Route::Speaker, 0.4, true));
        assert_eq!(r.jack(false), None);
    }

    #[test]
    fn a_fresh_router_stays_muted_and_a_saved_one_is_kept() {
        assert_eq!(Router::from_prefs(prefs(&[]), false).volume().level(), 0.0, "no saved volume -> 0.0 (muted)");
        assert_eq!(Router::from_prefs(prefs(&[("volume", "0.75")]), false).volume().level(), 0.75, "a saved volume is kept");
    }

    #[test]
    fn plug_in_caps_loud_headphones_until_the_user_lifts_it() {
        let mut r = Router::from_prefs(prefs(&[(PREF_HEADPHONES, "1.0")]), false);
        let c = r.jack(true).unwrap();
        assert_eq!(c.volume.level(), HEADPHONE_CAP);
        assert_eq!(c.prefs, vec![(PREF_HEADPHONES, "0.850".to_string())]);
        assert_eq!(r.set_level(0.85), vec![(PREF_HEADPHONES, "0.850".to_string())], "at the cap is not a lift");
        let p = r.set_level(0.9);
        assert!(p.contains(&(PREF_CAP_LIFTED, "1".to_string())));
        r.jack(false);
        assert_eq!(r.jack(true).unwrap().volume.level(), 0.9, "kept after lifting");
        let r = Router::from_prefs(prefs(&[(PREF_HEADPHONES, "0.95")]), true);
        assert_eq!((r.route(), r.volume().level()), (Route::Headphones, HEADPHONE_CAP), "plugged at boot is capped too");
        let r = Router::from_prefs(prefs(&[(PREF_HEADPHONES, "0.95"), (PREF_CAP_LIFTED, "1")]), true);
        assert_eq!(r.volume().level(), 0.95);
    }
}
