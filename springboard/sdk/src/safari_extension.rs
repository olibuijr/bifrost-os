//! Safari-style web extensions for apps: the identity, host permissions, enable state and message shape a web extension or
//! content blocker declares. The model lives in `bifrost_ui::safari_extension` so the shell and apps share one format; this
//! module re-exports it for `bifrost_app`. See `docs/parity/safari-services.md` for the decisions.

pub use bifrost_ui::safari_extension::{Extension, HostPattern, Kind, Message, Registry, Error};
