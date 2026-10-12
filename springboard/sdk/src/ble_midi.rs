//! MIDI over Bluetooth Low Energy packets, for apps that talk to a BLE MIDI peripheral.
//!
//! The framing lives in `bifrost_midi::ble`, the codec the MIDI server uses: a header and timestamp byte per
//! message, running status, and SysEx. An app passes the bytes it reads from the MIDI I/O characteristic to
//! `decode`, and sends the packet that `encode` returns.

pub use bifrost_midi::ble::*;
