//! Rack inspector model: the parameters of a track's instrument and effects as sliders, each value in the device's
//! own unit and its slider position on the device's curve, the instrument's user presets (kept as JSON per instrument
//! kind and shared by every song), and the stream format a track plays at. The shell draws the rows; this module
//! holds no drawing and no device.
use crate::devices::{effect_spec, instrument_spec, Curve, Params, P};
use crate::model::{Extra, Track};
use crate::params;
use bifrost_app::controls::audio_render::StreamFormat;
use bifrost_app::controls::channel_layout::{ChannelLayout, Label, Tag};
use bifrost_app::controls::param_view::{ParamSpec, Scale};
use bifrost_app::controls::plugin_host::PresetStore;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Which device a parameter belongs to: the track's instrument, or the effect at this index in the track's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Instrument,
    Effect(usize),
}

/// One parameter of a device: the device's spec and the index of this parameter in it.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub owner: Owner,
    pub spec: &'static [P],
    pub index: usize,
}

impl Item {
    /// The spec entry for this parameter.
    pub fn param(&self) -> P {
        self.spec[self.index]
    }
}

/// A slider row: the automation key, the label, and the parameter.
#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    pub label: String,
    pub item: Item,
}

/// The rows of a track's rack: the instrument's parameters, then each effect's, in chain order.
pub fn rows(track: &Track) -> Vec<Row> {
    let mut out = vec![];
    if let Some(spec) = instrument_spec(&track.instrument.kind) {
        for (index, q) in spec.iter().enumerate() {
            out.push(Row {
                key: format!("inst.{}", q.name),
                label: format!("instrument · {}", q.name),
                item: Item { owner: Owner::Instrument, spec, index },
            });
        }
    }
    for (chain, effect) in track.effects.iter().enumerate() {
        if let Some(spec) = effect_spec(&effect.kind) {
            for (index, q) in spec.iter().enumerate() {
                out.push(Row {
                    key: format!("{}.{}", effect.id, q.name),
                    label: format!("{} · {}", effect.kind, q.name),
                    item: Item { owner: Owner::Effect(chain), spec, index },
                });
            }
        }
    }
    out
}

/// The stored values of the device an owner names, or `None` when the track has no such effect.
pub fn owner_extra(track: &Track, owner: Owner) -> Option<&Extra> {
    match owner {
        Owner::Instrument => Some(&track.instrument.params),
        Owner::Effect(i) => track.effects.get(i).map(|e| &e.params),
    }
}

/// The mutable stored values of the device an owner names.
pub fn owner_extra_mut(track: &mut Track, owner: Owner) -> Option<&mut Extra> {
    match owner {
        Owner::Instrument => Some(&mut track.instrument.params),
        Owner::Effect(i) => track.effects.get_mut(i).map(|e| &mut e.params),
    }
}

/// The parameter's current value in its own unit. A value that is not stored reads as the spec default.
pub fn value(item: &Item, extra: &Extra) -> f64 {
    Params::new(item.spec, extra).get(item.param().name)
}

/// Writes a value, clamped into the parameter's range, and returns the clamped value.
pub fn set(item: &Item, extra: &mut Extra, v: f64) -> f64 {
    let q = item.param();
    let mut params = Params::new(item.spec, extra);
    params.set(item.spec, q.name, v);
    let clamped = params.get(q.name);
    extra.insert(q.name.to_string(), Value::from(clamped));
    clamped
}

/// The parameter's model for slider mapping: logarithmic when the spec says so and the range starts above zero.
fn slider_spec(item: &Item, value: f64) -> ParamSpec {
    let q = item.param();
    let logarithmic = matches!(q.curve, Curve::Log) && q.min > 0.0;
    ParamSpec {
        id: item.index as u32,
        name: q.name.to_string(),
        min: q.min as f32,
        max: q.max as f32,
        default: q.def as f32,
        value: value as f32,
        scale: if logarithmic { Scale::Logarithmic } else { Scale::Linear },
        expert: false,
    }
}

/// The slider position (0 to 1) for a value.
pub fn position(item: &Item, value: f64) -> f32 {
    slider_spec(item, value).normalized()
}

