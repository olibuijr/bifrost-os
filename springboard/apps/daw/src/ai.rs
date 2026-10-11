//! The AI co-producer: asks the Anthropic Messages API for a JSON reply `{ "reply": "...", "ops": [...] }` and
//! applies the operations to the project. The key is read at runtime from the opencode auth store
//! (`~/.local/share/opencode/auth.json`, provider "anthropic"); it is never copied or logged.
//!
//! Operations ported so far: set_bpm, set_swing, rename_track, set_mix, set_clip, clear_clip. The rest of the
//! mobile `applyOps` list (scenes, arrangement, instruments, effects, compose) is not ported yet and is reported
//! back as unsupported.

use crate::model::{Clip, Extra, Note, Project};
use serde_json::{json, Value};
use springboard::deildu::http::{Client, Jar, Tls, Transport};
use std::path::PathBuf;
use std::sync::Arc;

pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-opus-5-5";

const SYSTEM: &str = r#"You are the co-producer of a DAW. Reply with ONLY a JSON object:
{"reply": "<short message to the musician>", "ops": [ ... ]}
Operations (applied in order):
- {"op":"set_bpm","bpm":120}                         (40..220)
- {"op":"set_swing","swing":0.2}                     (0..0.6)
- {"op":"rename_track","track":"<id|name|index>","name":"..."}
- {"op":"set_mix","track":"<id|name|index>","volume":0.8,"pan":0,"mute":false,"solo":false}   (volume 0..1.2, pan -1..1)
- {"op":"set_clip","track":"<id|name|index>","scene":"<id|name|index, 0-based>","length":16,"name":"...",
   "notes":[{"t":0,"d":2,"p":60,"v":0.8}]}   (t and d in 16th steps, p = MIDI pitch 0..127, v = velocity 0..1)
- {"op":"clear_clip","track":"...","scene":"..."}
- {"op":"generate","track":"...","scene":"...","kind":"progression|arp|line|groove","prog":"i VI III VII","bars":4,
   "octave":3,"roll":0,"rate":2,"pattern":"up|down|updown|downup","line":"E5:2 D5:1 r:1 C5:4","groove":"boombap|trap|triphop|dnb|house|lofi|reggaeton|rock|dubstep_halftime",
   "post":[{"fx":"humanize","time":0.06,"vel":0.05,"seed":1},{"fx":"dynamics","points":[[0,0.6],[16,1.0]]},{"fx":"pedal","from":0,"to":16},{"fx":"pedal_every","every":8}]}
Use only the tracks and scenes listed. Keep the reply short."#;

/// Path of the opencode auth store (XDG data home aware).
pub fn auth_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("opencode/auth.json")
}

