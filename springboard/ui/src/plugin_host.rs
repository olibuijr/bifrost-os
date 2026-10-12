//! Plugin host model. A hosted plugin exposes a parameter tree (groups and parameters, each with an address, a
//! range, a unit and flags), presets (factory presets plus user presets saved from a snapshot of the values), and
//! buses that accept only some channel counts. `Instance` ties these to a render unit from `audio_render`. The
//! model loads no plugin binary and starts no separate extension process: a plugin is a render unit that a
//! component factory made. Parameter values are floats in each parameter's own unit. No device is touched.

use std::collections::BTreeMap;
use std::fmt;

use crate::audio_render::{RenderError, RenderUnit, Scope, StreamFormat, MAX_CHANNELS};

/// Failures of the host model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The render model refused the call. The inner error says why.
    Render(RenderError),
    /// No parameter has that address.
    UnknownParameter,
    /// The parameter is read-only.
    ReadOnly,
    /// A value, range or default is not finite, or a range is empty or does not hold the default.
    InvalidValue,
    /// Two parameters share an address.
    DuplicateAddress,
    /// A name is empty.
    EmptyName,
    /// Two presets share a number.
    DuplicatePreset,
    /// No preset has that number.
    UnknownPreset,
    /// A factory preset cannot be deleted.
    FactoryPreset,
    /// The bus is disabled.
    BusDisabled,
    /// The bus does not accept this channel count.
    UnsupportedChannels,
    /// A bus has no channel counts, or one is outside 1 to 32.
    InvalidChannelList,
    /// The instance has no output bus.
    NoBuses,
}

impl From<RenderError> for HostError {
    fn from(error: RenderError) -> Self {
        HostError::Render(error)
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Render(error) => write!(f, "{}", error),
            HostError::UnknownParameter => f.write_str("no parameter has that address"),
            HostError::ReadOnly => f.write_str("the parameter is read-only"),
            HostError::InvalidValue => f.write_str("the value or range is not valid"),
            HostError::DuplicateAddress => f.write_str("two parameters share an address"),
            HostError::EmptyName => f.write_str("the name is empty"),
            HostError::DuplicatePreset => f.write_str("two presets share a number"),
            HostError::UnknownPreset => f.write_str("no preset has that number"),
            HostError::FactoryPreset => f.write_str("a factory preset cannot be deleted"),
            HostError::BusDisabled => f.write_str("the bus is disabled"),
            HostError::UnsupportedChannels => f.write_str("the bus does not accept that channel count"),
            HostError::InvalidChannelList => f.write_str("a bus needs channel counts from 1 to 32"),
            HostError::NoBuses => f.write_str("the instance has no output bus"),
        }
    }
}

/// The unit a parameter value is shown in. `Indexed` values pick one of the parameter's value strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Generic,
    Indexed,
    Boolean,
    Percent,
    Seconds,
    Hertz,
    Decibels,
    Pan,
}

/// One parameter: its address, identifier, name, range, default, unit, and whether a host may write it.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterSpec {
    pub address: u64,
    pub identifier: String,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
    pub writable: bool,
    pub value_strings: Vec<String>,
}

impl ParameterSpec {
    /// A writable parameter. The range must be finite and not empty, and the default must lie inside it.
    pub fn new(
        address: u64,
        identifier: &str,
        name: &str,
        min: f32,
        max: f32,
        default: f32,
        unit: Unit,
    ) -> Result<Self, HostError> {
        if !(min.is_finite() && max.is_finite() && default.is_finite()) || min >= max {
            return Err(HostError::InvalidValue);
        }
        if default < min || default > max {
            return Err(HostError::InvalidValue);
        }
        if name.trim().is_empty() {
            return Err(HostError::EmptyName);
        }
        Ok(ParameterSpec {
            address,
            identifier: identifier.to_string(),
            name: name.to_string(),
            min,
            max,
            default,
            unit,
            writable: true,
            value_strings: Vec::new(),
        })
    }

