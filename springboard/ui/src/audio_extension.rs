//! Audio extension point: the manifest an app ships for an audio processor, a registry that checks manifests and
//! finds them by component description, and a session that allocates and frees the render resources of one
//! processor. This follows the shape of the AUAudioUnit subclassing contract: a component description (kind,
//! sub-kind and manufacturer), bus layouts, allocation before rendering and deallocation after it, and a maximum
//! frame count per render. The processor runs in the host's process; no separate extension process is started.
//! The factory that builds the render unit is supplied by the host, so no plug-in binary is loaded.

use crate::audio_render::{
    ComponentDescription, ComponentFactory, RenderError, RenderUnit, Scope, StreamFormat, MAX_BUSES, MAX_CHANNELS,
};

/// Failures of the extension point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionError {
    /// The render model refused the call. The inner error says why.
    Render(RenderError),
    /// The name is empty or only spaces.
    EmptyName,
    /// A component code is empty. A manifest must name its kind, sub-kind and manufacturer exactly.
    WildcardDescription,
    /// A manifest has no output bus.
    NoOutputBus,
    /// A manifest has more buses than the render model allows, or its input and output counts differ.
    BusCountMismatch,
    /// Another manifest already has this component description or this name.
    DuplicateComponent,
    /// No manifest is registered at that index.
    UnknownExtension,
    /// The session has already released its render resources.
    NotAllocated,
}

impl From<RenderError> for ExtensionError {
    fn from(error: RenderError) -> Self {
        ExtensionError::Render(error)
    }
}

/// What an app declares about its processor: identity, version, and the channel count of each bus.
/// `inputs` may be empty for a generator. Otherwise it has one entry per output bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: u32,
    pub description: ComponentDescription,
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
}

impl Manifest {
    /// A checked manifest. The three codes must each be four printable characters, the buses must be within the
    /// render model's limits, and every channel count must be from 1 to 32.
    pub fn new(
        name: &str,
        version: u32,
        kind: &str,
        sub_kind: &str,
        manufacturer: &str,
        inputs: &[u32],
        outputs: &[u32],
    ) -> Result<Self, ExtensionError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ExtensionError::EmptyName);
        }
        if kind.is_empty() || sub_kind.is_empty() || manufacturer.is_empty() {
            return Err(ExtensionError::WildcardDescription);
        }
        let description = ComponentDescription::new(kind, sub_kind, manufacturer)?;
        if outputs.is_empty() {
            return Err(ExtensionError::NoOutputBus);
        }
        if outputs.len() > MAX_BUSES as usize || (!inputs.is_empty() && inputs.len() != outputs.len()) {
            return Err(ExtensionError::BusCountMismatch);
        }
        for &channels in inputs.iter().chain(outputs.iter()) {
            if !(1..=MAX_CHANNELS).contains(&channels) {
                return Err(ExtensionError::Render(RenderError::InvalidFormat));
            }
        }
        Ok(Manifest {
            name: name.to_string(),
            version,
            description,
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
        })
    }

    /// Two manifests collide when they share a component description or a name.
    fn collides_with(&self, other: &Manifest) -> bool {
        self.description == other.description || self.name == other.name
    }
}

/// The manifests a host knows about, in registration order.
#[derive(Default)]
pub struct ExtensionRegistry {
    manifests: Vec<Manifest>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a manifest and returns its index. A manifest whose description or name is already registered is refused.
    pub fn register(&mut self, manifest: Manifest) -> Result<usize, ExtensionError> {
        if self.manifests.iter().any(|m| m.collides_with(&manifest)) {
            return Err(ExtensionError::DuplicateComponent);
        }
        self.manifests.push(manifest);
        Ok(self.manifests.len() - 1)
    }

    pub fn manifest(&self, index: usize) -> Option<&Manifest> {
        self.manifests.get(index)
    }

    /// The index of the first manifest after `after` (or from the start when `None`) that the query matches.
    /// A query field that is a wildcard matches any value, as in the component registry.
    pub fn find_next(&self, query: &ComponentDescription, after: Option<usize>) -> Option<usize> {
        let start = after.map_or(0, |i| i + 1);
        (start..self.manifests.len()).find(|&i| query.matches(&self.manifests[i].description))
    }

