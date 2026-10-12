//! `bifrost-bootframe`: the single description and renderer of the boot frame (docs/boot-architecture.md, "One
//! frame"), and the boot timing record behind the progress bar ("The progress bar"). `no_std` plus `alloc`, no
//! dependencies, so the UEFI loader (`springboard/bootweave/efi`), Bootweave, the installer and the window server all draw the
//! same pixels from the same code.
//!
//! Owners (boot team): `render` and the screens (task 02), `progress` and `timing` (task 03; task 06 writes records).

#![no_std]
extern crate alloc;

pub mod progress;
pub mod render;
pub mod timing;

pub use progress::Progress;
pub use render::{render, PixelOrder};
pub use timing::{BootTiming, Stage};

/// Light or Dark, as the user last chose (stored with the timing record so the loader knows it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// Which screen the frame shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Screen {
    /// The mark (and the progress bar when `Frame::progress` is set).
    #[default]
    Boot,
    /// Pre-boot unlock: the user's name, a password field with `dots` dots, and whether the last try failed.
    Unlock { name: alloc::string::String, dots: u32, failed: bool },
    /// The boot options (held Shift): entries and the selected one.
    Options { entries: alloc::vec::Vec<alloc::string::String>, selected: u32 },
    /// Starting the previous version after a failed update.
    Rollback,
    /// The session could not start: Restart and Recovery, the selected button.
    StartFailed { selected: u32 },
    /// After a kernel panic: "Your computer restarted because of a problem".
    RestartedAfterProblem,
    ShuttingDown,
}

/// One boot frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub appearance: Appearance,
    /// 0..1 when the progress bar shows, `None` to hide it.
    pub progress: Option<f32>,
    pub screen: Screen,
}
