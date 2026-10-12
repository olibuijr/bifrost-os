//! Offline song render: the port of the mobile engine's scheduling (`engine.js`, `render.js`) and mix chain.
//!
//! Timing: 16th steps; a bar is round(16 * num / den) steps; swing delays odd integer steps by swing * 0.66 step.
//! Songs in arrangement mode play the sections in order (section bpm/meter override the scene's, which override
//! the project's); otherwise the scenes play in order, each for its longest clip rounded up to whole bars, twice.
//! Notes, automation (per quarter step, linear between points) and tempo changes are scheduled per block and
//! rendered through the device traits in `devices`.
//!
//! Mix chain (as in `engine.js`): track instrument -> track effects -> volume^2 (times group volume^2) and
//! equal-power pan -> master bus with headroom (1/sqrt(max(1, active/4)) unless master.headroom is false) ->
//! master effects -> master gain 0.9 * v^2 -> bus compressor (-6 dB, 20:1) -> safety clip (tanh above 0.7).

use crate::devices::{self, Ctx, Effect, Instrument};
use crate::dsp::{pan_gains, safety_clip, Compressor};
use crate::model::{Clip, Project, Section};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_SR: u32 = 48_000;
const T0: f64 = 0.05;
const BLOCK: usize = 512;
const QUARTERS: u32 = 4;

#[derive(Clone, Debug)]
pub struct RenderOpts {
    pub sample_rate: u32,
    /// Directory holding the plugin assets (`data/plugins`).
    pub plugins: PathBuf,
    pub seed: u64,
    /// Scene mode: plays each scene `loops` times (default 2).
    pub loops: u32,
    /// Extra time after the last note, in seconds (reverb tails).
    pub tail: f64,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts { sample_rate: DEFAULT_SR, plugins: crate::assets::asset_root().join("plugins"), seed: 0x5EED, loops: 2, tail: 2.0 }
    }
}

/// Stereo render result.
pub struct Rendered {
    pub sample_rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Start (seconds) of every bar.
    pub bars: Vec<f64>,
    /// Instruments and effects that could not be built (their tracks or devices are silent).
    pub missing: Vec<String>,
    /// End of the musical content, in seconds (before the tail).
    pub content_end: f64,
}

/// One scheduled section: a scene (or arrangement section) with its meter and tempo.
#[derive(Clone, Debug)]
pub struct Segment {
    pub scene: String,
    pub s0: u64,
    pub steps: u64,
    pub spb: u64,
    pub bpm: Option<f64>,
    pub mutes: Vec<String>,
}

fn clean_sig(sig: Option<[u32; 2]>) -> Option<[u32; 2]> {
    let [n, d] = sig?;
    (1..=16).contains(&n).then_some(())?;
    matches!(d, 2 | 4 | 8 | 16).then_some([n, d])
}

fn clean_bpm(b: Option<f64>) -> Option<f64> {
    b.filter(|b| (40.0..=240.0).contains(b))
}

/// Steps per bar for a meter (16 steps = one 4/4 bar).
pub fn sig_steps(sig: Option<[u32; 2]>) -> u64 {
    match sig {
        Some([n, d]) => ((16.0 * n as f64 / d as f64).round() as u64).clamp(2, 64),
        None => 16,
    }
}

/// The arrangement (song mode) as segments, skipping sections whose scene is gone.
pub fn arrangement_segments(proj: &Project) -> Vec<Segment> {
    let mut out = vec![];
    let mut st = 0u64;
    for a in &proj.arrangement {
        let Some(sc) = proj.scenes.iter().find(|s| s.id == a.scene_id) else { continue };
        let bars = (a.bars as f64).round().max(1.0) as u64;
        let spb = sig_steps(clean_sig(section_sig(a)).or(clean_sig(sc.sig)));
        let bpm = clean_bpm(a.bpm).or(clean_bpm(sc.bpm));
        let mutes = a.mutes.clone().unwrap_or_default();
        out.push(Segment { scene: a.scene_id.clone(), s0: st, steps: bars * spb, spb, bpm, mutes });
        st += bars * spb;
    }
    out
}

fn section_sig(a: &Section) -> Option<[u32; 2]> {
    a.sig
}