/// The Anthropic key from the opencode auth store. Never printed.
pub fn load_key() -> Result<String, String> {
    let text = std::fs::read_to_string(auth_path()).map_err(|e| format!("auth store: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("auth store: {e}"))?;
    v.get("anthropic")
        .and_then(|a| a.get("key"))
        .and_then(|k| k.as_str())
        .filter(|k| !k.is_empty())
        .map(String::from)
        .ok_or_else(|| "no anthropic key in the opencode auth store".to_string())
}

/// A short description of the song for the model: tempo, tracks, scenes, and clip sizes.
pub fn describe(project: &Project) -> String {
    let mut s = format!("Song \"{}\": {} BPM, swing {:.2}\nScenes:\n", project.name, project.bpm, project.swing);
    for (i, sc) in project.scenes.iter().enumerate() {
        s += &format!("  {i}: {} (id {})\n", sc.name, sc.id);
    }
    s += "Tracks:\n";
    for (i, t) in project.tracks.iter().enumerate() {
        let clips: Vec<String> = project
            .scenes
            .iter()
            .enumerate()
            .filter_map(|(si, sc)| t.clips.get(&sc.id).map(|c| format!("scene {si}: {} notes, {} steps", c.notes.len(), c.length)))
            .collect();
        s += &format!(
            "  {i}: {} (id {}, {}, volume {:.2}, pan {:.2}{}{})  {}\n",
            t.name,
            t.id,
            t.instrument.kind,
            t.volume,
            t.pan,
            if t.mute { ", muted" } else { "" },
            if t.solo { ", solo" } else { "" },
            clips.join("; ")
        );
    }
    s
}

/// The model's reply, parsed.
#[derive(Debug)]
pub struct Reply {
    pub text: String,
    pub ops: Vec<Value>,
}

/// Sends one request and parses the reply. `model` defaults to DAW_MODEL or claude-opus-5-5.
pub fn ask(project: &Project, prompt: &str, model: Option<&str>) -> Result<Reply, String> {
    let key = load_key()?;
    let model = model.map(String::from).or_else(|| std::env::var("DAW_MODEL").ok()).unwrap_or_else(|| DEFAULT_MODEL.into());
    let body = json!({
        "model": model,
        "max_tokens": 4096,
        "system": SYSTEM,
        "messages": [{"role": "user", "content": format!("{}\n\nMusician: {}", describe(project), prompt)}],
    });
    let client = Client::new(Arc::new(Tls::new()) as Arc<dyn Transport>, Jar::default()).require_https();
    let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let resp = client
        .post_with_headers(
            API_URL,
            &[("x-api-key", key.as_str()), ("anthropic-version", API_VERSION), ("content-type", "application/json")],
            &bytes,
            4 * 1024 * 1024,
        )
        .map_err(|e| format!("request failed: {e}"))?;
    if resp.status != 200 {
        return Err(format!("anthropic answered {}", resp.status));
    }
    let answer: Value = serde_json::from_slice(&resp.body).map_err(|e| format!("reply: {e}"))?;
    let text = answer
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| blocks.iter().find_map(|b| b.get("text").and_then(|t| t.as_str())))
        .ok_or_else(|| "reply has no text".to_string())?;
    parse_reply(text)
}

/// Extracts the JSON object from the model's text.
pub fn parse_reply(text: &str) -> Result<Reply, String> {
    let start = text.find('{').ok_or("reply has no JSON")?;
    let end = text.rfind('}').ok_or("reply has no JSON")?;
    let v: Value = serde_json::from_str(&text[start..=end]).map_err(|e| format!("reply JSON: {e}"))?;
    Ok(Reply {
        text: v.get("reply").and_then(|r| r.as_str()).unwrap_or("").to_string(),
        ops: v.get("ops").and_then(|o| o.as_array()).cloned().unwrap_or_default(),
    })
}

fn find_track(project: &Project, key: &Value) -> Option<usize> {
    if let Some(i) = key.as_u64() {
        return ((i as usize) < project.tracks.len()).then_some(i as usize);
    }
    let s = key.as_str()?;
    project.tracks.iter().position(|t| t.id == s).or_else(|| project.tracks.iter().position(|t| t.name.eq_ignore_ascii_case(s))).or_else(|| s.parse::<usize>().ok().filter(|&i| i < project.tracks.len()))
}

fn find_scene(project: &Project, key: &Value) -> Option<String> {
    if let Some(i) = key.as_u64() {
        return project.scenes.get(i as usize).map(|s| s.id.clone());
    }
    let s = key.as_str()?;
    project
        .scenes
        .iter()
        .find(|sc| sc.id == s || sc.name.eq_ignore_ascii_case(s))
        .or_else(|| s.parse::<usize>().ok().and_then(|i| project.scenes.get(i)))
        .map(|sc| sc.id.clone())
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64()).filter(|x| x.is_finite())
}

