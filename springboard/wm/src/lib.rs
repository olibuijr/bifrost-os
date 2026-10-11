//! BifrOSt window server (`docs/window-server.md`). The window-management rules live in `wm`, free of Smithay types.

pub mod grabs;
pub mod handlers;
pub mod headless;
pub mod input;
#[cfg(feature = "nested")]
pub mod nested;
pub mod state;
pub mod wm;
