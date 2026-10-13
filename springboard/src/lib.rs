//! BifrOSt OS shell. Shared rendering and infrastructure live in independent
//! bifrost-render, bifrost-ui and bifrost-core crates.

pub mod bootreveal;
pub mod bootsplash;
pub mod catalog;
pub mod cff;
pub mod emoji;
pub mod first_frame_cache;
pub mod font;
pub mod gfx;
pub mod gpu;
pub mod gvar;
pub mod imgdec;
pub mod keychain;
pub mod installer;
pub mod layout;
pub mod media;
pub mod pager;
pub mod pngenc;
pub mod pstore;
#[cfg(target_os = "linux")]
pub mod privd;
#[cfg(target_os = "linux")]
pub mod privd_client;
pub mod scene;
pub mod shell;
pub mod status;
pub mod svc;
pub mod symbols;
#[cfg(target_os = "linux")]
pub mod term;
#[cfg(target_os = "linux")]
pub mod deildu;
#[cfg(target_os = "linux")]
pub mod files;
#[cfg(target_os = "linux")]
pub mod thumbs;


#[cfg(target_os = "linux")]
pub mod appearance_portal;
#[cfg(target_os = "linux")]
pub mod bare;

#[cfg(target_os = "linux")]
pub mod net;