    /// The same parameter, marked read-only.
    pub fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    /// The same parameter, with names for its indexed values.
    pub fn with_value_strings(mut self, strings: &[&str]) -> Self {
        self.value_strings = strings.iter().map(|s| s.to_string()).collect();
        self
    }

    /// The position of a value across the range, from 0 at the minimum to 1 at the maximum.
    pub fn normalized(&self, value: f32) -> f32 {
        ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    /// The value at a position across the range. The position is clamped to 0 to 1.
    pub fn value_from_normalized(&self, position: f32) -> f32 {
        self.min + position.clamp(0.0, 1.0) * (self.max - self.min)
    }
}

/// A node of the parameter tree: a parameter, or a named group of further nodes.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Parameter(ParameterSpec),
    Group { identifier: String, name: String, children: Vec<Node> },
}

/// Collects every parameter below `nodes`, in tree order.
pub fn collect_parameters<'a>(nodes: &'a [Node], out: &mut Vec<&'a ParameterSpec>) {
    for node in nodes {
        match node {
            Node::Parameter(spec) => out.push(spec),
            Node::Group { children, .. } => collect_parameters(children, out),
        }
    }
}

/// The parameters of a plugin, by address. Values start at each default. Writes are clamped to the range, and a
/// change that moves a value is recorded until `take_changes` reads it.
#[derive(Debug)]
pub struct ParameterTree {
    roots: Vec<Node>,
    specs: BTreeMap<u64, ParameterSpec>,
    values: BTreeMap<u64, f32>,
    order: Vec<u64>,
    changes: Vec<(u64, f32)>,
}

impl ParameterTree {
    /// A tree from its top-level nodes. Two parameters may not share an address.
    pub fn new(roots: Vec<Node>) -> Result<Self, HostError> {
        let mut found: Vec<&ParameterSpec> = Vec::new();
        collect_parameters(&roots, &mut found);
        let mut specs = BTreeMap::new();
        let mut values = BTreeMap::new();
        let mut order = Vec::new();
        for spec in found {
            if specs.contains_key(&spec.address) {
                return Err(HostError::DuplicateAddress);
            }
            values.insert(spec.address, spec.default);
            order.push(spec.address);
            specs.insert(spec.address, spec.clone());
        }
        Ok(ParameterTree { roots, specs, values, order, changes: Vec::new() })
    }

    /// The top-level nodes, in the order the plugin gave them.
    pub fn roots(&self) -> &[Node] {
        &self.roots
    }

    /// Every address, in tree order.
    pub fn addresses(&self) -> &[u64] {
        &self.order
    }

    pub fn parameter(&self, address: u64) -> Option<&ParameterSpec> {
        self.specs.get(&address)
    }

    pub fn value(&self, address: u64) -> Option<f32> {
        self.values.get(&address).copied()
    }

    /// Writes a value. A finite value outside the range is clamped; the clamped value is returned. A value that
    /// does not change the stored one records no change.
    pub fn set_value(&mut self, address: u64, value: f32) -> Result<f32, HostError> {
        if !value.is_finite() {
            return Err(HostError::InvalidValue);
        }
        let (min, max, writable) = {
            let spec = self.specs.get(&address).ok_or(HostError::UnknownParameter)?;
            (spec.min, spec.max, spec.writable)
        };
        if !writable {
            return Err(HostError::ReadOnly);
        }
        let applied = value.clamp(min, max);
        let moved = self.values.get(&address).map_or(true, |old| *old != applied);
        if moved {
            self.values.insert(address, applied);
            self.changes.push((address, applied));
        }
        Ok(applied)
    }

    /// The changes made since the last call, oldest first. Each entry is an address and its new value.
    pub fn take_changes(&mut self) -> Vec<(u64, f32)> {
        std::mem::take(&mut self.changes)
    }

