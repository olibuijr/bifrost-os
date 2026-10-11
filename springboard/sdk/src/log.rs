//! Unified Logging for apps (`os.Logger`, `OSLogStore`): the implementation is `bifrost_core::log` so the shell,
//! which cannot depend on the SDK, shares it. See docs/bifrost-ui.md, "Unified Logging".
pub use bifrost_core::log::*;
pub use bifrost_core::signpost;