fn parse_notes(v: &Value) -> Vec<Note> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    Some(Note {
                        t: num(n, "t")?.max(0.0),
                        d: num(n, "d").unwrap_or(1.0).max(0.0),
                        p: num(n, "p")?.clamp(0.0, 127.0),
                        v: num(n, "v").unwrap_or(0.8).clamp(0.0, 1.0),
                        prob: num(n, "prob"),
                        extra: Extra::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Applies the supported operations. Returns (applied, unsupported op names or problems).
pub fn apply_ops(project: &mut Project, ops: &[Value]) -> (usize, Vec<String>) {
    let mut applied = 0;
    let mut skipped = vec![];
    for op in ops {
        let name = op.get("op").and_then(|o| o.as_str()).unwrap_or("").to_string();
        let ok = match name.as_str() {
            "set_bpm" => num(op, "bpm").map(|b| project.bpm = b.clamp(40.0, 220.0)).is_some(),
            "set_swing" => num(op, "swing").map(|s| project.swing = s.clamp(0.0, 0.6)).is_some(),
            "rename_track" => match (op.get("track").and_then(|t| find_track(project, t)), op.get("name").and_then(|n| n.as_str())) {
                (Some(i), Some(n)) => {
                    project.tracks[i].name = n.to_string();
                    true
                }
                _ => false,
            },
            "set_mix" => match op.get("track").and_then(|t| find_track(project, t)) {
                Some(i) => {
                    let t = &mut project.tracks[i];
                    if let Some(v) = num(op, "volume") {
                        t.volume = v.clamp(0.0, 1.2);
                    }
                    if let Some(p) = num(op, "pan") {
                        t.pan = p.clamp(-1.0, 1.0);
                    }
                    if let Some(m) = op.get("mute").and_then(|m| m.as_bool()) {
                        t.mute = m;
                    }
                    if let Some(s) = op.get("solo").and_then(|m| m.as_bool()) {
                        t.solo = s;
                    }
                    true
                }
                None => false,
            },
            "set_clip" => {
                let track = op.get("track").and_then(|t| find_track(project, t));
                let scene = op.get("scene").and_then(|s| find_scene(project, s));
                match (track, scene) {
                    (Some(ti), Some(sid)) => {
                        let notes = parse_notes(op.get("notes").unwrap_or(&Value::Null));
                        let t = &mut project.tracks[ti];
                        let clip = t.clips.entry(sid.clone()).or_insert_with(|| Clip {
                            id: format!("{}-{}", t.id, sid),
                            name: String::new(),
                            length: 16,
                            notes: vec![],
                            automation: None,
                            cc: None,
                            extra: Extra::new(),
                        });
                        clip.notes = notes;
                        // sustain (cc "64") and pitch-bend ("bend") lanes; absent lanes clear the clip's lanes
                        let mut lanes = serde_json::Map::new();
                        if let Some(c) = op.get("cc").and_then(|c| c.as_object()) {
                            for (k, v) in c {
                                lanes.insert(k.clone(), v.clone());
                            }
                        }
                        if let Some(b) = op.get("bend") {
                            lanes.insert("bend".to_string(), b.clone());
                        }
                        clip.cc = (!lanes.is_empty()).then(|| Value::Object(lanes));
                        if let Some(len) = num(op, "length") {
                            clip.length = (len as u32).clamp(4, 4096);
                        }
                        if let Some(n) = op.get("name").and_then(|n| n.as_str()) {
                            clip.name = n.to_string();
                        }
                        true
                    }
                    _ => false,
                }
            }
            "clear_clip" => {
                let track = op.get("track").and_then(|t| find_track(project, t));
                let scene = op.get("scene").and_then(|s| find_scene(project, s));
                match (track, scene) {
                    (Some(ti), Some(sid)) => {
                        project.tracks[ti].clips.remove(&sid);
                        true
                    }
                    _ => false,
                }
            }
            _ => apply_model_op(project, &name, op),
        };
        if ok {
            applied += 1;
        } else {
            skipped.push(if name.is_empty() { "(unnamed op)".into() } else { name });
        }
    }
    (applied, skipped)
}


pub fn new_id(prefix: &str, seed: usize) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}{:x}{:x}", t as u64 & 0xFFFFFF, seed)
}

