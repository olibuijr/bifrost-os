//! App extension host for network providers: the configuration each provider app declares, one enabled provider per
//! extension point, and the provider's process taken from the extension process table.
//!
//! Rules of this model:
//! - A configuration is a bundle identifier (reverse-DNS: at least three labels of ASCII letters, digits and hyphens,
//!   no label empty or starting or ending with a hyphen, at most 255 bytes), an extension point, and a provider class
//!   name (ASCII letters, digits, underscores and dots, starting with a letter, at most 128 bytes).
//! - A bundle identifier is registered once per point. The same bundle may register for other points.
//! - At most one provider is enabled per point. Enabling a second one is refused until the first is disabled.
//! - [`ProviderHost::start`] returns the process for the enabled provider from [`ProcessTable`]. The table starts no OS
//!   process itself: a launcher reports the process running, and only a running process takes a connection.
//!
//! Nothing in this crate launches a provider or hands it traffic yet.

use crate::extension_process::{ConnectionId, Error as ProcessError, Key, ProcessId, ProcessTable};
use std::collections::BTreeMap;

/// The extension points a network provider can serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Point {
    PacketTunnel,
    AppProxy,
    FilterData,
    FilterControl,
    DnsProxy,
}

impl Point {
    pub const ALL: [Point; 5] = [Point::PacketTunnel, Point::AppProxy, Point::FilterData, Point::FilterControl, Point::DnsProxy];

    /// The stable text name of the point, as it appears in the process key.
    pub fn name(self) -> &'static str {
        match self {
            Point::PacketTunnel => "packet-tunnel",
            Point::AppProxy => "app-proxy",
            Point::FilterData => "filter-data",
            Point::FilterControl => "filter-control",
            Point::DnsProxy => "dns-proxy",
        }
    }

    pub fn parse(text: &str) -> Option<Point> {
        Point::ALL.iter().copied().find(|point| point.name() == text)
    }
}

/// Why a configuration, registration, enablement or process request is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    InvalidBundleId,
    InvalidProviderClass,
    /// The bundle is already registered for this point.
    Duplicate,
    /// No configuration with this bundle and point is registered.
    NotRegistered,
    /// Another provider is enabled on the point; the bundle identifier is given.
    AnotherEnabled(String),
    /// The provider is not enabled on the point.
    NotEnabled,
    /// The extension process table refused the request.
    Process(ProcessError),
}

/// What a provider app declares for one extension point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    bundle_id: String,
    point: Point,
    provider_class: String,
}

impl Configuration {
    pub fn new(bundle_id: &str, point: Point, provider_class: &str) -> Result<Configuration, ConfigError> {
        if !valid_bundle_id(bundle_id) {
            return Err(ConfigError::InvalidBundleId);
        }
        if !valid_provider_class(provider_class) {
            return Err(ConfigError::InvalidProviderClass);
        }
        Ok(Configuration { bundle_id: bundle_id.to_string(), point, provider_class: provider_class.to_string() })
    }

    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    pub fn point(&self) -> Point {
        self.point
    }

    pub fn provider_class(&self) -> &str {
        &self.provider_class
    }
}

fn valid_bundle_id(id: &str) -> bool {
    id.len() <= 255
        && id.split('.').count() >= 3
        && id.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

fn valid_provider_class(name: &str) -> bool {
    name.len() <= 128
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// The registered providers, the one enabled provider per point, and the process table that runs them.
#[derive(Default)]
pub struct ProviderHost {
    configurations: Vec<Configuration>,
    enabled: BTreeMap<Point, String>,
    processes: ProcessTable,
}

impl ProviderHost {
    pub fn new() -> ProviderHost {
        ProviderHost::default()
    }

    pub fn register(&mut self, configuration: Configuration) -> Result<(), ConfigError> {
        let exists = self
            .configurations
            .iter()
            .any(|known| known.bundle_id == configuration.bundle_id && known.point == configuration.point);
        if exists {
            return Err(ConfigError::Duplicate);
        }
        self.configurations.push(configuration);
        Ok(())
    }

    /// Enables the registered provider on its point. Enabling it again is harmless.
    pub fn enable(&mut self, bundle_id: &str, point: Point) -> Result<(), ConfigError> {
        if !self.is_registered(bundle_id, point) {
            return Err(ConfigError::NotRegistered);
        }
        if let Some(active) = self.enabled.get(&point) {
            if active != bundle_id {
                return Err(ConfigError::AnotherEnabled(active.clone()));
            }
        }
        self.enabled.insert(point, bundle_id.to_string());
        Ok(())
    }

    /// Disables the provider on `point`, which frees the point for another provider.
    pub fn disable(&mut self, bundle_id: &str, point: Point) -> Result<(), ConfigError> {
        if self.enabled.get(&point).map(String::as_str) != Some(bundle_id) {
            return Err(ConfigError::NotEnabled);
        }
        self.enabled.remove(&point);
        Ok(())
    }

    /// The configuration of the provider enabled on `point`, if one is.
    pub fn enabled_for(&self, point: Point) -> Option<&Configuration> {
        let bundle_id = self.enabled.get(&point)?;
        self.configurations.iter().find(|known| known.point == point && &known.bundle_id == bundle_id)
    }

    /// The process of the provider enabled on `point`. A live process is reused; otherwise a new one is recorded as
    /// launching.
    pub fn start(&mut self, point: Point) -> Result<ProcessId, ConfigError> {
        let key = {
            let configuration = self.enabled_for(point).ok_or(ConfigError::NotEnabled)?;
            Key::new(configuration.bundle_id(), point.name())
        };
        Ok(self.processes.acquire(&key))
    }

    /// The launcher reports that the process is up and may take connections.
    pub fn mark_running(&mut self, process: ProcessId) -> Result<(), ConfigError> {
        self.processes.running(process).map_err(ConfigError::Process)
    }

    /// Opens a connection to a running provider process.
    pub fn connect(&mut self, process: ProcessId) -> Result<ConnectionId, ConfigError> {
        self.processes.connect(process).map_err(ConfigError::Process)
    }

    fn is_registered(&self, bundle_id: &str, point: Point) -> bool {
        self.configurations.iter().any(|known| known.point == point && known.bundle_id == bundle_id)
    }
}
