//! VPN configuration for apps, shaped like the reference `NEVPNManager` flow: set the protocol configuration, enable
//! the VPN, and save. The manager holds one personal VPN configuration with its enabled flag.
//!
//! The model changes no network state and owns no socket. [`VpnManager::networkmanager_keyfile`] renders the
//! configuration as a keyfile for the system connection manager (NetworkManager), and the system layer writes that
//! file and brings the connection up. The profile holds no secrets, so the keyfile holds none either.
//!
//! Decisions:
//! - IKEv2 and IPsec/IKEv1 map to the connection manager's VPN plugin services. A `Tunnel` profile is implemented by
//!   an app provider, so it has no system keyfile and cannot be enabled through this manager.
//! - `on_demand` is applied by the system layer when a network appears. The keyfile always carries `autoconnect=false`,
//!   so the connection manager never brings the tunnel up on its own.
//! - The port is written under `[vpn]` for the system layer to read, because the plugin keys do not carry one.

use crate::tunnel::{TunnelError, TunnelState, VpnProfile, VpnProtocol};
use std::fmt::Write as _;

/// The connection manager's VPN plugin service for a protocol the system implements, or `None` for a protocol an app
/// provider implements.
fn service_type(protocol: VpnProtocol) -> Option<&'static str> {
    match protocol {
        VpnProtocol::Ikev2 => Some("org.freedesktop.NetworkManager.strongswan"),
        VpnProtocol::IpsecIkev1 => Some("org.freedesktop.NetworkManager.libreswan"),
        VpnProtocol::Tunnel => None,
    }
}

/// The VPN status an app observes, named after the reference status values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpnStatus {
    /// No protocol configuration is saved.
    Invalid,
    Disconnected,
    Connecting,
    Connected,
    Reasserting,
    Disconnecting,
}

/// One personal VPN configuration and whether it is enabled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpnManager {
    enabled: bool,
    profile: Option<VpnProfile>,
}

impl VpnManager {
    /// A manager with no configuration, disabled.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The saved protocol configuration, if any.
    pub fn protocol_configuration(&self) -> Option<&VpnProfile> {
        self.profile.as_ref()
    }

    /// Replace the protocol configuration. The profile is checked first; a refused profile leaves the saved one in place.
    pub fn set_protocol_configuration(&mut self, profile: VpnProfile) -> Result<(), TunnelError> {
        if profile.name.chars().any(char::is_control) {
            return Err(TunnelError::InvalidConfiguration(
                "the profile name cannot contain control characters".to_string(),
            ));
        }
        profile.validate()?;
        self.profile = Some(profile);
        Ok(())
    }

