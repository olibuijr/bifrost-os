//! The shell side of `bifrost-chrome`: serves `$XDG_RUNTIME_DIR/bifrost-chrome.sock`, builds the models from the
//! shell's state and handles what the chrome clients send back. Enabled with `BIFROST_CHROME=1` until the
//! switch-over; the shell keeps drawing its own menu bar, Dock and panels otherwise.
//!
//! Owners (wave G2): `server.rs` (`chrome-core`: the `HostLink` in the shell loop), `menubar.rs` (`chrome-menubar`), `dock.rs`
//! (`chrome-dock`), `panels.rs` (`chrome-panels`).

pub mod dock;
pub mod menubar;
pub mod panels;
pub mod server;

/// Whether the separate chrome clients are requested.
pub fn requested() -> bool {
    std::env::var("BIFROST_CHROME").map(|v| v == "1").unwrap_or(false)
}