/// The scene list as segments (no arrangement): each scene for `loops` times its longest clip, in whole bars.
pub fn scene_segments(proj: &Project, loops: u32, scenes: Option<&[String]>) -> Vec<Segment> {
    let mut out = vec![];
    let mut st = 0u64;
    let wanted: Vec<String> = match scenes {
        Some(list) => list.to_vec(),
        None => proj.scenes.iter().map(|s| s.id.clone()).collect(),
    };
    for id in wanted {
        if !proj.tracks.iter().any(|t| t.clips.contains_key(&id)) {
            continue;
        }
        let Some(sc) = proj.scenes.iter().find(|s| s.id == id) else { continue };
        let spb = sig_steps(clean_sig(sc.sig));
        let longest = proj.tracks.iter().filter_map(|t| t.clips.get(&id)).map(|c| c.length as u64).max().unwrap_or(0);
        let bars = longest.div_ceil(spb).max(1);
        let steps = spb * bars * loops.max(1) as u64;
        out.push(Segment { scene: id, s0: st, steps, spb, bpm: clean_bpm(sc.bpm), mutes: vec![] });
        st += steps;
    }
    out
}

/// Start time of every step (seconds) plus the end of the last step.
pub fn step_times(proj: &Project, segs: &[Segment]) -> (Vec<f64>, f64) {
    let total: u64 = segs.iter().map(|s| s.steps).sum();
    let mut times = Vec::with_capacity(total as usize + 1);
    let base = proj.bpm.clamp(40.0, 240.0);
    let mut t = T0;
    for seg in segs {
        let sd = 60.0 / seg.bpm.unwrap_or(base) / 4.0;
        for _ in 0..seg.steps {
            times.push(t);
            t += sd;
        }
    }
    times.push(t);
    (times, t)
}

/// Deterministic per-note chance (the probability check of `engine.js`), in 0..1.
fn chance(clip_id: &str, note_index: usize, loop_no: u64) -> f64 {
    let mut h: u32 = 2_166_136_261;
    for b in clip_id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    h = (h ^ note_index as u32).wrapping_mul(16_777_619);
    h = (h ^ loop_no as u32).wrapping_mul(16_777_619);
    h ^= h >> 15;
    h = h.wrapping_mul(2_246_822_507);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_909);
    h ^= h >> 16;
    h as f64 / 4_294_967_296.0
}

/// Linear interpolation of automation points at position `t` (steps); holds before the first and after the last.
pub fn value_at(points: &[(f64, f64)], t: f64) -> Option<f64> {
    let (first, last) = (points.first()?, points.last()?);
    if t <= first.0 {
        return Some(first.1);
    }
    if t > last.0 {
        return Some(last.1);
    }
    let i = points.iter().position(|p| p.0 >= t)?.max(1);
    let (a, b) = (points[i - 1], points[i]);
    Some(if b.0 == a.0 { b.1 } else { a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0) })
}

/// Automation lanes of one clip, keyed "inst.<param>" or "<effectId>.<param>", as sorted (t, v) points.
fn lanes_of(clip: &Clip) -> BTreeMap<String, Vec<(f64, f64)>> {
    let mut out = BTreeMap::new();
    if let Some(a) = &clip.automation {
        for (k, pts) in a {
            let mut v: Vec<(f64, f64)> = pts.iter().map(|p| (p.t, p.v)).collect();
            v.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
            if !v.is_empty() {
                out.insert(k.clone(), v);
            }
        }
    }
    out
}

#[derive(Clone, Copy)]
struct NoteEv {
    frame: u64,
    track: usize,
    pitch: f64,
    vel: f64,
    dur: u64,
}

#[derive(Clone)]
enum Target {
    Inst(String),
    Fx(usize, String),
}

/// A clip's sustain or bend lane point, delivered to the track's instrument.
#[derive(Clone, Copy)]
struct LaneEv {
    frame: u64,
    track: usize,
    kind: LaneKind,
}

#[derive(Clone, Copy)]
enum LaneKind {
    Pedal(bool),
    Bend(f64),
}

/// Sustain (CC64) and pitch-bend lanes of a clip: `cc` = {"64": [[t, 0..127]], "bend": [[t, semitones]]}, t in steps.
fn cc_lanes(clip: &Clip) -> (Option<Vec<(f64, f64)>>, Option<Vec<(f64, f64)>>) {
    let read = |key: &str| -> Option<Vec<(f64, f64)>> {
        let arr = clip.cc.as_ref()?.get(key)?.as_array()?;
        let mut v: Vec<(f64, f64)> = arr
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
            })
            .collect();
        v.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        (!v.is_empty()).then_some(v)
    };
    (read("64"), read("bend"))
}

