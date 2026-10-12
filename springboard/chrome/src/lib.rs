//! `bifrost-chrome`: the desktop chrome as separate clients of the window server (docs/graphics-architecture.md,
//! "Dock, menu bar, SystemUIServer"). The shell keeps the session and the models; these processes draw them on the
//! GPU in layer-shell surfaces with layer trees, so a busy shell never stalls the menu bar or the Dock.
//!
//! - `model`: the messages between the shell and the chrome clients (lead contract; extend only additively; the
//!   codec is `chrome-core`'s).
//! - `link`: the socket both sides use: `$XDG_RUNTIME_DIR/bifrost-chrome.sock`, served by the shell; one message per
//!   `SOCK_SEQPACKET` packet. Owner: `chrome-core`.
//! - `surface`: a layer-shell surface drawn with `bifrost-2d` into dma-buf backing stores and committed as a layer
//!   tree (shared by the three clients). Owner: `chrome-core`.
//! - `menubar`, `dock`, `panels`: the clients. Owners: `chrome-menubar`, `chrome-dock`, `chrome-panels`.
//!
//! The shell side lives in `springboard/src/shell/chrome_host/` (one file per client, same owners).

pub mod dock;
pub mod link;
pub mod menubar;
pub mod model;
pub mod panels;
pub mod surface;
