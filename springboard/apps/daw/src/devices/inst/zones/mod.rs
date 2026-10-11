//! Sampled instruments: the port of `app/src/plugins/multisample.js` (host.multisample) for the zone sets in
//! `data/plugins/<id>/zones.json`. Split by concern:
//! - `manifest`: zones.json parsing and normalised zone records
//! - `select`: zone pick, articulation pool, round robin and velocity-layer crossfades
//! - `layer`: one sample layer's playback, loop points and loop-seam blend
//! - `voice`: one sounding note's state (envelope, filter, glide, pedal)
//! - `spec`: the per-device parameter list
//! This file holds the instrument and its `Instrument` impl.
//! Not ported yet: legato crossfade transitions, the release-tail crossfade, release noise, the decay param and
//! brass/soft-attack envelopes.

mod layer;
mod manifest;
mod select;
mod spec;
mod voice;

pub use spec::spec_for;

use crate::assets::{self, Audio};
use crate::devices::{p, Ctx, Curve, Instrument, Params, P};
use crate::dsp::{pan_gains, Biquad, FilterKind, Rng};
use layer::{loop_frames, Layer};
use manifest::Manifest;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use voice::{Glide, Shape, Voice};

/// Instrument types that play a zone set (`data/plugins/<type>/zones.json`).
const SAMPLED: &[&str] = &[
    "celesta", "cello", "choir", "clarinet", "contrabass", "flute", "frenchhorn", "glock", "grandpiano", "harp",
    "nylonguitar", "oboe", "organ", "rhodes", "strings", "trumpet", "uprightbass", "vibes", "violin", "wurli",
    "808clean", "808dist", "808punch", "boombapkit", "trapkit", "acoustickit",
];

/// Plugin folder of a device type. Song types are named after their sound (`clean808`), folders are `808clean`.
pub fn plugin_dir(kind: &str) -> &str {
    match kind {
        "clean808" => "808clean",
        "dist808" => "808dist",
        "punch808" => "808punch",
        k => k,
    }
}

pub fn is_sampled(kind: &str) -> bool {
    SAMPLED.contains(&plugin_dir(kind))
}

/// The shared parameter list (the `MULTI_PARAMS` table of multisample.js). Per-device lists come from `spec_for`.
pub const SPEC: &[P] = &[
    p("attack", 0.1, 10.0, 1.0, Curve::Log, "x"),
    p("release", 0.1, 10.0, 1.0, Curve::Log, "x"),
    p("tone", 200.0, 20000.0, 20000.0, Curve::Log, "Hz"),
    p("bright", 0.0, 1.0, 0.6, Curve::Lin, ""),
    p("keytrack", 0.0, 1.0, 0.0, Curve::Lin, ""),
    p("vibrato", 0.0, 60.0, 0.0, Curve::Lin, "ct"),
    p("rate", 3.0, 8.0, 5.5, Curve::Lin, ""),
    p("velocity", 0.0, 1.0, 0.7, Curve::Lin, ""),
    p("velcurve", 0.3, 3.0, 1.6, Curve::Log, ""),
    p("human", 0.0, 1.0, 0.0, Curve::Lin, ""),
    p("detune", 0.0, 25.0, 0.0, Curve::Lin, "ct"),
    p("stereo", 0.0, 1.0, 0.15, Curve::Lin, ""),
    p("gain", 0.0, 1.5, 0.8, Curve::Lin, ""),
];

/// Defaults the mobile DAW's wrappers set for this device (they replace the spec default, not the song value).
pub fn defaults(kind: &str) -> &'static [(&'static str, f64)] {
    match plugin_dir(kind) {
        "808clean" => &[("stereo", 0.0), ("velocity", 0.4), ("gain", 0.65)],
        "808dist" => &[("stereo", 0.0), ("velocity", 0.4), ("gain", 0.6)],
        "808punch" => &[("stereo", 0.0), ("velocity", 0.4), ("gain", 0.45)],
        "boombapkit" | "trapkit" | "acoustickit" => &[("stereo", 0.0), ("velocity", 0.75), ("gain", 0.5)],
        "organ" => &[("velocity", 0.15), ("stereo", 0.1)],
        _ => &[],
    }
}