#[derive(Clone)]
struct ParamEv {
    frame: u64,
    track: usize,
    target: Target,
    value: f64,
}

struct TrackState {
    instrument: Option<Box<dyn Instrument>>,
    effects: Vec<(String, Box<dyn Effect>)>,
    gain: f32,
    pan: (f32, f32),
    bufs_l: Vec<f32>,
    bufs_r: Vec<f32>,
}

/// Renders a project to stereo f32 samples.
pub fn render(proj: &Project, opts: &RenderOpts) -> Rendered {
    let sr = opts.sample_rate as f32;
    let segs = if proj.arrangement.is_empty() { scene_segments(proj, opts.loops, None) } else { arrangement_segments(proj) };
    let (times, end) = step_times(proj, &segs);
    let frame_of = |secs: f64| (secs * sr as f64).round() as u64;

    let ctx = Ctx { sr, plugins: opts.plugins.clone(), seed: opts.seed };
    let mut missing = vec![];
    let any_solo = proj.tracks.iter().any(|t| t.solo);
    let active = proj.tracks.iter().filter(|t| !t.mute && t.volume > 0.01).count() as f32;
    let headroom = if proj.master.as_ref().is_some_and(|m| m.extra.get("headroom").and_then(|v| v.as_bool()) == Some(false)) {
        1.0
    } else {
        1.0 / (active.max(1.0) / 4.0).sqrt()
    };

    // tracks
    let mut tracks: Vec<TrackState> = Vec::with_capacity(proj.tracks.len());
    for t in &proj.tracks {
        let instrument = devices::instrument(&t.instrument, &ctx);
        if instrument.is_none() {
            missing.push(format!("{} ({})", t.instrument.kind, t.name));
        }
        let effects = t
            .effects
            .iter()
            .map(|e| (e.id.clone(), devices::effect(&e.kind, &e.params, &ctx)))
            .collect();
        let group = proj.groups.as_ref().and_then(|gs| {
            let name = t.group.as_ref()?;
            gs.iter().find(|g| g.get("name").and_then(|n| n.as_str()) == Some(name.as_str()))
        });
        let group_volume = group.and_then(|g| g.get("volume")).and_then(|v| v.as_f64()).unwrap_or(1.0);
        let audible = !t.mute && (!any_solo || t.solo);
        let v = t.volume * group_volume;
        let gain = if audible { (v * v) as f32 } else { 0.0 };
        tracks.push(TrackState {
            instrument,
            effects,
            gain,
            pan: pan_gains(t.pan.clamp(-1.0, 1.0) as f32),
            bufs_l: vec![0.0; BLOCK],
            bufs_r: vec![0.0; BLOCK],
        });
    }
    let mut master_fx: Vec<(String, Box<dyn Effect>)> = proj
        .master
        .as_ref()
        .map(|m| m.effects.iter().map(|e| (e.id.clone(), devices::effect(&e.kind, &e.params, &ctx))).collect())
        .unwrap_or_default();
    for e in proj.master.iter().flat_map(|m| m.effects.iter()) {
        if devices::effect_spec(&e.kind).is_none() {
            missing.push(format!("master {}", e.kind));
        }
    }
    let master_volume = proj.master.as_ref().map_or(1.0, |m| m.volume);
    let master_gain = (0.9 * master_volume * master_volume) as f32;
    let mut bus = Compressor::new(sr);
    bus.threshold = -6.0;
    bus.ratio = 20.0;
    bus.knee = 0.0;
    bus.attack = 0.003;
    bus.release = 0.12;

    // events, in frame order
    let mut notes: Vec<NoteEv> = vec![];
    let mut params: Vec<ParamEv> = vec![];
    let mut lane_evs: Vec<LaneEv> = vec![];
    let mut tempo: Vec<(u64, f64)> = vec![];
    let swing = proj.swing.clamp(0.0, 0.6);
    let track_index: BTreeMap<&str, usize> = proj.tracks.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
    let mut bars = vec![];

    for seg in &segs {
        let bpm = seg.bpm.unwrap_or(proj.bpm);
        tempo.push((frame_of(times[seg.s0 as usize]), bpm));
        for k in 0..seg.steps / seg.spb.max(1) {
            bars.push(times[(seg.s0 + k * seg.spb) as usize]);
        }
        for (ti, t) in proj.tracks.iter().enumerate() {
            let Some(clip) = t.clips.get(&seg.scene) else { continue };
            if clip.length == 0 || seg.mutes.iter().any(|m| m == &t.id) {
                continue;
            }
            let lanes = lanes_of(clip);
            let (pedal_pts, bend_pts) = cc_lanes(clip);
            let by_step = bucket_notes(clip);
            let len = clip.length as u64;
            for s in 0..seg.steps {
                let gs = seg.s0 + s;
                let local = s % len;
                let step_t = times[gs as usize];
                let sd = times[gs as usize + 1] - step_t;
                let loop_no = s / len;
                if let Some(bucket) = by_step.get(&(local as u32)) {
                    for &(idx, ref n) in bucket {
                        if let Some(prob) = n.prob {
                            if prob < 1.0 && chance(&clip.id, idx, loop_no) >= prob {
                                continue;
                            }
                        }
                        let mut at = step_t + (n.t - local as f64) * sd;
                        if n.t.fract() == 0.0 && (n.t as i64) % 2 == 1 {
                            at += swing * sd * 0.66;
                        }
                        notes.push(NoteEv { frame: frame_of(at), track: ti, pitch: n.p, vel: n.v, dur: frame_of((n.d * sd).max(0.0)) });
                    }
                }
                for q in 0..QUARTERS {
                    let lt = local as f64 + q as f64 / QUARTERS as f64;
                    let at = frame_of(step_t + sd * q as f64 / QUARTERS as f64);
                    for (key, pts) in &lanes {
                        let Some((dev, param)) = key.rsplit_once('.') else { continue };
                        let Some(value) = value_at(pts, lt) else { continue };
                        let target = if dev == "inst" {
                            Target::Inst(param.to_string())
                        } else {
                            match t.effects.iter().position(|e| e.id == dev) {
                                Some(fi) => Target::Fx(fi, param.to_string()),
                                None => continue,
                            }
                        };
                        params.push(ParamEv { frame: at, track: ti, target, value });
                    }
                    if let Some(pts) = &pedal_pts {
                        if let Some(v) = value_at(pts, lt) {
                            lane_evs.push(LaneEv { frame: at, track: ti, kind: LaneKind::Pedal(v >= 64.0) });
                        }
                    }
                    if let Some(pts) = &bend_pts {
                        if let Some(v) = value_at(pts, lt) {
                            lane_evs.push(LaneEv { frame: at, track: ti, kind: LaneKind::Bend(v) });
                        }
                    }
                }
            }
        }
    }
    let _ = &track_index;
    notes.sort_by_key(|n| n.frame);
    params.sort_by_key(|p| p.frame);
    lane_evs.sort_by_key(|l| l.frame);
    tempo.sort_by_key(|t| t.0);

    // tempo-synced devices know the transport phase (origin = first step)
    let origin = frame_of(T0);
    for tr in tracks.iter_mut() {
        if let Some(ins) = tr.instrument.as_mut() {
            ins.tempo(proj.bpm, origin);
        }
        for (_, fx) in tr.effects.iter_mut() {
            fx.tempo(proj.bpm, origin);
        }
    }

    let total_frames = frame_of(end + opts.tail);
    let mut left = vec![0.0f32; total_frames as usize + BLOCK];
    let mut right = vec![0.0f32; total_frames as usize + BLOCK];
    let (mut ni, mut pi, mut ti) = (0usize, 0usize, 0usize);
    let mut li = 0usize;
    let mut block_start = 0u64;
    while block_start < total_frames {
        let block_end = (block_start + BLOCK as u64).min(total_frames);
        let n = (block_end - block_start) as usize;

        // tempo changes starting in this block
        while ti < tempo.len() && tempo[ti].0 < block_end {
            let bpm_now = tempo[ti].1;
            for tr in tracks.iter_mut() {
                if let Some(ins) = tr.instrument.as_mut() {
                    ins.tempo(bpm_now, origin);
                }
                for (_, fx) in tr.effects.iter_mut() {
                    fx.tempo(bpm_now, origin);
                }
            }
            ti += 1;
        }
        // automation within this block (applied at block granularity)
        while pi < params.len() && params[pi].frame < block_end {
            let ev = params[pi].clone();
            pi += 1;
            let tr = &mut tracks[ev.track];
            match ev.target {
                Target::Inst(name) => {
                    if let Some(ins) = tr.instrument.as_mut() {
                        ins.set(&name, ev.value);
                    }
                }
                Target::Fx(fi, name) => {
                    if let Some((_, fx)) = tr.effects.get_mut(fi) {
                        fx.set(&name, ev.value);
                    }
                }
            }
        }
        // sustain and bend lanes first, so a note starting in this block sees the pedal state at its start
        while li < lane_evs.len() && lane_evs[li].frame < block_end {
            let ev = lane_evs[li];
            li += 1;
            if let Some(ins) = tracks[ev.track].instrument.as_mut() {
                match ev.kind {
                    LaneKind::Pedal(down) => ins.pedal(down, ev.frame),
                    LaneKind::Bend(semis) => ins.bend(semis, ev.frame),
                }
            }
        }
        // note-ons that start before this block ends (voices wait for their start frame)
        while ni < notes.len() && notes[ni].frame < block_end {
            let nev = notes[ni];
            ni += 1;
            if let Some(ins) = tracks[nev.track].instrument.as_mut() {
                ins.note_on(nev.pitch, nev.vel, nev.frame, nev.dur.max(1));
            }
        }

        // render each track, then sum it into the master bus
        let mut master_l = vec![0.0f32; n];
        let mut master_r = vec![0.0f32; n];
        for tr in tracks.iter_mut() {
            let tl = &mut tr.bufs_l[..n];
            let trr = &mut tr.bufs_r[..n];
            tl.iter_mut().for_each(|x| *x = 0.0);
            trr.iter_mut().for_each(|x| *x = 0.0);
            if let Some(ins) = tr.instrument.as_mut() {
                ins.render(tl, trr, block_start);
            }
            for (_, fx) in tr.effects.iter_mut() {
                fx.process(block_start, tl, trr);
            }
            if tr.gain == 0.0 {
                continue;
            }
            let (gl, gr) = (tr.pan.0 * tr.gain * std::f32::consts::SQRT_2, tr.pan.1 * tr.gain * std::f32::consts::SQRT_2);
            for i in 0..n {
                master_l[i] += tl[i] * gl;
                master_r[i] += trr[i] * gr;
            }
        }
        for i in 0..n {
            master_l[i] *= headroom;
            master_r[i] *= headroom;
        }
        // master effects run on the whole block (one call per effect, not per sample)
        for (_, fx) in master_fx.iter_mut() {
            fx.process(block_start, &mut master_l, &mut master_r);
        }
        for i in 0..n {
            let l = master_l[i] * master_gain;
            let r = master_r[i] * master_gain;
            let g = bus.gain(l, r);
            left[(block_start as usize) + i] = safety_clip(l * g);
            right[(block_start as usize) + i] = safety_clip(r * g);
        }
        block_start = block_end;
    }

    let content_frames = (frame_of(end) as usize).min(left.len());
    let _ = content_frames;
    Rendered {
        sample_rate: opts.sample_rate,
        left,
        right,
        bars,
        missing,
        content_end: end,
    }
}

