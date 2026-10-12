//! The libjack ABI replacement (owner: task 18): client, ports, process callback and transport, mapped onto HAL
//! IO procedures and the process-local graph (docs/audio/18-jack.md). The crate builds `libjack.so`; its exported
//! symbols are the functions of the public JACK headers.

pub mod driver;
pub mod engine;
pub mod ffi;
pub mod midi;
pub mod pattern;
pub mod ringbuffer;
