//! Device parameters and presets as plug-in parameters. A device's spec list (`devices::P`) becomes a
//! parameter tree from `bifrost_ui::plugin_host`, with each parameter's address its index in the spec. The
//! user's presets are a `PresetStore` of value snapshots, saved from the tree, applied back to it, and written
//! to JSON so they survive a restart. `params_from_tree` hands the tree's values to the device engine.
use crate::devices::{Params, P};
use crate::model::Extra;
use bifrost_app::controls::plugin_host::{HostError, Node, ParameterSpec, ParameterTree, PresetStore, Unit};
use serde::{Deserialize, Serialize};

/// The display unit for a device unit string (`P::unit`).
pub fn unit_for(unit: &str) -> Unit {
    match unit {
        "Hz" => Unit::Hertz,
        "dB" => Unit::Decibels,
        "%" => Unit::Percent,
        "s" => Unit::Seconds,
        _ => Unit::Generic,
    }
}

/// The device's parameters as a tree. Addresses are spec indexes, and values start at each spec default.
/// Fails if a spec entry has an empty range or a default outside its range.
pub fn tree_for(spec: &[P]) -> Result<ParameterTree, HostError> {
    let mut nodes = Vec::with_capacity(spec.len());
    for (index, s) in spec.iter().enumerate() {
        let param = ParameterSpec::new(
            index as u64,
            s.name,
            s.name,
            s.min as f32,
            s.max as f32,
            s.def as f32,
            unit_for(s.unit),
        )?;
        nodes.push(Node::Parameter(param));
    }
    ParameterTree::new(nodes)
}

/// The device engine's parameters, with the tree's current values written over the defaults. A tree value
/// that no longer matches a spec name is ignored.
pub fn params_from_tree(spec: &[P], tree: &ParameterTree) -> Params {
    let mut params = Params::new(spec, &Extra::new());
    for (index, s) in spec.iter().enumerate() {
        if let Some(value) = tree.value(index as u64) {
            params.set(spec, s.name, f64::from(value));
        }
    }
    params
}

/// An empty preset store: the device has no factory presets, and the user saves their own.
pub fn new_presets() -> Result<PresetStore, HostError> {
    PresetStore::new(Vec::new())
}

/// Saves the tree's current values as a user preset. Returns its number.
pub fn save_preset(store: &mut PresetStore, tree: &ParameterTree, name: &str) -> Result<i32, HostError> {
    store.save_user(name, tree.snapshot())
}

/// Sets the tree to a preset and marks it as the current one. The tree is unchanged if the preset is unknown.
pub fn apply_preset(store: &mut PresetStore, tree: &mut ParameterTree, number: i32) -> Result<(), HostError> {
    store.select(number, tree)
}

#[derive(Serialize, Deserialize)]
struct SavedPreset {
    name: String,
    values: Vec<(u64, f32)>,
}

/// The user presets as JSON, in number order. Factory presets are not written.
pub fn presets_to_json(store: &PresetStore) -> String {
    let saved: Vec<SavedPreset> = store
        .presets()
        .iter()
        .filter(|p| !p.factory)
        .map(|p| SavedPreset { name: p.name.clone(), values: p.state.clone() })
        .collect();
    serde_json::to_string_pretty(&saved).unwrap_or_else(|_| "[]".to_string())
}

/// Adds the user presets from JSON written by `presets_to_json`. Returns how many were read. Presets are
/// numbered after the ones already in the store.
pub fn presets_from_json(store: &mut PresetStore, json: &str) -> Result<usize, String> {
    let saved: Vec<SavedPreset> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let count = saved.len();
    for preset in saved {
        store.save_user(&preset.name, preset.values).map_err(|e| e.to_string())?;
    }
    Ok(count)
}
