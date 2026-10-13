//! Browser-style web extensions for apps: the identity, host permissions, enable state and message shape a web extension or
//! content blocker declares. The model lives in `bifrost_ui::web_extension` so the shell and apps share one format; this
//! module re-exports it for `bifrost_app`. See `docs/bifrost-ui.md` for the decisions.

pub use bifrost_ui::web_extension::{Extension, HostPattern, Kind, Message, Registry, Error};
