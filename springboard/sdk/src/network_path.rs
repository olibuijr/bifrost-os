//! Network path parity: the app-facing names for the connectivity model in `bifrost_core::net_path`.
//!
//! An app reads the path through `PathMonitor`: it feeds each report to `update` and gets the new
//! `Path` back only when the status, the interfaces or the address families changed. The shell does
//! not yet push live link state into this model (netd publishes Wi-Fi status over its own socket),
//! so an app that runs today sees the path its own caller builds. The model is pure std with no
//! socket and no interface lookup.

pub use bifrost_core::net_path::{IfaceKind, Interface, Path, PathMonitor, Status};