    /// Every value, by address. Presets store this shape.
    pub fn snapshot(&self) -> Vec<(u64, f32)> {
        self.values.iter().map(|(address, value)| (*address, *value)).collect()
    }

    /// Writes a list of values. The whole list is checked first, so a bad entry changes nothing.
    pub fn restore(&mut self, state: &[(u64, f32)]) -> Result<(), HostError> {
        for (address, value) in state {
            let spec = self.specs.get(address).ok_or(HostError::UnknownParameter)?;
            if !spec.writable {
                return Err(HostError::ReadOnly);
            }
            if !value.is_finite() {
                return Err(HostError::InvalidValue);
            }
        }
        for (address, value) in state {
            self.set_value(*address, *value)?;
        }
        Ok(())
    }
}

/// A named set of parameter values. Factory presets come with the plugin; user presets are saved by the host.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub number: i32,
    pub name: String,
    pub state: Vec<(u64, f32)>,
    pub factory: bool,
}

/// The presets of one plugin, and which one is selected.
#[derive(Debug)]
pub struct PresetStore {
    presets: Vec<Preset>,
    current: Option<i32>,
}

impl PresetStore {
    /// A store with the plugin's factory presets. Numbers must be unique and names must not be empty.
    pub fn new(factory: Vec<Preset>) -> Result<Self, HostError> {
        let mut presets: Vec<Preset> = Vec::new();
        for mut preset in factory {
            if preset.name.trim().is_empty() {
                return Err(HostError::EmptyName);
            }
            if presets.iter().any(|p| p.number == preset.number) {
                return Err(HostError::DuplicatePreset);
            }
            preset.factory = true;
            presets.push(preset);
        }
        Ok(PresetStore { presets, current: None })
    }

    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn preset(&self, number: i32) -> Option<&Preset> {
        self.presets.iter().find(|p| p.number == number)
    }

    /// The number of the preset last selected, if it is still set.
    pub fn current(&self) -> Option<i32> {
        self.current
    }

    /// Saves a user preset from a snapshot of values. Its number is one more than the highest number in the store,
    /// or 0 when the store is empty.
    pub fn save_user(&mut self, name: &str, state: Vec<(u64, f32)>) -> Result<i32, HostError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HostError::EmptyName);
        }
        let number = self.presets.iter().map(|p| p.number).max().map_or(0, |n| n.saturating_add(1));
        self.presets.push(Preset { number, name: name.to_string(), state, factory: false });
        Ok(number)
    }

    /// Deletes a user preset. Deleting the selected one clears the selection.
    pub fn delete_user(&mut self, number: i32) -> Result<(), HostError> {
        let index = self.presets.iter().position(|p| p.number == number).ok_or(HostError::UnknownPreset)?;
        if self.presets[index].factory {
            return Err(HostError::FactoryPreset);
        }
        self.presets.remove(index);
        if self.current == Some(number) {
            self.current = None;
        }
        Ok(())
    }

    /// Applies a preset to the tree. If the preset cannot be applied, the tree and the selection stay as they were.
    pub fn select(&mut self, number: i32, tree: &mut ParameterTree) -> Result<(), HostError> {
        let state = self.preset(number).ok_or(HostError::UnknownPreset)?.state.clone();
        tree.restore(&state)?;
        self.current = Some(number);
        Ok(())
    }
}

/// One input or output bus: its name, whether it is enabled, and the channel counts it accepts.
#[derive(Clone, Debug, PartialEq)]
pub struct Bus {
    pub name: String,
    pub enabled: bool,
    /// Sorted, without repeats, each from 1 to 32.
    pub channels: Vec<u32>,
}