    /// Enable or disable the VPN. Enabling needs a saved configuration that the system can run.
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), TunnelError> {
        if enabled {
            let profile = self.profile.as_ref().ok_or_else(|| {
                TunnelError::InvalidConfiguration("set a protocol configuration before enabling the VPN".to_string())
            })?;
            if service_type(profile.protocol).is_none() {
                return Err(TunnelError::InvalidConfiguration(
                    "this protocol is implemented by an app provider, not by the system".to_string(),
                ));
            }
        }
        self.enabled = enabled;
        Ok(())
    }

    /// Remove the configuration and disable the VPN, as `removeFromPreferences` does.
    pub fn remove_from_preferences(&mut self) {
        self.enabled = false;
        self.profile = None;
    }

    /// The status to show, given the session state (`None` when no session exists).
    pub fn status(&self, state: Option<TunnelState>) -> VpnStatus {
        if self.profile.is_none() {
            return VpnStatus::Invalid;
        }
        match state.unwrap_or(TunnelState::Disconnected) {
            TunnelState::Disconnected => VpnStatus::Disconnected,
            TunnelState::Connecting => VpnStatus::Connecting,
            TunnelState::Connected => VpnStatus::Connected,
            TunnelState::Reasserting => VpnStatus::Reasserting,
            TunnelState::Disconnecting => VpnStatus::Disconnecting,
        }
    }

    /// Render the saved configuration as a NetworkManager keyfile. Fails when no configuration is saved or when the
    /// protocol is implemented by an app provider.
    pub fn networkmanager_keyfile(&self) -> Result<String, TunnelError> {
        let profile = self
            .profile
            .as_ref()
            .ok_or_else(|| TunnelError::InvalidConfiguration("there is no VPN configuration".to_string()))?;
        let service = service_type(profile.protocol).ok_or_else(|| {
            TunnelError::InvalidConfiguration("this protocol is implemented by an app provider, not by the system".to_string())
        })?;
        profile.validate()?;
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = writeln!(out, "[connection]");
        let _ = writeln!(out, "id={}", profile.name);
        let _ = writeln!(out, "type=vpn");
        let _ = writeln!(out, "autoconnect=false");
        out.push('\n');
        let _ = writeln!(out, "[vpn]");
        let _ = writeln!(out, "service-type={}", service);
        let _ = writeln!(out, "remote={}", profile.server);
        let _ = writeln!(out, "port={}", profile.port);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> VpnProfile {
        VpnProfile::new("Office", "vpn.example.net", VpnProtocol::Ikev2)
    }

    #[test]
    fn new_manager_is_disabled_without_configuration() {
        let m = VpnManager::new();
        assert!(!m.is_enabled());
        assert!(m.protocol_configuration().is_none());
        assert!(m.networkmanager_keyfile().is_err());
        assert_eq!(m.status(None), VpnStatus::Invalid);
    }

    #[test]
    fn enabling_requires_a_saved_configuration() {
        let mut m = VpnManager::new();
        assert!(m.set_enabled(true).is_err());
        assert!(!m.is_enabled());
        m.set_protocol_configuration(profile()).unwrap();
        m.set_enabled(true).unwrap();
        assert!(m.is_enabled());
        m.set_enabled(false).unwrap();
        assert!(!m.is_enabled());
    }

    #[test]
    fn refused_profile_keeps_the_saved_one() {
        let mut m = VpnManager::new();
        m.set_protocol_configuration(profile()).unwrap();
        let mut bad = profile();
        bad.server = "https://vpn.example.net".to_string();
        assert!(m.set_protocol_configuration(bad).is_err());
        assert_eq!(m.protocol_configuration(), Some(&profile()));
    }

    #[test]
    fn control_characters_in_the_name_are_refused() {
        let mut m = VpnManager::new();
        let mut p = profile();
        p.name = "Office\nremote=elsewhere".to_string();
        assert!(m.set_protocol_configuration(p).is_err());
        assert!(m.protocol_configuration().is_none());
    }

    #[test]
    fn ikev2_keyfile_names_the_plugin_service_and_never_autoconnects() {
        let mut m = VpnManager::new();
        let mut p = profile();
        p.on_demand = true;
        m.set_protocol_configuration(p).unwrap();
        let text = m.networkmanager_keyfile().unwrap();
        assert!(text.starts_with("[connection]\nid=Office\ntype=vpn\nautoconnect=false\n\n[vpn]\n"));
        assert!(text.contains("service-type=org.freedesktop.NetworkManager.strongswan\n"));
        assert!(text.contains("remote=vpn.example.net\n"));
        assert!(text.contains("port=500\n"));
    }

    #[test]
    fn ikev1_keyfile_uses_its_own_plugin_service() {
        let mut m = VpnManager::new();
        m.set_protocol_configuration(VpnProfile::new("Legacy", "10.0.0.1", VpnProtocol::IpsecIkev1)).unwrap();
        let text = m.networkmanager_keyfile().unwrap();
        assert!(text.contains("service-type=org.freedesktop.NetworkManager.libreswan\n"));
    }

    #[test]
    fn app_provider_protocol_has_no_system_keyfile_and_cannot_be_enabled() {
        let mut m = VpnManager::new();
        m.set_protocol_configuration(VpnProfile::new("Custom", "relay.example.net", VpnProtocol::Tunnel)).unwrap();
        assert!(m.networkmanager_keyfile().is_err());
        assert!(m.set_enabled(true).is_err());
        assert!(!m.is_enabled());
    }

    #[test]
    fn remove_clears_configuration_and_disables() {
        let mut m = VpnManager::new();
        m.set_protocol_configuration(profile()).unwrap();
        m.set_enabled(true).unwrap();
        m.remove_from_preferences();
        assert!(!m.is_enabled());
        assert!(m.protocol_configuration().is_none());
        assert_eq!(m.status(None), VpnStatus::Invalid);
    }

    #[test]
    fn status_follows_configuration_and_session_state() {
        let mut m = VpnManager::new();
        m.set_protocol_configuration(profile()).unwrap();
        assert_eq!(m.status(None), VpnStatus::Disconnected);
        assert_eq!(m.status(Some(TunnelState::Connecting)), VpnStatus::Connecting);
        assert_eq!(m.status(Some(TunnelState::Connected)), VpnStatus::Connected);
        assert_eq!(m.status(Some(TunnelState::Reasserting)), VpnStatus::Reasserting);
        assert_eq!(m.status(Some(TunnelState::Disconnecting)), VpnStatus::Disconnecting);
    }
}
