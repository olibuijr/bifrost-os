//! Settings model for installed audio processors: which registered manifests are switched on, and the order the enabled
//! ones run in. The Settings pane lists `rows` and sends back `set_enabled` and `move_unit`. The model holds no render
//! resources; the host allocates the enabled processors from `chain` in the order it gives. A processor starts disabled,
//! so nothing runs until the user turns it on.

use crate::audio_extension::{ExtensionError, ExtensionRegistry, Session};
use crate::audio_render::ComponentFactory;

/// Failures of the settings model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// No processor is registered at that index.
    UnknownUnit,
}

/// One line of the Settings list: a processor, its place in the run order, and whether it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitRow {
    pub index: usize,
    pub name: String,
    pub version: u32,
    pub position: usize,
    pub enabled: bool,
}

/// Enabled flags and run order for the processors of one registry, indexed like the registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioUnitSettings {
    enabled: Vec<bool>,
    order: Vec<usize>,
}

impl AudioUnitSettings {
    /// Settings for `count` processors, all disabled, in registration order.
    pub fn new(count: usize) -> Self {
        Self { enabled: vec![false; count], order: (0..count).collect() }
    }

    /// The number of processors the settings cover.
    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }

    /// Covers processors registered after `new`: the new ones are added at the end, disabled. Never shrinks.
    pub fn grow_to(&mut self, count: usize) {
        for index in self.enabled.len()..count {
            self.enabled.push(false);
            self.order.push(index);
        }
    }

    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Switches a processor on or off. Its place in the run order does not change.
    pub fn set_enabled(&mut self, index: usize, on: bool) -> Result<(), SettingsError> {
        let flag = self.enabled.get_mut(index).ok_or(SettingsError::UnknownUnit)?;
        *flag = on;
        Ok(())
    }

    /// The place of a processor in the run order, from 0.
    pub fn position_of(&self, index: usize) -> Option<usize> {
        self.order.iter().position(|&i| i == index)
    }

    /// Moves a processor to `position` in the run order. A position past the end puts it last.
    pub fn move_unit(&mut self, index: usize, position: usize) -> Result<(), SettingsError> {
        let from = self.position_of(index).ok_or(SettingsError::UnknownUnit)?;
        let unit = self.order.remove(from);
        let to = position.min(self.order.len());
        self.order.insert(to, unit);
        Ok(())
    }

    /// The enabled processors in run order. This is what the host allocates.
    pub fn chain(&self) -> Vec<usize> {
        self.order.iter().copied().filter(|&i| self.is_enabled(i)).collect()
    }

    /// The Settings list: every processor the registry knows, in run order, with its name and enabled state.
    /// Processors the settings do not cover yet are left out until `grow_to` adds them.
    pub fn rows(&self, registry: &ExtensionRegistry) -> Vec<UnitRow> {
        self.order
            .iter()
            .enumerate()
            .filter_map(|(position, &index)| {
                let manifest = registry.manifest(index)?;
                Some(UnitRow {
                    index,
                    name: manifest.name.clone(),
                    version: manifest.version,
                    position,
                    enabled: self.is_enabled(index),
                })
            })
            .collect()
    }
}

