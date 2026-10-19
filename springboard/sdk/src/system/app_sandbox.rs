//! What an app's sandbox grants: the capabilities its manifest permissions and entitlements turn on. The shell builds
//! the sandbox from the same [`Policy`]; this module names those grants so an app can read them. The app cannot
//! widen them, and nothing here changes the sandbox.

use springboard::bare::app_policy::Policy;

/// One thing the sandbox may allow an app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Network,
    Storage,
    Audio,
    Notifications,
    Documents,
    Keychain,
    Accessibility,
    DisplayConfig,
    RemoteDesktop,
    LogStore,
    SystemMonitor,
}

impl Capability {
    /// Every capability, in the order the manifest names them.
    pub const ALL: [Capability; 11] = [
        Capability::Network,
        Capability::Storage,
        Capability::Audio,
        Capability::Notifications,
        Capability::Documents,
        Capability::Keychain,
        Capability::Accessibility,
        Capability::DisplayConfig,
        Capability::RemoteDesktop,
        Capability::LogStore,
        Capability::SystemMonitor,
    ];

    /// The manifest name of the permission or entitlement that grants this capability.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Network => "network",
            Capability::Storage => "storage",
            Capability::Audio => "audio",
            Capability::Notifications => "notifications",
            Capability::Documents => "documents",
            Capability::Keychain => "keychain",
            Capability::Accessibility => "accessibility",
            Capability::DisplayConfig => "display-config",
            Capability::RemoteDesktop => "remote-desktop",
            Capability::LogStore => "log-store",
            Capability::SystemMonitor => "system-monitor",
        }
    }

    fn granted_by(self, policy: &Policy) -> bool {
        match self {
            Capability::Network => policy.network,
            Capability::Storage => policy.storage,
            Capability::Audio => policy.audio,
            Capability::Notifications => policy.notifications,
            Capability::Documents => policy.documents,
            Capability::Keychain => policy.keychain,
            Capability::Accessibility => policy.accessibility,
            Capability::DisplayConfig => policy.display_config,
            Capability::RemoteDesktop => policy.remote,
            Capability::LogStore => policy.log_store,
            Capability::SystemMonitor => policy.system_monitor,
        }
    }
}

/// Whether the sandbox built from `policy` allows `capability`. A policy with no grants allows nothing.
pub fn permits(policy: &Policy, capability: Capability) -> bool {
    capability.granted_by(policy)
}

/// The capabilities `policy` grants, in manifest order.
pub fn granted(policy: &Policy) -> Vec<Capability> {
    Capability::ALL.into_iter().filter(|c| c.granted_by(policy)).collect()
}
