//! The account key pair and the HPKE seal and open helpers, for apps: Find My (Locate) and the shell's Universal Clipboard
//! seal with the same functions. The implementation is `springboard::cloud_keys`; this module re-exports it so an app
//! reaches it as `bifrost_app::cloud_keys`.

pub use springboard::cloud_keys::*;
