//! The audio engine model for apps: a registry of named audio assets, the update mode that decides who drives rendering,
//! and the start and stop lifecycle. Pure state, no device: the sound server owns the speaker and the mixer, and this
//! model is the contract an app's engine object follows. An asset is named once and played by its name, so an app never
//! handles file paths during playback.

use crate::media::loop_region::LoopMode;
use std::collections::BTreeMap;
use std::fmt;

/// Who drives the engine's render pass. Automatic renders on the server's clock. Manual waits for the app to ask for a
/// pass, which suits an app that renders offline or steps a sequence itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateMode {
    Automatic,
    Manual,
}

/// Why an engine or registry request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// The asset name is empty.
    EmptyName,
    /// The asset path is empty.
    EmptyPath,
    /// An asset with this name is already registered.
    DuplicateAsset(String),
    /// No asset with this name is registered.
    UnknownAsset(String),
    /// The engine is already running.
    AlreadyRunning,
    /// The engine is not running.
    NotRunning,
    /// The update mode cannot change while the engine runs.
    ModeLockedWhileRunning,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::EmptyName => f.write_str("the asset name is empty"),
            EngineError::EmptyPath => f.write_str("the asset path is empty"),
            EngineError::DuplicateAsset(name) => write!(f, "an asset named {name} is already registered"),
            EngineError::UnknownAsset(name) => write!(f, "no asset named {name} is registered"),
            EngineError::AlreadyRunning => f.write_str("the engine is already running"),
            EngineError::NotRunning => f.write_str("the engine is not running"),
            EngineError::ModeLockedWhileRunning => f.write_str("the update mode cannot change while the engine runs"),
        }
    }
}

/// One registered sound: where it lives and how it loops when played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub path: String,
    pub loop_mode: LoopMode,
}

/// Named audio assets. Names are unique and listed in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetRegistry {
    assets: BTreeMap<String, Asset>,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `name` for the file at `path`. Names and paths must not be empty, and a name may be registered once.
    pub fn register(&mut self, name: &str, path: &str, loop_mode: LoopMode) -> Result<(), EngineError> {
        if name.trim().is_empty() {
            return Err(EngineError::EmptyName);
        }
        if path.trim().is_empty() {
            return Err(EngineError::EmptyPath);
        }
        if self.assets.contains_key(name) {
            return Err(EngineError::DuplicateAsset(name.to_string()));
        }
        self.assets.insert(name.to_string(), Asset { path: path.to_string(), loop_mode });
        Ok(())
    }

    /// Removes an asset. Refused when the name is unknown.
    pub fn unregister(&mut self, name: &str) -> Result<Asset, EngineError> {
        self.assets.remove(name).ok_or_else(|| EngineError::UnknownAsset(name.to_string()))
    }

    pub fn get(&self, name: &str) -> Option<&Asset> {
        self.assets.get(name)
    }

    /// The registered names, in alphabetical order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.assets.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

/// The engine an app owns: its assets, its update mode and whether it runs. The mode is set before the engine starts
/// and is locked while it runs, so a render loop never changes driver under itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Engine {
    assets: AssetRegistry,
    mode: UpdateMode,
    running: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Engine { assets: AssetRegistry::new(), mode: UpdateMode::Automatic, running: false }
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn assets(&self) -> &AssetRegistry {
        &self.assets
    }

    pub fn assets_mut(&mut self) -> &mut AssetRegistry {
        &mut self.assets
    }

    pub fn update_mode(&self) -> UpdateMode {
        self.mode
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Changes the update mode. Refused while the engine runs.
    pub fn set_update_mode(&mut self, mode: UpdateMode) -> Result<(), EngineError> {
        if self.running {
            return Err(EngineError::ModeLockedWhileRunning);
        }
        self.mode = mode;
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), EngineError> {
        if self.running {
            return Err(EngineError::AlreadyRunning);
        }
        self.running = true;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), EngineError> {
        if !self.running {
            return Err(EngineError::NotRunning);
        }
        self.running = false;
        Ok(())
    }

    /// Plays the asset `name`: returns its path and loop mode, which the sound server then renders. Refused when the
    /// engine is not running or the name is unknown.
    pub fn play(&self, name: &str) -> Result<&Asset, EngineError> {
        if !self.running {
            return Err(EngineError::NotRunning);
        }
        self.assets.get(name).ok_or_else(|| EngineError::UnknownAsset(name.to_string()))
    }
}
