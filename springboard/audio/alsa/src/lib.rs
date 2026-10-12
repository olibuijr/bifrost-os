//! `bifrost-audio-alsa`: the ALSA driver of `bifrost-audiod` in pure Rust over the kernel uapi (owner: task 04):
//! PCM hw and sw params, mmap of the data, status and control pages (SYNC_PTR fallback), the DLL, timerfd
//! scheduling, capture and playback linking, ctl elements, jack events and ELD names. Implements
//! `bifrost_audio_driver::{Driver, DeviceIo}` as `AlsaDriver`. The uapi structs and ioctl numbers were moved
//! (copied) from `springboard/src/media/alsa.rs` and `mixer.rs`, with their layout tests.
//!
//! Layers, bottom up:
//! - `sys`: ioctl encoding and calls, device open, the clocks.
//! - `uapi` (PCM) and `ctl` (control device): kernel structs, request numbers, element access, jacks, ELD.
//! - `chmap`: the PCM channel-map TLV and values parsed into a channel layout (pure, host-testable).
//! - `convert` and `ring`: sample formats and the DMA ring arithmetic (pure, host-testable).
//! - `pcm`: one stream (negotiation, mmap, position, application pointer, linking).
//! - `dll` and `timer`: the clock model and the timerfd the server sleeps on.
//! - `driver`: `AlsaDriver` and `AlsaIo`.
//!
//! libasound is not used. `unsafe` is confined to the syscalls and the mmapped pages, each with a SAFETY comment.

pub mod chmap;
pub mod convert;
pub mod ctl;
pub mod dll;
pub mod driver;
pub mod pcm;
pub mod ring;
pub mod sys;
pub mod timer;
pub mod uapi;

pub use chmap::Chmap;
pub use ctl::{Control, Ctl, Jack};
pub use dll::Dll;
pub use driver::{AlsaDriver, AlsaIo, AlsaOptions};
pub use pcm::{Direction, Pcm, PcmCaps, PcmRequest};
pub use timer::CycleTimer;