    /// Builds the render unit with `factory`, configures every bus from the manifest at the host's sample rate
    /// and frame limit, and initializes it. This is the allocation step before rendering.
    pub fn allocate(
        &self,
        index: usize,
        factory: ComponentFactory,
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<Session, ExtensionError> {
        let manifest = self.manifests.get(index).ok_or(ExtensionError::UnknownExtension)?;
        let mut unit = factory();
        unit.set_max_frames(max_frames)?;
        for (bus, &channels) in manifest.inputs.iter().enumerate() {
            let format = StreamFormat::new(sample_rate, channels)?;
            unit.set_stream_format(Scope::Input, bus as u32, format)?;
        }
        for (bus, &channels) in manifest.outputs.iter().enumerate() {
            let format = StreamFormat::new(sample_rate, channels)?;
            unit.set_stream_format(Scope::Output, bus as u32, format)?;
        }
        unit.initialize()?;
        Ok(Session { name: manifest.name.clone(), unit: Some(unit) })
    }
}

/// A processor with allocated render resources. Dropping it without `deallocate` leaves the unit initialized,
/// so the host should always call `deallocate` when it is done.
pub struct Session {
    name: String,
    unit: Option<RenderUnit>,
}

impl Session {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_allocated(&self) -> bool {
        self.unit.is_some()
    }

    /// Produces `frames` frames of output at `sample_time`. Fails once the resources are released.
    pub fn render(&mut self, frames: usize, sample_time: f64) -> Result<Vec<Vec<f32>>, ExtensionError> {
        let unit = self.unit.as_mut().ok_or(ExtensionError::NotAllocated)?;
        Ok(unit.render(frames, sample_time)?)
    }

