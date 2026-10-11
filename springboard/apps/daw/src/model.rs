//! Typed project model. It reads and writes the mobile DAW's project JSON (`data/projects/<id>.json`).
//!
//! Fields this port does not interpret yet stay in `extra` maps and are written back unchanged, so a
//! song imported today keeps everything the web app stored.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub type Extra = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub bpm: f64,
    #[serde(default)]
    pub swing: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master: Option<Master>,
    #[serde(default)]
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub arrangement: Vec<Section>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Value>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Master {
    pub volume: f64,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One arrangement section: a scene played for a number of bars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    #[serde(rename = "sceneId")]
    pub scene_id: String,
    pub bars: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub instrument: Instrument,
    #[serde(default)]
    pub effects: Vec<Effect>,
    pub volume: f64,
    #[serde(default)]
    pub pan: f64,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<Value>,
    /// Clips keyed by scene id.
    #[serde(default)]
    pub clips: BTreeMap<String, Clip>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub params: Extra,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pads: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "yes")]
    pub on: bool,
    #[serde(default)]
    pub params: Extra,
    #[serde(flatten)]
    pub extra: Extra,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Length in 16th steps (16 = one 4/4 bar), 4..=4096.
    pub length: u32,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<BTreeMap<String, Vec<Point>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `t` and `d` are in 16th steps; `p` is the MIDI pitch; `v` is velocity 0..1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub t: f64,
    pub d: f64,
    pub p: f64,
    pub v: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prob: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub t: f64,
    pub v: f64,
}

impl Project {
    /// Parses a project from the mobile DAW JSON text.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Serialises the project as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serialises")
    }

    /// Structural checks that the web app also relies on. Returns every problem found.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = vec![];
        if !(40.0..=220.0).contains(&self.bpm) {
            problems.push(format!("bpm {} outside 40..220", self.bpm));
        }
        let scene_ids: Vec<&str> = self.scenes.iter().map(|s| s.id.as_str()).collect();
        for section in &self.arrangement {
            if !scene_ids.contains(&section.scene_id.as_str()) {
                problems.push(format!("arrangement references unknown scene {}", section.scene_id));
            }
        }
        let mut track_ids = std::collections::BTreeSet::new();
        for track in &self.tracks {
            if !track_ids.insert(track.id.as_str()) {
                problems.push(format!("duplicate track id {}", track.id));
            }
            for scene in track.clips.keys() {
                if !scene_ids.contains(&scene.as_str()) {
                    problems.push(format!("track {} has a clip for unknown scene {}", track.name, scene));
                }
            }
            for (scene, clip) in &track.clips {
                if !(4..=4096).contains(&clip.length) {
                    problems.push(format!("track {} scene {} clip length {} outside 4..4096", track.name, scene, clip.length));
                }
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = r##"{
        "id":"abc","name":"Demo","bpm":120,"swing":0,"key":6,"scale":"minor",
        "scenes":[{"id":"s1","name":"Intro"}],
        "tracks":[{"id":"t1","name":"Keys","color":"#fff","instrument":{"type":"rhodes","params":{"gain":1.2}},
            "effects":[{"id":"e1","type":"lofi","on":true,"params":{"mix":1}}],
            "volume":0.8,"pan":0,"mute":false,"solo":false,
            "clips":{"s1":{"id":"c1","name":"","length":16,
                "notes":[{"t":0,"d":2.5,"p":60,"v":0.5}],
                "automation":{"e1.mix":[{"t":0,"v":1},{"t":16,"v":0.5}]}}}}],
        "arrangement":[{"sceneId":"s1","bars":8}],
        "future_field":{"kept":true}
    }"##;

    #[test]
    fn parses_and_keeps_unknown_fields() {
        let p = Project::from_json(SONG).unwrap();
        assert_eq!(p.name, "Demo");
        assert_eq!(p.tracks[0].clips["s1"].notes[0].p, 60.0);
        assert!(p.extra.contains_key("future_field"));
        assert!(p.validate().is_empty());
    }

    #[test]
    fn round_trip_is_lossless() {
        let p = Project::from_json(SONG).unwrap();
        let again = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(p, again);
        assert!(again.to_json().contains("future_field"));
    }

    #[test]
    fn validate_reports_dangling_references() {
        let mut p = Project::from_json(SONG).unwrap();
        p.arrangement[0].scene_id = "missing".into();
        p.bpm = 300.0;
        let problems = p.validate();
        assert_eq!(problems.len(), 2, "{problems:?}");
    }
}
