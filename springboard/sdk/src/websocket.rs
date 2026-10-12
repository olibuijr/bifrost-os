//! WebSocket framing parity: the app-facing names for the RFC 6455 frame codec in `bifrost_core::websocket`.
//!
//! The codec encodes and decodes frames and computes the `Sec-WebSocket-Accept` value of the opening
//! handshake. It opens no connection: an app owns its transport and passes the bytes it received to
//! `decode`, and writes what `encode` returns. Client frames take a masking key, server frames take `None`.

pub use bifrost_core::websocket::{accept_key, decode, encode, Error, Frame, Opcode};