/// Allocates the enabled processors in run order. Each one is built by `factory(index)`, the factory for its registry
/// index, at the host's sample rate and frame limit. If one fails, the ones already allocated are released and the
/// error is returned, so the host never keeps a half-built chain.
pub fn allocate_chain(
    settings: &AudioUnitSettings,
    registry: &ExtensionRegistry,
    factory: impl Fn(usize) -> ComponentFactory,
    sample_rate: f64,
    max_frames: usize,
) -> Result<Vec<Session>, ExtensionError> {
    let mut sessions: Vec<Session> = Vec::new();
    for index in settings.chain() {
        match registry.allocate(index, factory(index), sample_rate, max_frames) {
            Ok(session) => sessions.push(session),
            Err(error) => {
                for mut session in sessions {
                    let _ = session.deallocate();
                }
                return Err(error);
            }
        }
    }
    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_extension::Manifest;
    use crate::audio_render::RenderUnit;

    fn mono_unit() -> RenderUnit {
        RenderUnit::new(1).unwrap()
    }

    #[test]
    fn allocate_chain_builds_only_the_enabled_processors_in_run_order() {
        let registry = registry(&["Alpha", "Beta", "Gamma"]);
        let mut settings = AudioUnitSettings::new(3);
        settings.set_enabled(0, true).unwrap();
        settings.set_enabled(2, true).unwrap();
        settings.move_unit(2, 0).unwrap();
        let sessions = allocate_chain(&settings, &registry, |_| mono_unit, 48_000.0, 512).unwrap();
        let names: Vec<&str> = sessions.iter().map(|s| s.name()).collect();
        assert_eq!(names, vec!["Gamma", "Alpha"]);
        assert!(sessions.iter().all(|s| s.is_allocated()));
    }

    #[test]
    fn allocate_chain_with_nothing_enabled_is_empty() {
        let registry = registry(&["Alpha"]);
        let settings = AudioUnitSettings::new(1);
        let sessions = allocate_chain(&settings, &registry, |_| mono_unit, 48_000.0, 512).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn allocate_chain_refuses_a_processor_the_registry_does_not_have() {
        let registry = registry(&["Alpha"]);
        let mut settings = AudioUnitSettings::new(2);
        settings.set_enabled(1, true).unwrap();
        let result = allocate_chain(&settings, &registry, |_| mono_unit, 48_000.0, 512);
        assert!(matches!(result, Err(ExtensionError::UnknownExtension)));
    }

    fn registry(names: &[&str]) -> ExtensionRegistry {
        let mut registry = ExtensionRegistry::new();
        for (n, name) in names.iter().enumerate() {
            let sub = format!("u{:03}", n);
            let manifest = Manifest::new(name, n as u32 + 1, "aufx", &sub, "Test", &[2], &[2]).unwrap();
            registry.register(manifest).unwrap();
        }
        registry
    }

    #[test]
    fn every_processor_starts_disabled_in_registration_order() {
        let settings = AudioUnitSettings::new(3);
        assert_eq!(settings.len(), 3);
        assert_eq!(settings.chain(), Vec::<usize>::new());
        assert_eq!(settings.position_of(2), Some(2));
        assert!(!settings.is_enabled(0));
    }

    #[test]
    fn the_chain_holds_only_enabled_processors_in_run_order() {
        let mut settings = AudioUnitSettings::new(3);
        settings.set_enabled(0, true).unwrap();
        settings.set_enabled(2, true).unwrap();
        assert_eq!(settings.chain(), vec![0, 2]);
        settings.move_unit(2, 0).unwrap();
        assert_eq!(settings.chain(), vec![2, 0]);
        // Turning a processor off keeps its place for when it is turned on again.
        settings.set_enabled(2, false).unwrap();
        assert_eq!(settings.chain(), vec![0]);
        assert_eq!(settings.position_of(2), Some(0));
    }

    #[test]
    fn moving_past_the_end_puts_the_processor_last() {
        let mut settings = AudioUnitSettings::new(3);
        settings.move_unit(0, 99).unwrap();
        assert_eq!(settings.position_of(0), Some(2));
        assert_eq!(settings.position_of(1), Some(0));
        assert_eq!(settings.move_unit(7, 0), Err(SettingsError::UnknownUnit));
    }

    #[test]
    fn unknown_processors_are_refused() {
        let mut settings = AudioUnitSettings::new(2);
        assert_eq!(settings.set_enabled(2, true), Err(SettingsError::UnknownUnit));
        assert!(!settings.is_enabled(2));
    }

    #[test]
    fn growing_adds_disabled_processors_at_the_end_and_never_shrinks() {
        let mut settings = AudioUnitSettings::new(1);
        settings.set_enabled(0, true).unwrap();
        settings.grow_to(3);
        assert_eq!(settings.len(), 3);
        assert_eq!(settings.position_of(2), Some(2));
        assert!(!settings.is_enabled(2));
        assert_eq!(settings.chain(), vec![0]);
        settings.grow_to(1);
        assert_eq!(settings.len(), 3);
    }

    #[test]
    fn rows_show_name_version_position_and_state_in_run_order() {
        let registry = registry(&["Alpha", "Beta"]);
        let mut settings = AudioUnitSettings::new(2);
        settings.set_enabled(1, true).unwrap();
        settings.move_unit(1, 0).unwrap();
        let rows = settings.rows(&registry);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            UnitRow { index: 1, name: "Beta".to_string(), version: 2, position: 0, enabled: true }
        );
        assert_eq!(rows[1].name, "Alpha");
        assert!(!rows[1].enabled);
    }

    #[test]
    fn rows_leave_out_processors_the_registry_no_longer_has() {
        let registry = registry(&["Alpha"]);
        let settings = AudioUnitSettings::new(2);
        let rows = settings.rows(&registry);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].index, 0);
    }
}