/// The value a slider position (0 to 1) stands for. The caller passes it to `set`, which clamps it.
pub fn value_at(item: &Item, position: f32) -> f64 {
    f64::from(slider_spec(item, 0.0).value_at(position))
}

/// The label of a channel role, as the rack lists a layout.
fn label_name(label: Label) -> &'static str {
    match label {
        Label::Unknown => "Unknown",
        Label::Left => "Left",
        Label::Right => "Right",
        Label::Center => "Center",
        Label::Lfe => "LFE",
        Label::LeftSurround => "Left surround",
        Label::RightSurround => "Right surround",
        Label::LeftRear => "Left rear",
        Label::RightRear => "Right rear",
    }
}

/// The standard arrangement for a channel count, when there is one.
fn tag_for(channels: u32) -> Option<(Tag, &'static str)> {
    match channels {
        1 => Some((Tag::Mono, "Mono")),
        2 => Some((Tag::Stereo, "Stereo")),
        4 => Some((Tag::Quadraphonic, "Quadraphonic")),
        6 => Some((Tag::Surround51, "5.1 surround")),
        8 => Some((Tag::Surround71, "7.1 surround")),
        _ => None,
    }
}

/// The stream format of a track, as the rack shows it: "48000 Hz · Stereo · Left, Right". A rate or channel count
/// the render model refuses reads as unavailable.
pub fn describe(sample_rate: u32, channels: u32) -> String {
    if StreamFormat::new(f64::from(sample_rate), channels).is_err() {
        return "Stream format unavailable".to_string();
    }
    match tag_for(channels) {
        Some((tag, name)) => match ChannelLayout::from_tag(tag) {
            Ok(layout) => {
                let labels: Vec<&str> = layout.labels().iter().map(|l| label_name(*l)).collect();
                format!("{sample_rate} Hz · {name} · {}", labels.join(", "))
            }
            Err(_) => "Stream format unavailable".to_string(),
        },
        None => format!("{sample_rate} Hz · {channels} discrete channels"),
    }
}

/// Where an instrument kind's user presets are written: `<store>/presets/<kind>.json`. Characters other than letters,
/// digits, `-` and `_` are dropped from the kind, so a kind never names a path.
pub fn preset_path(store_dir: &Path, kind: &str) -> PathBuf {
    let safe: String = kind.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    store_dir.join("presets").join(format!("{safe}.json"))
}

/// The user presets of an instrument kind. A missing file is an empty store; a file that cannot be read is an error.
pub fn load_presets(path: &Path) -> Result<PresetStore, String> {
    let mut store = params::new_presets().map_err(|e| e.to_string())?;
    match std::fs::read_to_string(path) {
        Ok(json) => {
            params::presets_from_json(&mut store, &json)?;
            Ok(store)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(store),
        Err(e) => Err(e.to_string()),
    }
}

/// Writes the user presets to their file, atomically.
pub fn save_presets(path: &Path, store: &PresetStore) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::store::write_atomic(path, &params::presets_to_json(store))
}

/// Adds a preset named from its number ("Preset 3") with the instrument's current values. Returns the name.
pub fn add_preset(store: &mut PresetStore, kind: &str, extra: &Extra) -> Result<String, String> {
    let spec = instrument_spec(kind).ok_or("This track has no instrument with parameters")?;
    let tree = params::tree_from_extra(spec, extra).map_err(|e| e.to_string())?;
    let name = format!("Preset {}", store.presets().len() + 1);
    params::save_preset(store, &tree, &name).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Sets the instrument's values to a preset. Returns the preset's name.
pub fn load_preset(store: &mut PresetStore, kind: &str, extra: &mut Extra, number: i32) -> Result<String, String> {
    let spec = instrument_spec(kind).ok_or("This track has no instrument with parameters")?;
    let mut tree = params::tree_from_extra(spec, extra).map_err(|e| e.to_string())?;
    params::apply_preset(store, &mut tree, number).map_err(|e| e.to_string())?;
    params::write_back(spec, &tree, extra);
    Ok(store.preset(number).map(|p| p.name.clone()).unwrap_or_default())
}
