//! The BifrOSt installer: a native Rust program that draws itself on the DRM/KMS display and reads the
//! keyboard and mouse directly (the `bifrost-installer` binary). Everything that is not drawing lives in
//! testable modules here.

pub mod app;
pub mod disks;
pub mod keymaps;
pub mod plan;
pub mod unattended;
pub mod keymaps_data;
