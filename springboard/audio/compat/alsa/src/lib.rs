//! The ALSA compatibility plug-in (owner: task 08): a cdylib exporting `_snd_pcm_bifrost_open` on alsa-lib's
//! ioplug API (the few C symbols declared by hand in `ffi`), playback and capture through `bifrost-audio-hal`.
//!
//! Data path: the application thread (alsa-lib's `transfer` callback) converts its samples to f32 and pushes them
//! into a `PcmRing` of one ALSA buffer; the HAL IO procedure (the library's real-time thread) pops from it each
//! cycle. Capture runs the other way. `pointer` is the ring position and `delay` is the ring fill, so
//! `snd_pcm_delay` reports the ring delay. The IO procedure never allocates, locks or makes a syscall other than
//! the futex wake of a small helper thread, which signals the poll eventfd.
//!
//! `/etc/alsa/conf.d/50-bifrost.conf` routes `pcm.!default` through alsa-lib's `plug` onto this plug-in, so rate,
//! channel and format conversion are alsa-lib's; the plug-in offers the device's nominal rate only.

pub mod ffi;
pub mod plugin;
pub mod ring;
