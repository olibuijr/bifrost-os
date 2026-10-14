//! The app-facing surface for pointer lock, band selection and the focus halo. The models live in `bifrost_ui`; this
//! module re-exports them under the SDK so an app writes `pointer_focus::band_select::BandSelection`.
//!
//! ```ignore
//! use bifrost_app::pointer_focus::{band_select::{BandSelection, Item}, focus_halo};
//! // on a drag over empty space:
//! let mut band = BandSelection::begin(x, y, selected.clone(), shift_held);
//! if let Some(rect) = band.update(px, py) { draw_band(rect); }
//! let picked = band.finish(&items);
//! // on keyboard focus, around the focused control with corner radius r:
//! if let Some(h) = focus_halo::halo(frame, r) { draw_ring(h.inner, h.inner_radius, h.outer, h.outer_radius); }
//! ```
//!
//! A scene asks for the pointer with `pointer_lock::PointerLock::request`; the shell owns one `PointerLock` and decides
//! which scene is key. A tablet has no pointer, so the lock and band selection apply to a pointer or a finger drag and
//! the halo applies to keyboard focus.

pub use bifrost_ui::band_select;
pub use bifrost_ui::focus_halo;
pub use bifrost_ui::pointer_lock;
