//! Packet tunnels for apps: the host side of a custom VPN provider.
//!
//! A provider is a program that speaks its own protocol to a server. It needs a virtual interface to read the packets the
//! system routes into the tunnel, and the routes that decide which traffic that is. `TunDevice` is the interface (a Linux
//! `tun` device moving whole IP packets), `interface_commands` and `route_commands` give the `ip` invocations that address
//! it and install a full-tunnel or split-tunnel policy, `TunnelSession` is the life cycle the provider reports through,
//! `VpnProfileStore` keeps the saved servers, and `classify_packet` tells a provider whether a packet belongs in the tunnel.
//! Opening the device and running `ip` need the privilege the shell has; a provider app asks the shell to set the
//! tunnel up rather than doing it itself.
pub use bifrost_core::tun_device::{
    interface_commands, route_commands, run_commands, valid_interface_name, PlanError, TunDevice, Uplink,
};
pub use bifrost_core::tunnel::{
    classify_packet, parse_ipv4, IllegalTransition, Ipv4Header, Ipv4Route, PacketError, RouteAction, RoutePolicy, StopReason,
    TunnelError, TunnelSession, TunnelState, VpnProfile, VpnProtocol,
};
pub use bifrost_core::vpn_profiles::VpnProfileStore;