/// Default parameter object for a device type, from its spec.
pub fn default_params(kind: &str, given: Option<&Value>) -> Extra {
    let mut out = Extra::new();
    if let Some(spec) = crate::devices::instrument_spec(kind).or_else(|| crate::devices::effect_spec(kind)) {
        for p in spec {
            out.insert(p.name.to_string(), json!(p.def));
        }
    }
    if let Some(obj) = given.and_then(|g| g.as_object()) {
        for (k, v) in obj {
            if v.is_number() {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// The operations past the core set: tracks, instruments, effects, master, scenes, arrangement, key.
fn apply_model_op(project: &mut Project, name: &str, op: &Value) -> bool {
    use crate::model::{Effect, Instrument, Scene, Section, Track};
    match name {
        "add_track" => {
            let Some(kind) = op.get("instrument").and_then(|v| v.as_str()) else { return false };
            if crate::devices::instrument_spec(kind).is_none() {
                return false;
            }
            let n = project.tracks.len();
            let id = new_id("t", n);
            let name = op.get("name").and_then(|v| v.as_str()).unwrap_or(kind).to_string();
            let color = op.get("color").and_then(|v| v.as_str()).unwrap_or("#8e8e93").to_string();
            let effects = op
                .get("effects")
                .and_then(|v| v.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|e| {
                            let kind = e.as_str().or_else(|| e.get("type").and_then(|t| t.as_str()))?;
                            crate::devices::effect_spec(kind)?;
                            Some(Effect { id: new_id("e", n), kind: kind.to_string(), on: true, params: default_params(kind, None), extra: Extra::new() })
                        })
                        .collect()
                })
                .unwrap_or_default();
            project.tracks.push(Track {
                id,
                name,
                color,
                group: None,
                instrument: Instrument { kind: kind.to_string(), params: default_params(kind, op.get("params")), sample: None, pads: None, extra: Extra::new() },
                effects,
                volume: 0.8,
                pan: 0.0,
                mute: false,
                solo: false,
                frozen: None,
                clips: Default::default(),
                extra: Extra::new(),
            });
            true
        }
        "remove_track" => match op.get("track").and_then(|t| find_track(project, t)) {
            Some(i) => {
                project.tracks.remove(i);
                true
            }
            None => false,
        },
        "set_instrument" => {
            let Some(i) = op.get("track").and_then(|t| find_track(project, t)) else { return false };
            let Some(kind) = op.get("instrument").and_then(|v| v.as_str()) else { return false };
            if crate::devices::instrument_spec(kind).is_none() {
                return false;
            }
            let inst = &mut project.tracks[i].instrument;
            inst.kind = kind.to_string();
            inst.params = default_params(kind, op.get("params"));
            if let Some(p) = op.get("pads") {
                inst.pads = Some(p.clone());
            }
            if let Some(s) = op.get("sample") {
                inst.sample = Some(s.clone());
            }
            true
        }
        "set_param" => {
            let Some(i) = op.get("track").and_then(|t| find_track(project, t)) else { return false };
            let Some(param) = op.get("param").and_then(|p| p.as_str()) else { return false };
            let Some(value) = num(op, "value") else { return false };
            let device = op.get("device").and_then(|d| d.as_str());
            let t = &mut project.tracks[i];
            let target = match device {
                None | Some("inst") => Some(&mut t.instrument.params),
                Some(id) => t.effects.iter_mut().find(|e| e.id == id).map(|e| &mut e.params),
            };
            match target {
                Some(params) => {
                    params.insert(param.to_string(), json!(value));
                    true
                }
                None => false,
            }
        }
        "add_effect" => {
            let Some(i) = op.get("track").and_then(|t| find_track(project, t)) else { return false };
            let Some(kind) = op.get("type").and_then(|v| v.as_str()) else { return false };
            if crate::devices::effect_spec(kind).is_none() {
                return false;
            }
            let id = new_id("e", project.tracks[i].effects.len());
            project.tracks[i].effects.push(Effect { id, kind: kind.to_string(), on: true, params: default_params(kind, op.get("params")), extra: Extra::new() });
            true
        }
        "remove_effect" => {
            let Some(i) = op.get("track").and_then(|t| find_track(project, t)) else { return false };
            let Some(id) = op.get("effect").and_then(|v| v.as_str()) else { return false };
            let before = project.tracks[i].effects.len();
            project.tracks[i].effects.retain(|e| e.id != id);
            project.tracks[i].effects.len() != before
        }
        "set_master" => {
            let master = project.master.get_or_insert_with(|| crate::model::Master { volume: 0.9, effects: vec![], extra: Extra::new() });
            if let Some(v) = num(op, "volume") {
                master.volume = v.clamp(0.0, 1.2);
            }
            true
        }
        "set_key" => {
            let key = num(op, "key").map(|k| k.clamp(0.0, 11.0));
            let scale = op.get("scale").and_then(|s| s.as_str()).map(String::from);
            if key.is_none() && scale.is_none() {
                return false;
            }
            if let Some(k) = key {
                project.key = Some(k);
            }
            if let Some(s) = scale {
                project.scale = Some(s);
            }
            true
        }
        "add_scene" => {
            let n = project.scenes.len();
            let id = new_id("s", n);
            let name = op.get("name").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| format!("Scene {}", n + 1));
            project.scenes.push(Scene { id, name, bpm: None, sig: None, color: None, extra: Extra::new() });
            true
        }
        "rename_scene" => {
            let Some(id) = op.get("scene").and_then(|s| find_scene(project, s)) else { return false };
            let Some(name) = op.get("name").and_then(|n| n.as_str()) else { return false };
            if let Some(sc) = project.scenes.iter_mut().find(|s| s.id == id) {
                sc.name = name.to_string();
            }
            true
        }
        "remove_scene" => {
            let Some(id) = op.get("scene").and_then(|s| find_scene(project, s)) else { return false };
            project.scenes.retain(|s| s.id != id);
            project.arrangement.retain(|a| a.scene_id != id);
            for t in project.tracks.iter_mut() {
                t.clips.remove(&id);
            }
            true
        }
        "set_arrangement" => {
            let Some(list) = op.get("sections").and_then(|v| v.as_array()) else { return false };
            let mut out = vec![];
            for s in list {
                let Some(sid) = s.get("scene").and_then(|k| find_scene(project, k)) else { continue };
                let bars = num(s, "bars").unwrap_or(4.0).round().clamp(1.0, 512.0) as u32;
                out.push(Section {
                    scene_id: sid,
                    bars,
                    bpm: num(s, "bpm"),
                    sig: None,
                    mutes: s.get("mutes").and_then(|m| m.as_array()).map(|m| m.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
                    label: None,
                    extra: Extra::new(),
                });
            }
            project.arrangement = out;
            true
        }
        "generate" => {
            let (Some(ti), Some(sid)) = (
                op.get("track").and_then(|t| find_track(project, t)),
                op.get("scene").and_then(|s| find_scene(project, s)),
            ) else {
                return false;
            };
            let kind = op.get("kind").and_then(|k| k.as_str()).unwrap_or("progression");
            let bars = num(op, "bars").unwrap_or(4.0).clamp(1.0, 64.0) as u32;
            let key = project.key.unwrap_or(0.0) as i32;
            let scale_name = project.scale.clone().unwrap_or_else(|| "minor".into());
            let notes: Option<Vec<crate::model::Note>> = match kind {
                "progression" => {
                    let prog = op.get("prog").and_then(|p| p.as_str()).unwrap_or("i VI III VII");
                    let oct = num(op, "octave").unwrap_or(3.0) as i32;
                    let roll = num(op, "roll").unwrap_or(0.0);
                    crate::compose::progression(prog, key, &scale_name, bars, oct).ok().map(|chords| {
                        chords.iter().enumerate().flat_map(|(bar, c)| crate::compose::chord(bar as f64 * 16.0, c, 16.0, 0.75, roll)).collect()
                    })
                }
                "arp" => {
                    let prog = op.get("prog").and_then(|p| p.as_str()).unwrap_or("i VI III VII");
                    let oct = num(op, "octave").unwrap_or(4.0) as i32;
                    let rate = num(op, "rate").unwrap_or(2.0);
                    let pattern = op.get("pattern").and_then(|p| p.as_str()).unwrap_or("updown");
                    crate::compose::progression(prog, key, &scale_name, bars, oct).ok().map(|chords| {
                        chords
                            .iter()
                            .enumerate()
                            .flat_map(|(bar, c)| crate::compose::arpeggio(bar as f64 * 16.0, c, (16.0 / rate) as usize, rate, pattern, 0.7))
                            .collect()
                    })
                }
                "line" => {
                    let text = op.get("line").and_then(|l| l.as_str()).unwrap_or("");
                    let (notes, _) = crate::compose::line(0.0, text, 0.8);
                    Some(notes)
                }
                "groove" => {
                    let name = op.get("groove").and_then(|g| g.as_str()).unwrap_or("boombap");
                    crate::compose::groove(name, bars)
                }
                _ => None,
            };
            let notes = notes.map(|mut notes| {
                let length = (bars * 16) as f64;
                for step in op.get("post").and_then(|p| p.as_array()).cloned().unwrap_or_default() {
                    match step.get("fx").and_then(|f| f.as_str()) {
                        Some("humanize") => {
                            let time = num(&step, "time").unwrap_or(0.06);
                            let vel = num(&step, "vel").unwrap_or(0.05);
                            let seed = num(&step, "seed").unwrap_or(1.0) as u32;
                            crate::compose::humanize(&mut notes, length, time, vel, seed);
                        }
                        Some("dynamics") => {
                            let pts: Vec<(f64, f64)> = step
                                .get("points")
                                .and_then(|p| p.as_array())
                                .map(|arr| arr.iter().filter_map(|pt| Some((pt.get(0)?.as_f64()?, pt.get(1)?.as_f64()?))).collect())
                                .unwrap_or_default();
                            crate::compose::dynamics(&mut notes, &pts);
                        }
                        Some("pedal") => {
                            let from = num(&step, "from").unwrap_or(0.0);
                            let to = num(&step, "to").unwrap_or(length);
                            crate::compose::pedal(&mut notes, from, to, 127.0, 0.1);
                        }
                        Some("pedal_every") => {
                            let every = num(&step, "every").unwrap_or(8.0).max(1.0);
                            crate::compose::pedal_every(&mut notes, length, every);
                        }
                        _ => {}
                    }
                }
                notes
            });
            match notes {
                Some(notes) => {
                    let t = &mut project.tracks[ti];
                    let clip = t.clips.entry(sid.clone()).or_insert_with(|| Clip {
                        id: format!("{}-{}", t.id, sid),
                        name: String::new(),
                        length: bars * 16,
                        notes: vec![],
                        automation: None,
                        cc: None,
                        extra: Extra::new(),
                    });
                    clip.notes = notes;
                    clip.length = (bars * 16).clamp(4, 4096);
                    true
                }
                None => false,
            }
        }
        "copy_clip" | "move_clip" => {
            let (Some(ti), Some(from), Some(to)) = (
                op.get("track").and_then(|t| find_track(project, t)),
                op.get("from").and_then(|s| find_scene(project, s)),
                op.get("to").and_then(|s| find_scene(project, s)),
            ) else {
                return false;
            };
            let t = &mut project.tracks[ti];
            let Some(mut clip) = t.clips.get(&from).cloned() else { return false };
            clip.id = format!("{}-{}", t.id, to);
            if name == "move_clip" {
                t.clips.remove(&from);
            }
            t.clips.insert(to, clip);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_finds_the_json_object() {
        let r = parse_reply("sure:\n{\"reply\":\"ok\",\"ops\":[{\"op\":\"set_bpm\",\"bpm\":100}]}").unwrap();
        assert_eq!(r.text, "ok");
        assert_eq!(r.ops.len(), 1);
    }
}
