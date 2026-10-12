//! BifrOSt window server (`docs/window-server.md`). The window-management rules live in `wm`, free of Smithay types.

pub mod background_effect;
pub mod boot;
pub mod decorations;
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
pub mod session_protocols;
pub mod state;
pub mod sync;
pub mod wm;
#[cfg(feature = "xwayland")]
pub mod xwayland;

/// Starts a client on the compositor's socket.
pub fn spawn(command: &[String], socket: &str) {
    if let Some((prog, rest)) = command.split_first() {
        let mut child = std::process::Command::new(prog);
        child.args(rest).env("WAYLAND_DISPLAY", socket);
        // X11 clients get XWayland's display once it is ready (`xwayland::start` runs the command then).
        #[cfg(feature = "xwayland")]
        match xwayland::display() {
            Some(display) => child.env("DISPLAY", display),
            None => child.env_remove("DISPLAY"),
        };
        #[cfg(not(feature = "xwayland"))]
        child.env_remove("DISPLAY");
        if let Err(e) = child.spawn() {
            eprintln!("bifrost-wm: {prog}: {e}");
        }
    }
}