    /// Releases the render resources. Calling it again returns `NotAllocated`.
    pub fn deallocate(&mut self) -> Result<(), ExtensionError> {
        let mut unit = self.unit.take().ok_or(ExtensionError::NotAllocated)?;
        unit.uninitialize()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono_unit() -> RenderUnit {
        RenderUnit::new(1).unwrap()
    }

    fn two_bus_unit() -> RenderUnit {
        RenderUnit::new(2).unwrap()
    }

    fn gain_manifest(name: &str) -> Manifest {
        Manifest::new(name, 1, "aufx", "gain", "Bfro", &[2], &[2]).unwrap()
    }

    fn registry_with_gain() -> ExtensionRegistry {
        let mut registry = ExtensionRegistry::new();
        registry.register(gain_manifest("Gain")).unwrap();
        registry
    }

    #[test]
    fn a_manifest_needs_a_name_exact_codes_and_an_output() {
        assert_eq!(Manifest::new("  ", 1, "aufx", "gain", "Bfro", &[], &[2]), Err(ExtensionError::EmptyName));
        assert_eq!(Manifest::new("G", 1, "", "gain", "Bfro", &[], &[2]), Err(ExtensionError::WildcardDescription));
        assert_eq!(Manifest::new("G", 1, "aufx", "gain", "", &[], &[2]), Err(ExtensionError::WildcardDescription));
        assert_eq!(Manifest::new("G", 1, "aufx", "gain", "Bfro", &[], &[]), Err(ExtensionError::NoOutputBus));
        assert_eq!(
            Manifest::new("G", 1, "aufx", "gain", "Bfro", &[2, 2], &[2]),
            Err(ExtensionError::BusCountMismatch)
        );
    }

    #[test]
    fn channel_counts_are_checked_against_the_render_model() {
        let zero = Manifest::new("G", 1, "aufx", "gain", "Bfro", &[], &[0]);
        assert_eq!(zero, Err(ExtensionError::Render(RenderError::InvalidFormat)));
        let too_many = Manifest::new("G", 1, "aufx", "gain", "Bfro", &[], &[MAX_CHANNELS + 1]);
        assert_eq!(too_many, Err(ExtensionError::Render(RenderError::InvalidFormat)));
    }

    #[test]
    fn a_generator_has_no_input_buses() {
        let generator = Manifest::new("Tone", 2, "aumu", "tone", "Bfro", &[], &[1]).unwrap();
        assert!(generator.inputs.is_empty());
        assert_eq!(generator.version, 2);
    }

    #[test]
    fn the_registry_refuses_a_duplicate_description_or_name() {
        let mut registry = registry_with_gain();
        assert_eq!(registry.register(gain_manifest("Other")), Err(ExtensionError::DuplicateComponent));
        assert_eq!(registry.register(gain_manifest("Gain")), Err(ExtensionError::DuplicateComponent));
        let distinct = Manifest::new("Delay", 1, "aufx", "dely", "Bfro", &[2], &[2]).unwrap();
        assert_eq!(registry.register(distinct), Ok(1));
    }

    #[test]
    fn a_query_finds_manifests_and_wildcards_match_any_code() {
        let mut registry = registry_with_gain();
        registry.register(Manifest::new("Delay", 1, "aufx", "dely", "Bfro", &[2], &[2]).unwrap()).unwrap();
        let effects = ComponentDescription::new("aufx", "", "").unwrap();
        assert_eq!(registry.find_next(&effects, None), Some(0));
        assert_eq!(registry.find_next(&effects, Some(0)), Some(1));
        assert_eq!(registry.find_next(&effects, Some(1)), None);
        let exact = ComponentDescription::new("aufx", "gain", "Bfro").unwrap();
        assert_eq!(registry.find_next(&exact, None), Some(0));
        let other = ComponentDescription::new("aumu", "", "").unwrap();
        assert_eq!(registry.find_next(&other, None), None);
        assert_eq!(registry.manifest(0).unwrap().name, "Gain");
    }

    #[test]
    fn allocation_configures_buses_and_renders_the_output_channels() {
        let registry = registry_with_gain();
        let mut session = registry.allocate(0, two_bus_unit, 48_000.0, 512).unwrap();
        assert_eq!(session.name(), "Gain");
        assert!(session.is_allocated());
        let out = session.render(256, 0.0).unwrap();
        assert_eq!(out.len(), 2, "two output channels");
        assert_eq!(out[0].len(), 256);
    }

    #[test]
    fn allocation_fails_for_an_unknown_index_or_a_bad_rate() {
        let registry = registry_with_gain();
        assert_eq!(registry.allocate(5, two_bus_unit, 48_000.0, 512).err(), Some(ExtensionError::UnknownExtension));
        assert_eq!(
            registry.allocate(0, two_bus_unit, 0.0, 512).err(),
            Some(ExtensionError::Render(RenderError::InvalidFormat))
        );
    }

    #[test]
    fn a_factory_with_too_few_buses_is_refused() {
        let mut registry = ExtensionRegistry::new();
        registry.register(Manifest::new("Mix", 1, "aufx", "mixr", "Bfro", &[2, 2], &[2, 2]).unwrap()).unwrap();
        // The manifest needs two bus pairs; a one-bus unit has no bus 1 to configure.
        assert_eq!(
            registry.allocate(0, mono_unit, 48_000.0, 512).err(),
            Some(ExtensionError::Render(RenderError::InvalidBus))
        );
    }

    #[test]
    fn rendering_more_frames_than_allocated_is_refused() {
        let registry = registry_with_gain();
        let mut session = registry.allocate(0, two_bus_unit, 48_000.0, 128).unwrap();
        assert_eq!(
            session.render(129, 0.0),
            Err(ExtensionError::Render(RenderError::TooManyFrames))
        );
    }

    #[test]
    fn deallocation_releases_once_and_then_rendering_stops() {
        let registry = registry_with_gain();
        let mut session = registry.allocate(0, two_bus_unit, 48_000.0, 512).unwrap();
        assert_eq!(session.deallocate(), Ok(()));
        assert!(!session.is_allocated());
        assert_eq!(session.deallocate(), Err(ExtensionError::NotAllocated));
        assert_eq!(session.render(64, 0.0), Err(ExtensionError::NotAllocated));
    }
}