pub struct Zones {
    man: Manifest,
    sr: f32,
    spec: &'static [P],
    params: Params,
    rng: Rng,
    voices: Vec<Voice>,
    next_id: u64,
    /// round-robin cursors per (region, velocity) key
    rr: HashMap<(i64, i64), usize>,
    /// choke group -> id of its most recent voice
    groups: HashMap<String, u64>,
    /// id of the most recently started voice (glide and legato target)
    last: Option<u64>,
    /// loop-seam blended audio, per (file, loop start, loop end) in frames
    loop_xf: HashMap<(PathBuf, i64, i64), Arc<Audio>>,
    /// plugin.json `legato`: wide intervals and zone changes crossfade instead of gliding
    legato: bool,
    /// explicit sustain-pedal events (frame, down), sorted; applied at render time from `pedal_cursor`
    pedal_ev: Vec<(u64, bool)>,
    pedal_cursor: usize,
    /// pitch-bend events (frame, semitones), sorted; `bend_ratio` is the current rate factor
    bend_ev: Vec<(u64, f64)>,
    bend_cursor: usize,
    bend_ratio: f64,
    max_voices: usize,
}

/// Builds a sampled instrument. None when the zone manifest is missing or unreadable.
pub fn new(kind: &str, params: &Params, ctx: &Ctx) -> Option<Box<dyn Instrument>> {
    let man = manifest::load(&ctx.plugins.join(plugin_dir(kind)))?;
    Some(Box::new(Zones {
        man,
        sr: ctx.sr,
        spec: spec_for(kind),
        params: params.clone(),
        rng: Rng::new(ctx.seed ^ 0x2_0000),
        voices: vec![],
        next_id: 1,
        rr: HashMap::new(),
        groups: HashMap::new(),
        last: None,
        loop_xf: HashMap::new(),
        legato: spec::plugin_flags(kind).0,
        pedal_ev: vec![],
        pedal_cursor: 0,
        bend_ev: vec![],
        bend_cursor: 0,
        bend_ratio: 1.0,
        max_voices: 48,
    }))
}

impl Zones {
    /// A parameter by name. Hidden parameters (not in this device's spec) keep the shared default, as in
    /// multisample.js; parameters the device does not have read as 0.
    fn param(&self, name: &str) -> f64 {
        if self.params.has(name) {
            return self.params.get(name);
        }
        SPEC.iter().find(|s| s.name == name).map_or(0.0, |s| s.def)
    }

    /// Choke: fades voice `id` out with the manifest's choke time, starting at frame `at`.
    fn fade_out_voice(&mut self, id: u64, at: u64) {
        let tau = self.man.choke_time as f32;
        let coeff = (-1.0 / (tau * self.sr)).exp();
        if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
            if v.start <= at {
                v.released = true;
                v.release_at = Some(at);
                v.rel_coeff = coeff;
            }
        }
    }

    /// Legato glide (`glide` param): a note in the same zone while the last voice is still held slides that voice
    /// to the new pitch instead of starting a new one. Returns false when there is nothing to glide.
    fn try_glide(&mut self, zone: usize, pitch: f64, at: u64, dur: u64, secs: f64) -> bool {
        let sr = self.sr as f64;
        let shape = self.param("glidecurve").clamp(0.0, 1.0);
        let Some(id) = self.last else { return false };
        let Some(v) = self.voices.iter_mut().find(|v| v.id == id) else { return false };
        let held = v.zone == zone
            && !v.released
            && !v.done
            && !v.pedal
            && v.start < at
            && v.release_at.map_or(true, |r| r > at);
        if !held {
            return false;
        }
        let to = v.ratio * 2f64.powf((pitch - v.pitch) / 12.0);
        v.glide = Some(Glide { from: v.ratio, to, start: at, len: (secs * sr).max(1.0), k: 4f64.powf(2.0 * shape - 1.0) });
        v.pitch = pitch;
        v.release_at = Some(at + dur.max(1));
        true
    }

    /// Explicit pedal state at frame `at` (the last CC64 event at or before it), None = automatic re-pedal.
    fn sustain_at(&self, at: u64) -> Option<bool> {
        self.pedal_ev.iter().rev().find(|e| e.0 <= at).map(|e| e.1)
    }

    /// Applies a pedal change at frame `t` to the sounding voices (multisample.js `pedal`): pedal down holds
    /// every voice whose note-off is still ahead; pedal up releases the held ones with the pedal-release time.
    fn apply_pedal(&mut self, down: bool, t: u64) {
        let cap = (self.man.pedal_max * self.sr as f64) as u64;
        for v in self.voices.iter_mut() {
            if down {
                if let Some(r) = v.release_at {
                    if r > t && !v.released && !v.pedal && !v.silent {
                        v.pedal = true;
                        v.pedal_cap = r + cap;
                        v.release_at = None;
                    }
                }
            } else if v.pedal && !v.released {
                v.pedal = false;
                v.release_at = Some(t);
                v.rel_coeff = v.pedal_coeff;
            }
        }
    }

    /// The last voice if it is still held at `at`: (id, pitch, zone).
    fn held_last(&self, at: u64) -> Option<(u64, f64, usize)> {
        let id = self.last?;
        let v = self.voices.iter().find(|v| v.id == id)?;
        let held = !v.released && !v.done && !v.pedal && v.start < at && v.release_at.map_or(true, |r| r > at);
        held.then_some((v.id, v.pitch, v.zone))
    }

    /// Sustain pedal lift: voices held by the pedal release now (the next note starts after them).
    fn lift_pedal(&mut self, at: u64) {
        for v in self.voices.iter_mut().filter(|v| v.pedal && !v.released) {
            v.pedal = false;
            v.release_at = Some(at);
            v.rel_coeff = v.pedal_coeff;
        }
    }

    /// Loop-seam blend for a looping layer, cached per file and loop points (`xfBuf` in multisample.js).
    fn seamed(&mut self, file: &Path, audio: Arc<Audio>, a: f64, b: f64) -> Arc<Audio> {
        if !self.man.loop_xfade {
            return audio;
        }
        let key = (file.to_path_buf(), a.round() as i64, b.round() as i64);
        if let Some(x) = self.loop_xf.get(&key) {
            return x.clone();
        }
        let out = match layer::seam(&audio, a, b, self.sr as f64) {
            Some(channels) => Arc::new(Audio { channels }),
            None => audio,
        };
        self.loop_xf.insert(key, out.clone());
        out
    }
}

