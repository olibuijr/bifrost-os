//! `bifrost-audio-alsa`: the ALSA driver of `bifrost-audiod` in pure Rust over the kernel uapi (owner: task 04):
//! PCM hw and sw params, mmap of the data, status and control pages (SYNC_PTR fallback), the DLL, timerfd
//! scheduling, capture and playback linking, ctl elements, jack events and ELD names. Implements
//! `bifrost_audio_driver::{Driver, DeviceIo}` as `AlsaDriver`. Moves the uapi structs and ioctl numbers from
//! `springboard/src/media/alsa.rs` and `mixer.rs` (their layout tests come along).