impl Bus {
    /// An enabled bus. The channel counts are sorted and de-duplicated; the list must not be empty.
    pub fn new(name: &str, channels: &[u32]) -> Result<Self, HostError> {
        if name.trim().is_empty() {
            return Err(HostError::EmptyName);
        }
        if channels.is_empty() || channels.iter().any(|c| *c == 0 || *c > MAX_CHANNELS) {
            return Err(HostError::InvalidChannelList);
        }
        let mut list = channels.to_vec();
        list.sort_unstable();
        list.dedup();
        Ok(Bus { name: name.to_string(), enabled: true, channels: list })
    }

    /// True when the bus is enabled and takes this channel count.
    pub fn accepts(&self, channels: u32) -> bool {
        self.enabled && self.channels.binary_search(&channels).is_ok()
    }

    /// The largest channel count the bus takes.
    pub fn maximum_channels(&self) -> u32 {
        self.channels.last().copied().unwrap_or(0)
    }
}

/// The version of the plugin format a component follows. Both versions are hosted by the same model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginVersion {
    V2,
    V3,
}

/// A plugin instance: its render unit, parameters, presets and buses. The buses in `inputs` and `outputs` are
/// indexed like the render unit's buses.
pub struct Instance {
    version: PluginVersion,
    unit: RenderUnit,
    tree: ParameterTree,
    presets: PresetStore,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
}

impl Instance {
    /// An instance. It needs at least one output bus. Inputs may be empty for a generator.
    pub fn new(
        version: PluginVersion,
        unit: RenderUnit,
        tree: ParameterTree,
        presets: PresetStore,
        inputs: Vec<Bus>,
        outputs: Vec<Bus>,
    ) -> Result<Self, HostError> {
        if outputs.is_empty() {
            return Err(HostError::NoBuses);
        }
        Ok(Instance { version, unit, tree, presets, inputs, outputs })
    }

    pub fn version(&self) -> PluginVersion {
        self.version
    }

    pub fn unit(&self) -> &RenderUnit {
        &self.unit
    }

    pub fn unit_mut(&mut self) -> &mut RenderUnit {
        &mut self.unit
    }

