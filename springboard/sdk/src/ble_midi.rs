//! MIDI over Bluetooth Low Energy packets, for apps that talk to a BLE MIDI peripheral.
//!
//! The framing lives in `bifrost_core::ble_midi`: a header and timestamp byte per message, running status, and SysEx
//! that continues across packets. This module is the app-facing name for it. An app passes the bytes it reads from the
//! MIDI I/O characteristic to `Decoder::decode`, and sends the packets that `encode` returns, one per write.

pub use bifrost_core::ble_midi::{encode, BleMidiError, Decoder, TimedMessage, MAX_TIMESTAMP_MS};
