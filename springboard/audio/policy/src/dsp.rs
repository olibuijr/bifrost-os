//! The system DSP graph: which processing stages run for each audio device and for each application's stream, and
//! how much latency they add.
//!
//! A device has a *chain* of stages (gain, equaliser, compressor, noise gate, delay, limiter) the user or a device
//! profile set. An application can add stages of its own after the device's. The manager turns those, plus the
//! session category of the stream, into the plan the audio thread runs: a stage that is off is left out, and a
//! category that wants clean audio bypasses the stages that colour it (a recording session bypasses the equaliser,
//! compressor, delay and limiter of the microphone chain). A limiter always runs last. The plan carries its total
//! added latency, so the server can report it with the device latency. Chains are checked when they are set, and
//! the whole configuration saves to and loads from one text file.

use crate::sessions::{Category, Direction};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Gain,
    Eq,
    Compressor,
    Limiter,
    NoiseGate,
    Delay,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Gain => "gain",
            Kind::Eq => "eq",
            Kind::Compressor => "compressor",
            Kind::Limiter => "limiter",
            Kind::NoiseGate => "noise-gate",
            Kind::Delay => "delay",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        [Kind::Gain, Kind::Eq, Kind::Compressor, Kind::Limiter, Kind::NoiseGate, Kind::Delay].into_iter().find(|k| k.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub id: String,
    pub kind: Kind,
    pub enabled: bool,
    pub params: BTreeMap<String, f32>,
}

impl Stage {
    pub fn new(id: &str, kind: Kind) -> Stage {
        Stage { id: id.to_string(), kind, enabled: true, params: BTreeMap::new() }
    }

    pub fn with(mut self, param: &str, value: f32) -> Stage {
        self.params.insert(param.to_string(), value);
        self
    }

    pub fn off(mut self) -> Stage {
        self.enabled = false;
        self
    }

    /// Frames of latency this stage adds: a delay adds its `frames`, a limiter its `lookahead`.
    pub fn latency_frames(&self) -> u32 {
        let param = match self.kind {
            Kind::Delay => "frames",
            Kind::Limiter => "lookahead",
            _ => return 0,
        };
        self.params.get(param).map_or(0, |v| v.max(0.0) as u32)
    }
}

/// Why a chain or a file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    BadId(String),
    DuplicateId(String),
    BadParam { stage: String, param: String },
    /// A limiter only makes sense at the end of the chain.
    LimiterNotLast(String),
    /// A uid or app id with whitespace cannot be saved.
    BadName(String),
    /// A line of a saved file that cannot be read, 1-based.
    Syntax(usize),
}

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(char::is_whitespace)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    pub stages: Vec<Stage>,
}

impl Chain {
    pub fn new(stages: Vec<Stage>) -> Chain {
        Chain { stages }
    }

    pub fn validate(&self) -> Result<(), Error> {
        let mut seen = BTreeSet::new();
        for (i, s) in self.stages.iter().enumerate() {
            if !valid_id(&s.id) {
                return Err(Error::BadId(s.id.clone()));
            }
            if !seen.insert(s.id.as_str()) {
                return Err(Error::DuplicateId(s.id.clone()));
            }
            for (p, v) in &s.params {
                if !valid_id(p) || !v.is_finite() {
                    return Err(Error::BadParam { stage: s.id.clone(), param: p.clone() });
                }
            }
            if s.kind == Kind::Limiter && i + 1 != self.stages.len() {
                return Err(Error::LimiterNotLast(s.id.clone()));
            }
        }
        Ok(())
    }
}

/// Where a planned stage came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Device,
    App,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub stage: Stage,
    pub origin: Origin,
}

/// The processing for one device (and optionally one application stream) and what it costs.
#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    pub uid: String,
    pub direction: Direction,
    pub stages: Vec<Planned>,
    pub latency_frames: u32,
}

#[derive(Clone, Debug)]
pub struct DspManager {
    devices: BTreeMap<(String, Direction), Chain>,
    apps: BTreeMap<String, Chain>,
    bypass: BTreeMap<(Category, Direction), BTreeSet<Kind>>,
}

impl Default for DspManager {
    fn default() -> Self {
        let mut bypass = BTreeMap::new();
        bypass.insert((Category::Record, Direction::Input), [Kind::Eq, Kind::Compressor, Kind::Limiter, Kind::Delay].into_iter().collect());
        bypass.insert((Category::PlayAndRecord, Direction::Input), [Kind::Compressor, Kind::Delay].into_iter().collect());
        DspManager { devices: BTreeMap::new(), apps: BTreeMap::new(), bypass }
    }
}

