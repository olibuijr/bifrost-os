//! BifrOSt window server (`docs/window-server.md`). The window-management rules live in `wm`, free of Smithay types.

pub mod background_effect;
pub mod boot;
#[cfg(feature = "drm")]
pub mod drm;
#[cfg(feature = "drm")]
pub mod drm_vulkan;
pub mod glass;
pub mod grabs;
pub mod handlers;
pub mod headless;
pub mod input;
pub mod lock;
pub mod render_server;
#[cfg(feature = "nested")]
pub mod nested;
pub mod scene;
pub mod state;
pub mod wm;

/// Starts a client on the compositor's socket.
pub fn spawn(command: &[String], socket: &str) {
    if let Some((prog, rest)) = command.split_first() {
        if let Err(e) = std::process::Command::new(prog).args(rest).env("WAYLAND_DISPLAY", socket).env_remove("DISPLAY").spawn() {
            eprintln!("bifrost-wm: {prog}: {e}");
        }
    }
}