/// Notes grouped by their integer step (floor(t)), keeping each note's index in the clip.
fn bucket_notes(clip: &Clip) -> BTreeMap<u32, Vec<(usize, crate::model::Note)>> {
    let mut m: BTreeMap<u32, Vec<(usize, crate::model::Note)>> = BTreeMap::new();
    for (i, n) in clip.notes.iter().enumerate() {
        m.entry(n.t.floor().max(0.0) as u32).or_default().push((i, n.clone()));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_sets_steps_per_bar() {
        assert_eq!(sig_steps(None), 16);
        assert_eq!(sig_steps(Some([3, 4])), 12);
        assert_eq!(sig_steps(Some([7, 8])), 14);
    }

    #[test]
    fn automation_interpolates_and_holds() {
        let pts = [(0.0, 0.0), (16.0, 1.0)];
        assert_eq!(value_at(&pts, -1.0), Some(0.0));
        assert!((value_at(&pts, 8.0).unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(value_at(&pts, 99.0), Some(1.0));
    }

    #[test]
    fn chance_is_deterministic_and_uniform_enough() {
        assert_eq!(chance("clip", 3, 1), chance("clip", 3, 1));
        let below = (0..1000).filter(|&i| chance("c", i, 0) < 0.5).count();
        assert!((400..600).contains(&below), "{below}");
    }
}
