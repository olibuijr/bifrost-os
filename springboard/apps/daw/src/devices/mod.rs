//! Devices: instruments and effects, with the same names and parameter ranges as the mobile DAW plugins
//! (`data/plugins/<id>/index.js`). Each device is built from a spec (`P` list) plus a `Params` value.

pub mod fx;
pub mod inst;

use crate::model::Extra;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Curve {
    Lin,
    Log,
    Int,
}

/// One parameter: name, range, default, curve, unit (same tuple as the web plugin specs).
#[derive(Clone, Copy, Debug)]
pub struct P {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub def: f64,
    pub curve: Curve,
    pub unit: &'static str,
}

pub const fn p(name: &'static str, min: f64, max: f64, def: f64, curve: Curve, unit: &'static str) -> P {
    P { name, min, max, def, curve, unit }
}

/// Resolved parameter values for one device.
#[derive(Clone, Debug, Default)]
pub struct Params {
    vals: BTreeMap<&'static str, f64>,
}

impl Params {
    /// Defaults from `spec`, overridden by the finite numeric values in `given` (clamped to the spec range).
    pub fn new(spec: &[P], given: &Extra) -> Self {
        Self::with_defaults(spec, &[], given)
    }

    /// Like `new`, but `defaults` replace the spec defaults (the mobile DAW's per-device wrapper defaults).
    /// Song values still win over both.
    pub fn with_defaults(spec: &[P], defaults: &[(&str, f64)], given: &Extra) -> Self {
        let mut vals = BTreeMap::new();
        for s in spec {
            let base = defaults.iter().find(|(n, _)| *n == s.name).map_or(s.def, |(_, v)| *v);
            let v = given.get(s.name).and_then(|x| x.as_f64()).filter(|x| x.is_finite());
            vals.insert(s.name, clamp(s, v.unwrap_or(base)));
        }
        Params { vals }
    }

    pub fn get(&self, name: &str) -> f64 {
        self.vals.get(name).copied().unwrap_or(0.0)
    }

    pub fn has(&self, name: &str) -> bool {
        self.vals.contains_key(name)
    }

    /// Sets a value (clamped by `spec`). Unknown names are ignored, like the web `clampParam`.
    pub fn set(&mut self, spec: &[P], name: &str, v: f64) {
        if let Some(s) = spec.iter().find(|s| s.name == name) {
            if v.is_finite() {
                self.vals.insert(s.name, clamp(s, v));
            }
        }
    }
}

fn clamp(s: &P, v: f64) -> f64 {
    let x = v.clamp(s.min, s.max);
    if s.curve == Curve::Int {
        x.round()
    } else {
        x
    }
}

/// Per-device environment: sample rate, where plugin assets live, and a deterministic seed.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub sr: f32,
    /// Directory holding `<plugin id>/…` assets (the mobile DAW's `data/plugins`).
    pub plugins: PathBuf,
    pub seed: u64,
}

/// A note-playing device. Times are absolute frame indices at the render sample rate.
pub trait Instrument: Send {
    /// Starts a note at frame `at`, lasting `dur` frames (the note-off is at `at + dur`).
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, dur: u64);
    /// Sets a parameter live (automation, pitch bend is not a parameter).
    fn set(&mut self, name: &str, value: f64);
    /// Adds the next `l.len()` frames, starting at absolute frame `start`, into `l` and `r`.
    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64);
    /// Tempo for tempo-synced instruments (wobble); `origin` is the frame of the first step.
    fn tempo(&mut self, _bpm: f64, _origin: u64) {}
    /// Sustain pedal (CC64) at frame `at`: true = down. Ignored by instruments without a pedal.
    fn pedal(&mut self, _down: bool, _at: u64) {}
    /// Pitch bend in semitones from frame `at`. Ignored by instruments without bend.
    fn bend(&mut self, _semis: f64, _at: u64) {}
}

/// An audio processor on a track or the master bus.
pub trait Effect: Send {
    /// Processes one block in place. `start` is the absolute frame index of `l[0]`.
    fn process(&mut self, start: u64, l: &mut [f32], r: &mut [f32]);
    /// Sets a parameter live (automation).
    fn set(&mut self, name: &str, value: f64);
    /// Tempo for tempo-synced devices; `origin` is the frame of the first step (the transport phase reference).
    fn tempo(&mut self, _bpm: f64, _origin: u64) {}
}

/// Pass-through effect used for unknown or disabled devices.
pub struct Bypass;

impl Effect for Bypass {
    fn process(&mut self, _start: u64, _l: &mut [f32], _r: &mut [f32]) {}
    fn set(&mut self, _name: &str, _value: f64) {}
}

/// Parameter spec for an instrument type, or None if the type is unknown.
pub fn instrument_spec(kind: &str) -> Option<&'static [P]> {
    inst::spec(kind)
}

/// Builds an instrument from its song entry (params, pads, sample). None when the type is unknown or its
/// assets are missing (the track is then silent).
pub fn instrument(model: &crate::model::Instrument, ctx: &Ctx) -> Option<Box<dyn Instrument>> {
    let spec = inst::spec(&model.kind)?;
    let params = Params::with_defaults(spec, inst::defaults(&model.kind), &model.params);
    inst::new(&model.kind, &params, model, ctx)
}

/// Parameter spec for an effect type, or None if the type is unknown.
pub fn effect_spec(kind: &str) -> Option<&'static [P]> {
    fx::spec(kind)
}

/// Builds an effect. Unknown types become a bypass so a song still plays.
pub fn effect(kind: &str, given: &Extra, ctx: &Ctx) -> Box<dyn Effect> {
    match fx::spec(kind) {
        Some(spec) => fx::new(kind, &Params::new(spec, given), ctx),
        None => Box::new(Bypass),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &[P] = &[
        p("gain", 0.0, 1.0, 0.7, Curve::Lin, ""),
        p("octave", -2.0, 2.0, 0.0, Curve::Int, ""),
    ];

    #[test]
    fn params_default_clamp_and_round() {
        let mut given = Extra::new();
        given.insert("gain".into(), serde_json::json!(5.0));
        given.insert("octave".into(), serde_json::json!(1.4));
        let p = Params::new(SPEC, &given);
        assert_eq!(p.get("gain"), 1.0);
        assert_eq!(p.get("octave"), 1.0);
    }

    #[test]
    fn non_numeric_values_fall_back_to_default() {
        let mut given = Extra::new();
        given.insert("gain".into(), serde_json::json!("loud"));
        let p = Params::new(SPEC, &given);
        assert!((p.get("gain") - 0.7).abs() < 1e-12);
    }
}

#[cfg(test)]
mod default_track_tests {
    /// "+ Track" creates a synth; it needs a known spec with defaults.
    #[test]
    fn the_default_track_instrument_has_parameters() {
        let spec = super::instrument_spec("synth").expect("synth is a known instrument");
        assert!(!spec.is_empty());
        assert!(!crate::ai::default_params("synth", None).is_empty());
    }
}
