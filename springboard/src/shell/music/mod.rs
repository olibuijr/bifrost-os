//! The parts of the Music app the shell still draws itself: the lock-screen Now Playing
//! card, the Control Center tile geometry, the radio station list (also used by Podcasts and
//! Weather for HTTP/JSON) and the app icon. The app itself is `apps/music-desktop`.

pub mod icons;
pub mod nowplaying;
pub mod radio;
mod system;

pub use system::{draw_cc_controls, LockCardView};
