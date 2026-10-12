//! Personal VPN configuration for apps: the app-facing names for `bifrost_core::vpn_manager` and the tunnel models it
//! uses (the reference `NEVPNManager`, `NEVPNProtocol` and `NEVPNStatus` shapes).
//!
//! The configuration holds no secrets and changes no network state. The system layer renders it with
//! `VpnManager::networkmanager_keyfile`, writes that file to the connection manager and brings the tunnel up.

pub use bifrost_core::tunnel::{StopReason, TunnelError, TunnelState, VpnProfile, VpnProtocol};
pub use bifrost_core::vpn_manager::{VpnManager, VpnStatus};