impl DspManager {
    pub fn new() -> DspManager {
        DspManager::default()
    }

    pub fn set_device_chain(&mut self, uid: &str, direction: Direction, chain: Chain) -> Result<(), Error> {
        if !valid_name(uid) {
            return Err(Error::BadName(uid.to_string()));
        }
        chain.validate()?;
        if chain.stages.is_empty() {
            self.devices.remove(&(uid.to_string(), direction));
        } else {
            self.devices.insert((uid.to_string(), direction), chain);
        }
        Ok(())
    }

    pub fn set_app_chain(&mut self, app: &str, chain: Chain) -> Result<(), Error> {
        if !valid_name(app) {
            return Err(Error::BadName(app.to_string()));
        }
        chain.validate()?;
        if chain.stages.is_empty() {
            self.apps.remove(app);
        } else {
            self.apps.insert(app.to_string(), chain);
        }
        Ok(())
    }

    /// Replaces the kinds a category bypasses in a direction.
    pub fn set_bypass(&mut self, category: Category, direction: Direction, kinds: &[Kind]) {
        self.bypass.insert((category, direction), kinds.iter().copied().collect());
    }

    pub fn device_chain(&self, uid: &str, direction: Direction) -> Option<&Chain> {
        self.devices.get(&(uid.to_string(), direction))
    }

    /// What runs for `uid` in `direction`, for the stream of `app` under `category` (either may be unknown).
    /// Disabled and bypassed stages are left out, and a limiter is moved last.
    pub fn assignment(&self, uid: &str, direction: Direction, app: Option<&str>, category: Option<Category>) -> Assignment {
        let bypassed = category.and_then(|c| self.bypass.get(&(c, direction)));
        let skip = |s: &Stage| !s.enabled || bypassed.map_or(false, |b| b.contains(&s.kind));
        let device = self.devices.get(&(uid.to_string(), direction)).into_iter().flat_map(|c| c.stages.iter()).filter(|s| !skip(*s)).map(|s| Planned { stage: s.clone(), origin: Origin::Device });
        let app_stages = app.and_then(|a| self.apps.get(a)).into_iter().flat_map(|c| c.stages.iter()).filter(|s| !skip(*s)).map(|s| Planned { stage: s.clone(), origin: Origin::App });
        let (limiters, mut stages): (Vec<Planned>, Vec<Planned>) = device.chain(app_stages).partition(|p| p.stage.kind == Kind::Limiter);
        stages.extend(limiters);
        let latency_frames = stages.iter().map(|p| p.stage.latency_frames()).sum();
        Assignment { uid: uid.to_string(), direction, stages, latency_frames }
    }

    /// The system-wide graph: the base assignment of every device that has a chain.
    pub fn graph(&self) -> Vec<Assignment> {
        self.devices.keys().map(|(uid, direction)| self.assignment(uid, *direction, None, None)).collect()
    }

    /// The configuration as text: `device <uid> <in|out>` or `app <id>` opens a chain, `stage <id> <kind> <on|off>
    /// [name=value ...]` adds a stage to it.
    pub fn save(&self) -> String {
        let mut out = String::new();
        let dir = |d: Direction| if d == Direction::Input { "in" } else { "out" };
        let stages = |out: &mut String, chain: &Chain| {
            for s in &chain.stages {
                out.push_str(&format!("stage {} {} {}", s.id, s.kind.name(), if s.enabled { "on" } else { "off" }));
                for (k, v) in &s.params {
                    out.push_str(&format!(" {k}={v}"));
                }
                out.push('\n');
            }
        };
        for ((uid, d), chain) in &self.devices {
            out.push_str(&format!("device {uid} {}\n", dir(*d)));
            stages(&mut out, chain);
        }
        for (app, chain) in &self.apps {
            out.push_str(&format!("app {app}\n"));
            stages(&mut out, chain);
        }
        out
    }