    pub fn tree(&self) -> &ParameterTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }

    pub fn presets(&self) -> &PresetStore {
        &self.presets
    }

    pub fn presets_mut(&mut self) -> &mut PresetStore {
        &mut self.presets
    }

    /// Selects a preset and applies it to the parameters.
    pub fn select_preset(&mut self, number: i32) -> Result<(), HostError> {
        self.presets.select(number, &mut self.tree)
    }

    /// The bus at an index, or `None`. The global scope has no buses.
    pub fn bus(&self, scope: Scope, index: u32) -> Option<&Bus> {
        match scope {
            Scope::Input => self.inputs.get(index as usize),
            Scope::Output => self.outputs.get(index as usize),
            Scope::Global => None,
        }
    }

    /// Enables or disables a bus. Refused while the unit is initialized.
    pub fn set_bus_enabled(&mut self, scope: Scope, index: u32, enabled: bool) -> Result<(), HostError> {
        if self.unit.is_initialized() {
            return Err(HostError::Render(RenderError::PropertyLocked));
        }
        let list = match scope {
            Scope::Input => &mut self.inputs,
            Scope::Output => &mut self.outputs,
            Scope::Global => return Err(HostError::Render(RenderError::InvalidValue)),
        };
        let bus = list.get_mut(index as usize).ok_or(HostError::Render(RenderError::InvalidBus))?;
        bus.enabled = enabled;
        Ok(())
    }

    /// Sets the format of a bus. The bus must exist, be enabled and accept the channel count. The render unit then
    /// checks the rate and the rest of its own rules.
    pub fn set_format(&mut self, scope: Scope, index: u32, format: StreamFormat) -> Result<(), HostError> {
        let (enabled, accepts) = {
            let bus = self.bus(scope, index).ok_or(HostError::Render(RenderError::InvalidBus))?;
            (bus.enabled, bus.accepts(format.channels))
        };
        if !enabled {
            return Err(HostError::BusDisabled);
        }
        if !accepts {
            return Err(HostError::UnsupportedChannels);
        }
        self.unit.set_stream_format(scope, index, format)?;
        Ok(())
    }

    pub fn initialize(&mut self) -> Result<(), HostError> {
        self.unit.initialize()?;
        Ok(())
    }

    pub fn uninitialize(&mut self) -> Result<(), HostError> {
        self.unit.uninitialize()?;
        Ok(())
    }

    /// Produces output from the render unit.
    pub fn render(&mut self, frames: usize, sample_time: f64) -> Result<Vec<Vec<f32>>, HostError> {
        Ok(self.unit.render(frames, sample_time)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_render::{RenderBlock, RenderCallback};

    fn gain(address: u64) -> ParameterSpec {
        ParameterSpec::new(address, "gain", "Gain", 0.0, 1.0, 0.5, Unit::Generic).unwrap()
    }

    fn preset(number: i32, name: &str, state: Vec<(u64, f32)>) -> Preset {
        Preset { number, name: name.to_string(), state, factory: false }
    }

    fn stereo() -> StreamFormat {
        StreamFormat::new(48_000.0, 2).unwrap()
    }

    fn constant(value: f32) -> RenderCallback {
        Box::new(move |block: &mut RenderBlock<'_>| {
            for channel in block.channels.iter_mut() {
                for s in channel.iter_mut() {
                    *s = value;
                }
            }
            Ok(())
        })
    }

    #[test]
    fn duplicate_addresses_and_bad_ranges_are_refused() {
        let dup = ParameterTree::new(vec![Node::Parameter(gain(1)), Node::Parameter(gain(1))]);
        assert_eq!(dup.unwrap_err(), HostError::DuplicateAddress);
        assert_eq!(
            ParameterSpec::new(2, "x", "X", 1.0, 1.0, 1.0, Unit::Generic),
            Err(HostError::InvalidValue)
        );
        assert_eq!(
            ParameterSpec::new(2, "x", "X", 0.0, 1.0, 1.5, Unit::Generic),
            Err(HostError::InvalidValue)
        );
        assert_eq!(ParameterSpec::new(2, "x", " ", 0.0, 1.0, 0.5, Unit::Generic), Err(HostError::EmptyName));
    }

    #[test]
    fn writes_clamp_and_respect_read_only() {
        let mut tree = ParameterTree::new(vec![
            Node::Parameter(gain(1)),
            Node::Parameter(gain(2).read_only()),
        ])
        .unwrap();
        assert_eq!(tree.set_value(1, 2.0), Ok(1.0));
        assert_eq!(tree.set_value(1, -3.0), Ok(0.0));
        assert_eq!(tree.set_value(2, 0.25), Err(HostError::ReadOnly));
        assert_eq!(tree.set_value(9, 0.25), Err(HostError::UnknownParameter));
        assert_eq!(tree.set_value(1, f32::NAN), Err(HostError::InvalidValue));
        assert_eq!(tree.value(2), Some(0.5));
        assert_eq!(tree.addresses().to_vec(), vec![1, 2]);
    }

    #[test]
    fn only_moved_values_are_reported_as_changes() {
        let mut tree = ParameterTree::new(vec![Node::Parameter(gain(1))]).unwrap();
        tree.set_value(1, 0.5).unwrap();
        tree.set_value(1, 0.75).unwrap();
        tree.set_value(1, 0.75).unwrap();
        assert_eq!(tree.take_changes(), vec![(1, 0.75)]);
        assert!(tree.take_changes().is_empty());
    }

    #[test]
    fn selecting_a_preset_applies_its_values_and_a_bad_one_changes_nothing() {
        let mut tree = ParameterTree::new(vec![Node::Parameter(gain(1)), Node::Parameter(gain(2))]).unwrap();
        let mut presets = PresetStore::new(vec![preset(0, "Warm", vec![(1, 0.9), (2, 0.1)])]).unwrap();
        presets.select(0, &mut tree).unwrap();
        assert_eq!(tree.value(1), Some(0.9));
        assert_eq!(presets.current(), Some(0));

        let bad = presets.save_user("Bad", vec![(1, 0.2), (7, 0.2)]).unwrap();
        assert_eq!(presets.select(bad, &mut tree), Err(HostError::UnknownParameter));
        assert_eq!(tree.value(1), Some(0.9));
        assert_eq!(presets.current(), Some(0));
    }

    #[test]
    fn user_presets_number_up_and_factory_presets_stay() {
        let mut store = PresetStore::new(vec![preset(0, "A", vec![]), preset(1, "B", vec![])]).unwrap();
        assert_eq!(PresetStore::new(vec![preset(4, "X", vec![]), preset(4, "Y", vec![])]).unwrap_err(), HostError::DuplicatePreset);
        let n = store.save_user("Mine", vec![(1, 0.3)]).unwrap();
        assert_eq!(n, 2);
        assert_eq!(store.delete_user(0), Err(HostError::FactoryPreset));
        assert_eq!(store.delete_user(9), Err(HostError::UnknownPreset));
        store.delete_user(n).unwrap();
        assert!(store.preset(n).is_none());
    }

    #[test]
    fn snapshot_round_trips_through_restore() {
        let mut tree = ParameterTree::new(vec![Node::Parameter(gain(1)), Node::Parameter(gain(2))]).unwrap();
        tree.set_value(2, 0.8).unwrap();
        let saved = tree.snapshot();
        assert_eq!(saved, vec![(1, 0.5), (2, 0.8)]);
        tree.set_value(2, 0.0).unwrap();
        tree.restore(&saved).unwrap();
        assert_eq!(tree.value(2), Some(0.8));
    }

    #[test]
    fn bus_channel_lists_are_sorted_and_checked() {
        let stereo_bus = Bus::new("Main", &[2, 1, 2]).unwrap();
        assert_eq!(stereo_bus.channels, vec![1, 2]);
        assert!(stereo_bus.accepts(2));
        assert!(!stereo_bus.accepts(6));
        assert_eq!(stereo_bus.maximum_channels(), 2);
        assert_eq!(Bus::new("Empty", &[]), Err(HostError::InvalidChannelList));
        assert_eq!(Bus::new("Wide", &[33]), Err(HostError::InvalidChannelList));
    }

    #[test]
    fn instance_negotiates_buses_and_renders_through_the_unit() {
        let mut unit = RenderUnit::new(1).unwrap();
        unit.set_stream_format(Scope::Output, 0, stereo()).unwrap();
        let tree = ParameterTree::new(vec![]).unwrap();
        let presets = PresetStore::new(vec![]).unwrap();
        let mut inst = Instance::new(
            PluginVersion::V3,
            unit,
            tree,
            presets,
            vec![Bus::new("In", &[2]).unwrap()],
            vec![Bus::new("Out", &[2]).unwrap()],
        )
        .unwrap();
        assert_eq!(inst.version(), PluginVersion::V3);
        assert_eq!(inst.set_format(Scope::Input, 0, StreamFormat::new(48_000.0, 1).unwrap()), Err(HostError::UnsupportedChannels));
        inst.set_format(Scope::Input, 0, stereo()).unwrap();
        inst.unit_mut().set_render_callback(0, Some(constant(0.25))).unwrap();
        inst.set_bus_enabled(Scope::Input, 0, false).unwrap();
        assert_eq!(inst.set_format(Scope::Input, 0, stereo()), Err(HostError::BusDisabled));
        inst.set_bus_enabled(Scope::Input, 0, true).unwrap();

        inst.initialize().unwrap();
        assert_eq!(inst.set_bus_enabled(Scope::Input, 0, false), Err(HostError::Render(RenderError::PropertyLocked)));
        let out = inst.render(8, 0.0).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].iter().all(|s| *s == 0.25));
    }
}
