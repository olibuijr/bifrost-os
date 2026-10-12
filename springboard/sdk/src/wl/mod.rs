//! The direct path: the app as a Wayland client of the window server (docs/graphics-architecture.md, "SDK").
//!
//! Selected with `BIFROST_DIRECT=1` (until the switch-over) when a Vulkan device opens and the compositor offers
//! `bifrost_layer_manager_v1`; otherwise the shared-memory path through the shell stays in use.
//!
//! - `connection`: connect to `WAYLAND_DISPLAY`, bind globals (compositor, xdg_wm_base, linux-dmabuf v4 with
//!   feedback, presentation-time, bifrost_layer_manager_v1, seat, cursor-shape), one xdg toplevel with the app id,
//!   configure/close handling. Owner: `sdk-wl-connection`.
//! - `backing`: backing stores as exportable `bifrost_gpu` textures, a pool of 2-3 per layer, wrapped as dma-buf
//!   `wl_buffer`s (zwp_linux_buffer_params_v1) and bound to contents ids; released buffers return to the pool.
//!   Owner: `sdk-wl-backing`.
//! - `input`: wl_seat pointer, keyboard (xkb keymap from the compositor, repeat info), axis with value120 and
//!   kinetic stop; converted to the SDK's `Input` events. Owner: `sdk-wl-input`.
//! - `frame`: draws a frame: the app's `draw` records into a `DisplayList` (the `Canvas` the SDK passes), `bifrost-2d`
//!   renders it into the next backing store with its damage, the root layer's contents is bound, one transaction is
//!   committed; paced by the existing `crate::display_link::DisplayLink`, which this owner extends with
//!   `presented(host_time, refresh)` so its phase follows the compositor's presentation feedback. Owner: `sdk-wl-frame`.

pub mod backing;
pub mod connection;
pub mod frame;
pub mod input;

/// Whether the direct path is requested.
pub fn requested() -> bool {
    std::env::var("BIFROST_DIRECT").map(|v| v == "1").unwrap_or(false)
}
