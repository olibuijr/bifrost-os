//! Settings > Sound rows built from the audio object model (device lists, defaults, input level) and the
//! menu-bar input indicator flag. Pure: `SoundView` is filled by the audio service from the daemon's properties
//! (docs/audio-architecture.md, "Policy"); nothing here opens a device.

use super::list::{Group, Item};
use super::pages::{Action, Pick};
use std::sync::atomic::{AtomicBool, Ordering};

pub const PREF_OUTPUT: &str = "audio_default_output";
pub const PREF_INPUT: &str = "audio_default_input";

/// One listed device: display name and the server's UID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceRow {
    pub name: String,
    pub uid: String,
}

/// What the page shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundView {
    pub outputs: Vec<DeviceRow>,
    pub inputs: Vec<DeviceRow>,
    /// Index into `outputs` / `inputs` of the current default.
    pub output: Option<usize>,
    pub input: Option<usize>,
    /// The input level, 0..=1 (peak of the default input).
    pub level: f32,
}

pub static NO_SOUND: SoundView = SoundView { outputs: Vec::new(), inputs: Vec::new(), output: None, input: None, level: 0.0 };

/// The groups Settings > Sound adds under the alert volume: Output, Input and the level meter.
pub fn groups(v: &SoundView) -> Vec<Group> {
    let mut g = Vec::new();
    if !v.outputs.is_empty() {
        let rows = v.outputs.iter().enumerate().map(|(i, d)| Item::new(d.name.as_str()).check(v.output == Some(i), Action::Pick(Pick::SoundOutput(i)))).collect();
        g.push(Group::new(rows).header("OUTPUT").footer("Choose where sound plays. Plugging in headphones switches to them."));
    }
    if !v.inputs.is_empty() {
        let mut rows: Vec<Item> = v.inputs.iter().enumerate().map(|(i, d)| Item::new(d.name.as_str()).check(v.input == Some(i), Action::Pick(Pick::SoundInput(i)))).collect();
        rows.push(Item::new("Input Level").value(level_label(v.level)));
        g.push(Group::new(rows).header("INPUT"));
    }
    g
}

/// The input level as a percentage.
pub fn level_label(level: f32) -> String {
    format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)
}

/// The pref (key, value) to store when the user picks output or input `k`.
pub fn pick_pref(v: &SoundView, input: bool, k: usize) -> Option<(&'static str, String)> {
    let list = if input { &v.inputs } else { &v.outputs };
    list.get(k).map(|d| (if input { PREF_INPUT } else { PREF_OUTPUT }, d.uid.clone()))
}

static INPUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The audio service sets this while any input stream or tap runs; the menu bar draws the indicator from it.
pub fn set_input_active(on: bool) {
    INPUT_ACTIVE.store(on, Ordering::Relaxed);
}

pub fn input_active() -> bool {
    INPUT_ACTIVE.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SoundView {
        let d = |n: &str, u: &str| DeviceRow { name: n.into(), uid: u.into() };
        SoundView { outputs: vec![d("Speakers", "spk"), d("Headphones", "hp")], inputs: vec![d("Mic", "mic")], output: Some(1), input: Some(0), level: 0.256 }
    }

    #[test]
    fn empty_view_adds_no_groups() {
        assert!(groups(&NO_SOUND).is_empty());
    }

    #[test]
    fn groups_list_devices_and_level() {
        let g = groups(&view());
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].items.len(), 2);
        assert_eq!(g[1].items.len(), 2);
        assert_eq!(g[1].items[1].label.as_str(), "Input Level");
        assert_eq!(level_label(0.256), "26%");
        assert_eq!(level_label(3.0), "100%");
    }

    #[test]
    fn pick_maps_to_pref() {
        let v = view();
        assert_eq!(pick_pref(&v, false, 1), Some((PREF_OUTPUT, "hp".to_string())));
        assert_eq!(pick_pref(&v, true, 0), Some((PREF_INPUT, "mic".to_string())));
        assert_eq!(pick_pref(&v, true, 5), None);
    }

    #[test]
    fn indicator_flag() {
        set_input_active(true);
        assert!(input_active());
        set_input_active(false);
        assert!(!input_active());
    }
}