impl Instrument for Zones {
    fn note_on(&mut self, pitch: f64, vel: f64, at: u64, dur: u64) {
        let pitch = pitch.round();
        let vel = vel.clamp(0.0, 1.0);
        // humanize: timing jitter up to 12 ms
        let hum = self.param("human").clamp(0.0, 1.0);
        let at = if hum > 0.0 { at + (self.rng.unit() as f64 * 0.012 * hum * self.sr as f64) as u64 } else { at };
        let artic = self.param("artic").round().clamp(0.0, 4.0) as usize;
        let picks = select::layers_for(&self.man, &mut self.rr, artic, pitch, vel);
        let Some(&(first, _)) = picks.first() else { return };
        let zone0 = self.man.zones[first].clone();
        let one_shot = zone0.one_shot.unwrap_or(self.man.one_shot);
        let sr = self.sr as f64;

        let glide_secs = self.param("glide");
        // legato: a held voice in another zone, or a wide interval, crossfades into a new voice with a pitch scoop
        let mut transition: Option<(f64, f64, u64)> = None;
        if glide_secs > 0.0 && artic == 0 && !one_shot {
            if let Some((old, pitch0, zone_prev)) = self.held_last(at) {
                let iv = pitch - pitch0;
                if self.legato && (iv.abs() > 2.0 || first != zone_prev) {
                    transition = Some((0.07, (iv * 25.0).clamp(-150.0, 150.0), old));
                } else if first == zone_prev && self.try_glide(first, pitch, at, dur, glide_secs) {
                    return;
                }
            }
        }
        // automatic re-pedal only while no explicit CC64 state is in force
        if self.sustain_at(at).is_none() {
            self.lift_pedal(at);
        }

        // choke group: a new note fades the previous voice of its group
        if let Some(group) = zone0.choke.clone() {
            if let Some(&id) = self.groups.get(&group) {
                self.fade_out_voice(id, at);
            }
        }

        // humanize pitch: up to 4 cents
        let human_cents = if self.man.mono_sub { 0.0 } else { self.rng.bipolar() as f64 * 4.0 * hum };
        let detune = if self.man.mono_sub { 0.0 } else { self.rng.bipolar() as f64 * self.param("detune") } + human_cents;
        let pan_spread = if self.man.mono_sub { 0.0 } else { self.rng.bipolar() as f64 * self.param("stereo") };
        let mut layers = vec![];
        for (zi, w) in picks {
            let z = self.man.zones[zi].clone();
            let Some(audio) = assets::cached(&z.file, self.sr) else { continue };
            let pitched = z.pitched.unwrap_or(self.man.pitched);
            let mut rate = if pitched { 2f64.powf((pitch - z.root) / 12.0) } else { 1.0 };
            rate *= 2f64.powf((z.tune + detune) / 1200.0);
            let lp = if one_shot { None } else { loop_frames(&self.man, sr, &z, &audio) };
            let lp = lp.or_else(|| {
                let len = audio.channels[0].len() as f64;
                let held = dur as f64 / sr > len / sr - 0.05;
                (!one_shot && self.man.auto_loop && held && len / sr > 0.3).then(|| (len * 0.6, len * 0.9))
            });
            let audio = match lp {
                Some((a, b)) => self.seamed(&z.file, audio, a, b),
                None => audio,
            };
            layers.push(Layer::new(audio, rate, w * z.gain as f32, lp));
        }
        if layers.is_empty() {
            return;
        }

        let velocity = self.param("velocity");
        let vc = self.man.vel_curve.unwrap_or(self.param("velcurve"));
        let curve = vel.max(0.01).powf(vc);
        let peak = (self.param("gain") * (1.0 - velocity + velocity * curve) * (1.0 + 0.06 * hum * self.rng.bipolar() as f64)) as f32;
        let mut attack = (self.man.attack * self.param("attack")).max(0.002);
        if self.man.soft_attack > 0.0 {
            attack *= 1.0 + (1.0 - vel) * self.man.soft_attack;
        }
        let mut release = (self.man.release * self.param("release")).max(0.01);
        if self.man.release_keytrack {
            release *= 2f64.powf(-(pitch - 60.0) / 36.0);
        }
        let mut dur = dur;
        match artic {
            1 => {
                dur = dur.min((0.12 * sr) as u64);
                attack = 0.004;
                release = 0.08;
            }
            2 => attack = 0.002,
            4 => attack = attack.min(0.01),
            _ => {}
        }
        if let Some((fade, _, _)) = transition {
            attack = fade;
        }
        let tone = self.param("tone");
        let pizz = artic == 2;
        let filter_on = pizz || tone < 19_999.0 || self.man.brass_env;
        let fc_base = if pizz {
            2500.0 * 2f64.powf((pitch - 60.0) / 24.0)
        } else if tone >= 19_999.0 {
            9000.0
        } else {
            tone
        };
        let bright = self.param("bright").clamp(0.0, 1.0);
        let kt = self.param("keytrack");
        let fc = (fc_base * 2f64.powf(kt * (pitch - 60.0) / 12.0) * (1.0 - bright + bright * (0.35 + 0.65 * vel)))
            .clamp(20.0, 20_000.0);
        let brass = (filter_on && self.man.brass_env && !pizz).then_some((fc, attack));
        let filter = filter_on.then(|| {
            let start_fc = if brass.is_some() { fc * 0.5 } else { fc };
            let q = if pizz { 0.9 } else { 0.5 };
            let f = Biquad::new(FilterKind::Lowpass, self.sr, start_fc as f32, q, 0.0);
            [f, f]
        });
        let pan_pos = if self.man.mono_sub { 0.0 } else { pan_spread.clamp(-1.0, 1.0) };
        let (pl, pr) = pan_gains(pan_pos as f32);

        // humanize sample start: looped voices start up to 15 ms into the sample
        if hum > 0.0 && attack > 0.02 && artic == 0 && !one_shot {
            for l in layers.iter_mut() {
                l.pos = self.rng.unit() as f64 * 0.015 * hum * sr;
            }
        }

        // articulation and expression shape
        let mut shape = Shape::default();
        let vib = self.param("vibrato");
        if vib > 0.0 && artic != 1 && !pizz {
            shape.vib_cents = vib;
            shape.vib_inc = std::f64::consts::TAU * self.param("rate") * (0.95 + 0.1 * self.rng.unit() as f64) / sr;
        }
        if artic == 3 {
            shape.trem_inc = std::f64::consts::TAU * (9.0 + 4.0 * self.rng.unit() as f64) / sr;
            shape.trem_phase = self.rng.unit() as f64 * std::f64::consts::TAU;
        }
        shape.marc = artic == 4;
        if pizz {
            shape.decay_start = at + (0.01 * sr) as u64;
            shape.decay_tau = ((0.35 - 0.23 * ((pitch - 28.0) / 60.0).clamp(0.0, 1.0)) * sr) as f32;
        }
        if one_shot {
            // kit decay: the sample fades after `decay` seconds when shorter than the sample
            let d = self.param("decay");
            let secs = layers.first().map_or(0.0, |l| l.len() / sr);
            if d > 0.0 && d < secs {
                shape.decay_start = at + (attack * sr) as u64 + (0.25 * d * sr) as u64;
                shape.decay_tau = (d / 4.0 * sr) as f32;
            }
        }

        let id = self.next_id;
        self.next_id += 1;
        let off = at + (dur as f64).max(attack * sr) as u64;
        let sustain = self.sustain_at(at).unwrap_or(self.param("pedal") >= 0.5);
        let pedal_on = sustain && !one_shot && artic == 0;
        let pedal_tau = self.man.pedal_release.unwrap_or(release).max(0.01) / 4.0;
        let voice = Voice {
            id,
            layers,
            start: at,
            release_at: if one_shot || pedal_on { None } else { Some(off) },
            released: false,
            rel_coeff: (-1.0 / (release / 4.0 * sr)).exp() as f32,
            atk_frames: ((attack * sr) as f32).max(1.0),
            peak,
            env: 0.0,
            choke: zone0.choke.clone(),
            filter,
            pan: (pl * std::f32::consts::SQRT_2, pr * std::f32::consts::SQRT_2),
            done: false,
            pitch,
            zone: first,
            ratio: transition.map_or(1.0, |(_, scoop, _)| 2f64.powf(-scoop / 1200.0)),
            glide: transition.map(|(_, scoop, _)| Glide {
                from: 2f64.powf(-scoop / 1200.0),
                to: 1.0,
                start: at,
                len: (0.045 * sr).max(1.0),
                k: 1.0,
            }),
            brass,
            eq_rise: transition.is_some(),
            shape,
            silent: false,
            pedal: pedal_on,
            pedal_cap: off + (self.man.pedal_max * sr) as u64,
            pedal_coeff: (-1.0 / (pedal_tau * sr)).exp() as f32,
            thump: None,
        };
        if self.voices.len() >= self.max_voices {
            if let Some(oldest) = self.voices.iter_mut().find(|v| !v.done) {
                oldest.done = true;
            }
        }
        self.voices.retain(|v| !v.done);
        if let Some(group) = zone0.choke {
            self.groups.insert(group, id);
        }
        // the legato transition's old voice fades out quickly
        if let Some((fade, _, old)) = transition {
            if let Some(o) = self.voices.iter_mut().find(|v| v.id == old) {
                o.release_at = Some(at);
                o.rel_coeff = (-1.0 / ((fade / 3.0) * sr)).exp() as f32;
                o.silent = true;
                o.glide = None;
            }
        }
        self.last = Some(id);
        self.voices.push(voice);
    }