    /// Reads what `save` wrote. The bypass table keeps its defaults. A line that cannot be read, or a chain that
    /// does not validate, refuses the whole file.
    pub fn load(text: &str) -> Result<DspManager, Error> {
        enum Target {
            Device(String, Direction),
            App(String),
        }
        let mut manager = DspManager::new();
        let mut open: Option<(Target, Vec<Stage>)> = None;
        let finish = |manager: &mut DspManager, open: Option<(Target, Vec<Stage>)>| -> Result<(), Error> {
            match open {
                Some((Target::Device(uid, d), stages)) => manager.set_device_chain(&uid, d, Chain::new(stages)),
                Some((Target::App(app), stages)) => manager.set_app_chain(&app, Chain::new(stages)),
                None => Ok(()),
            }
        };
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                ["device", uid, dir @ ("in" | "out")] => {
                    finish(&mut manager, open.take())?;
                    let d = if *dir == "in" { Direction::Input } else { Direction::Output };
                    open = Some((Target::Device(uid.to_string(), d), Vec::new()));
                }
                ["app", app] => {
                    finish(&mut manager, open.take())?;
                    open = Some((Target::App(app.to_string()), Vec::new()));
                }
                ["stage", id, kind, state, params @ ..] => {
                    let kind = Kind::from_name(kind).ok_or(Error::Syntax(line_no))?;
                    let enabled = match *state {
                        "on" => true,
                        "off" => false,
                        _ => return Err(Error::Syntax(line_no)),
                    };
                    let mut stage = Stage::new(id, kind);
                    stage.enabled = enabled;
                    for p in params {
                        let (k, v) = p.split_once('=').ok_or(Error::Syntax(line_no))?;
                        stage.params.insert(k.to_string(), v.parse::<f32>().map_err(|_| Error::Syntax(line_no))?);
                    }
                    match open.as_mut() {
                        Some((_, stages)) => stages.push(stage),
                        None => return Err(Error::Syntax(line_no)),
                    }
                }
                _ => return Err(Error::Syntax(line_no)),
            }
        }
        finish(&mut manager, open.take())?;
        Ok(manager)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic_chain() -> Chain {
        Chain::new(vec![
            Stage::new("trim", Kind::Gain).with("db", 6.0),
            Stage::new("gate", Kind::NoiseGate).with("threshold_db", -50.0),
            Stage::new("voice-eq", Kind::Eq),
            Stage::new("squash", Kind::Compressor).with("ratio", 3.0),
            Stage::new("safety", Kind::Limiter).with("lookahead", 48.0),
        ])
    }

    #[test]
    fn chains_are_checked_when_set() {
        let mut m = DspManager::new();
        assert!(m.set_device_chain("alsa:mic", Direction::Input, mic_chain()).is_ok());
        let dup = Chain::new(vec![Stage::new("a", Kind::Gain), Stage::new("a", Kind::Eq)]);
        assert_eq!(m.set_device_chain("x", Direction::Output, dup), Err(Error::DuplicateId("a".into())));
        let bad_id = Chain::new(vec![Stage::new("Bad Id", Kind::Gain)]);
        assert_eq!(m.set_device_chain("x", Direction::Output, bad_id), Err(Error::BadId("Bad Id".into())));
        let nan = Chain::new(vec![Stage::new("g", Kind::Gain).with("db", f32::NAN)]);
        assert!(matches!(m.set_device_chain("x", Direction::Output, nan), Err(Error::BadParam { .. })));
        let early = Chain::new(vec![Stage::new("lim", Kind::Limiter), Stage::new("g", Kind::Gain)]);
        assert_eq!(m.set_device_chain("x", Direction::Output, early), Err(Error::LimiterNotLast("lim".into())));
        assert_eq!(m.set_device_chain("has space", Direction::Output, Chain::default()), Err(Error::BadName("has space".into())));
        assert!(m.device_chain("x", Direction::Output).is_none());
    }

    #[test]
    fn an_assignment_leaves_out_stages_that_are_off() {
        let mut m = DspManager::new();
        let chain = Chain::new(vec![Stage::new("a", Kind::Gain), Stage::new("b", Kind::Eq).off()]);
        m.set_device_chain("spk", Direction::Output, chain).unwrap();
        let plan = m.assignment("spk", Direction::Output, None, None);
        assert_eq!(plan.stages.len(), 1);
        assert_eq!(plan.stages[0].stage.id, "a");
        assert_eq!(m.assignment("other", Direction::Output, None, None).stages.len(), 0);
    }

    #[test]
    fn a_recording_session_bypasses_the_stages_that_colour_the_signal() {
        let mut m = DspManager::new();
        m.set_device_chain("alsa:mic", Direction::Input, mic_chain()).unwrap();
        let ids = |a: &Assignment| a.stages.iter().map(|p| p.stage.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&m.assignment("alsa:mic", Direction::Input, None, None)), vec!["trim", "gate", "voice-eq", "squash", "safety"]);
        assert_eq!(ids(&m.assignment("alsa:mic", Direction::Input, None, Some(Category::Record))), vec!["trim", "gate"]);
        assert_eq!(ids(&m.assignment("alsa:mic", Direction::Input, None, Some(Category::PlayAndRecord))), vec!["trim", "gate", "voice-eq", "safety"]);
        assert_eq!(ids(&m.assignment("alsa:mic", Direction::Input, None, Some(Category::Playback))).len(), 5, "playback does not bypass");
        m.set_bypass(Category::Record, Direction::Input, &[]);
        assert_eq!(ids(&m.assignment("alsa:mic", Direction::Input, None, Some(Category::Record))).len(), 5);
    }

    #[test]
    fn an_app_chain_runs_after_the_device_chain_and_a_limiter_stays_last() {
        let mut m = DspManager::new();
        let dev = Chain::new(vec![Stage::new("bass", Kind::Eq), Stage::new("safety", Kind::Limiter).with("lookahead", 32.0)]);
        m.set_device_chain("spk", Direction::Output, dev).unwrap();
        m.set_app_chain("is.olibuijr.music", Chain::new(vec![Stage::new("echo", Kind::Delay).with("frames", 480.0)])).unwrap();
        let plan = m.assignment("spk", Direction::Output, Some("is.olibuijr.music"), None);
        let order: Vec<(&str, Origin)> = plan.stages.iter().map(|p| (p.stage.id.as_str(), p.origin)).collect();
        assert_eq!(order, vec![("bass", Origin::Device), ("echo", Origin::App), ("safety", Origin::Device)]);
        assert_eq!(plan.latency_frames, 480 + 32);
        let other = m.assignment("spk", Direction::Output, Some("someone.else"), None);
        assert_eq!(other.stages.len(), 2);
        assert_eq!(other.latency_frames, 32);
    }

    #[test]
    fn the_system_graph_lists_every_device_with_a_chain() {
        let mut m = DspManager::new();
        m.set_device_chain("spk", Direction::Output, Chain::new(vec![Stage::new("g", Kind::Gain)])).unwrap();
        m.set_device_chain("mic", Direction::Input, mic_chain()).unwrap();
        let graph = m.graph();
        assert_eq!(graph.len(), 2);
        assert!(graph.iter().any(|a| a.uid == "mic" && a.direction == Direction::Input && a.latency_frames == 48));
        m.set_device_chain("mic", Direction::Input, Chain::default()).unwrap();
        assert_eq!(m.graph().len(), 1, "an empty chain removes the device from the graph");
    }

    #[test]
    fn the_configuration_round_trips_through_text() {
        let mut m = DspManager::new();
        m.set_device_chain("alsa:mic", Direction::Input, mic_chain()).unwrap();
        m.set_device_chain("spk", Direction::Output, Chain::new(vec![Stage::new("off", Kind::Eq).off()])).unwrap();
        m.set_app_chain("app.one", Chain::new(vec![Stage::new("d", Kind::Delay).with("frames", 10.0)])).unwrap();
        let text = m.save();
        let back = DspManager::load(&text).unwrap();
        assert_eq!(back.save(), text);
        assert_eq!(back.device_chain("alsa:mic", Direction::Input), m.device_chain("alsa:mic", Direction::Input));
    }

    #[test]
    fn a_bad_file_is_refused_with_its_line() {
        assert_eq!(DspManager::load("stage a gain on\n").err(), Some(Error::Syntax(1)));
        assert_eq!(DspManager::load("device d out\nstage a loud on\n").err(), Some(Error::Syntax(2)));
        assert_eq!(DspManager::load("device d out\nstage a gain maybe\n").err(), Some(Error::Syntax(2)));
        assert_eq!(DspManager::load("device d out\nstage a gain on db=x\n").err(), Some(Error::Syntax(2)));
        assert_eq!(DspManager::load("device d out\nstage l limiter on\nstage a gain on\n").err(), Some(Error::LimiterNotLast("l".into())));
        assert!(DspManager::load("# nothing\n\n").unwrap().graph().is_empty());
    }
}