    fn pedal(&mut self, down: bool, at: u64) {
        let i = self.pedal_ev.partition_point(|e| e.0 <= at);
        self.pedal_ev.insert(i, (at, down));
    }

    fn bend(&mut self, semis: f64, at: u64) {
        let i = self.bend_ev.partition_point(|e| e.0 <= at);
        self.bend_ev.insert(i, (at, semis));
    }

    fn set(&mut self, name: &str, value: f64) {
        self.params.set(self.spec, name, value);
    }

    fn render(&mut self, l: &mut [f32], r: &mut [f32], start: u64) {
        let rate_hz = self.sr;
        let noise = self.man.release_noise;
        let release_sr = self.sr as f64;
        for i in 0..l.len() {
            let f = start + i as u64;
            // explicit pedal and bend events due at this frame
            while self.pedal_cursor < self.pedal_ev.len() && self.pedal_ev[self.pedal_cursor].0 <= f {
                let (t, down) = self.pedal_ev[self.pedal_cursor];
                self.pedal_cursor += 1;
                self.apply_pedal(down, t);
            }
            while self.bend_cursor < self.bend_ev.len() && self.bend_ev[self.bend_cursor].0 <= f {
                let (_, semis) = self.bend_ev[self.bend_cursor];
                self.bend_cursor += 1;
                self.bend_ratio = 2f64.powf(semis.clamp(-48.0, 48.0) / 12.0);
            }
            let bend = self.bend_ratio;
            let (mut sl, mut sr) = (0.0f32, 0.0f32);
            for v in self.voices.iter_mut() {
                if v.done || f < v.start {
                    continue;
                }
                if let Some(g) = v.glide {
                    if f >= g.start {
                        let t = (f - g.start) as f64 / g.len;
                        if t >= 1.0 {
                            v.ratio = g.to;
                            v.glide = None;
                        } else {
                            v.ratio = g.from * (g.to / g.from).powf(t.powf(g.k));
                        }
                        for layer in v.layers.iter_mut() {
                            layer.rate = layer.base * v.ratio;
                        }
                    }
                }
                if v.shape.vib_cents > 0.0 || bend != 1.0 {
                    // vibrato (depth fades in from 0.3 s to full at 0.9 s) and pitch bend, on every layer
                    let mut factor = bend;
                    if v.shape.vib_cents > 0.0 {
                        let t = (f - v.start) as f64 / rate_hz as f64;
                        let ramp = ((t - 0.3) / 0.6).clamp(0.0, 1.0);
                        let cents = v.shape.vib_cents * ramp * v.shape.vib_phase.sin();
                        v.shape.vib_phase += v.shape.vib_inc;
                        factor *= 2f64.powf(cents / 1200.0);
                    }
                    for layer in v.layers.iter_mut() {
                        layer.rate = layer.base * v.ratio * factor;
                    }
                }
                if !v.released {
                    let x = ((f - v.start) as f32 / v.atk_frames).min(1.0);
                    // legato crossfades rise on an equal-power curve (as the web engine's fade-in)
                    v.env = if v.eq_rise { (x * std::f32::consts::FRAC_PI_2).sin() } else { x };
                    // a pedal-held voice releases at the pedal cap if the pedal never lifts
                    let due = match v.release_at {
                        Some(r) => Some(r),
                        None if v.pedal => Some(v.pedal_cap),
                        None => None,
                    };
                    if matches!(due, Some(r) if f >= r) {
                        v.released = true;
                        v.pedal = false;
                        v.begin_release(release_sr, noise);
                    }
                } else {
                    v.env *= v.rel_coeff;
                }
                if let Some((fc, atk)) = v.brass {
                    // brass: the cutoff opens exponentially from half to full over the attack
                    let t = (f - v.start) as f64 / rate_hz as f64;
                    if t < atk && (f - v.start) % 32 == 0 {
                        let freq = (fc * 0.5 * 2f64.powf(t / atk)).min(fc) as f32;
                        if let Some([fl, fr]) = v.filter.as_mut() {
                            fl.set(FilterKind::Lowpass, rate_hz, freq, 0.5, 0.0);
                            fr.set(FilterKind::Lowpass, rate_hz, freq, 0.5, 0.0);
                        }
                    }
                }
                let sm = v.shape.mult(f, v.start, rate_hz as f64);
                let amp = v.env * v.peak * sm;
                let decayed = v.shape.decay_tau > 0.0 && f >= v.shape.decay_start && sm < 1e-5 && v.thump.is_none();
                if (v.released && amp < 1e-6 && v.thump.is_none()) || decayed {
                    v.done = true;
                    continue;
                }
                let (mut ml, mut mr) = (0.0f32, 0.0f32);
                let mut finished = true;
                for layer in v.layers.iter_mut() {
                    let (a, b) = if layer.audio.channels.len() > 1 {
                        (layer.sample(0), layer.sample(1))
                    } else {
                        let m = layer.sample(0);
                        (m, m)
                    };
                    ml += a * layer.weight * layer.gain;
                    mr += b * layer.weight * layer.gain;
                    layer.tick_gain();
                    layer.advance();
                    finished &= layer.finished();
                }
                if let Some([fl, fr]) = v.filter.as_mut() {
                    ml = fl.process(ml);
                    mr = fr.process(mr);
                }
                let mut th = 0.0f32;
                let mut thump_done = false;
                if let Some(t) = v.thump.as_mut() {
                    th = t.next(rate_hz);
                    thump_done = t.done(rate_hz);
                }
                if thump_done {
                    v.thump = None;
                }
                sl += ml * amp * v.pan.0 + th * v.pan.0;
                sr += mr * amp * v.pan.1 + th * v.pan.1;
                if finished && v.thump.is_none() {
                    v.done = true;
                }
            }
            l[i] += sl;
            r[i] += sr;
        }
        self.voices.retain(|v| !v.done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_set_is_complete() {
        assert!(is_sampled("grandpiano"));
        assert!(is_sampled("808clean"));
        assert!(!is_sampled("synth"));
    }

    #[test]
    fn wrapper_defaults_are_for_known_devices_only() {
        assert_eq!(defaults("organ").len(), 2);
        assert!(defaults("celesta").is_empty());
    }
}
