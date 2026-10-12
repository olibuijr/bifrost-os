//! WebSocket parity: the app-facing names for the RFC 6455 codec and handshake in `bifrost_core::websocket`.
//!
//! The codec encodes and decodes frames and computes the `Sec-WebSocket-Accept` value of the opening
//! handshake. It opens no connection: an app owns its transport and passes the bytes it received to
//! `decode`, and writes what `encode` returns. Client frames take a masking key, server frames take `None`.
//!
//! The task-level pieces of `URLSessionWebSocketTask` are the handshake and the message assembler: the app sends
//! `handshake_request`, checks the answer with `check_handshake_response`, then feeds each decoded frame to
//! `Assembler::push` and receives whole `Message` values (text, binary, ping, pong, close). `frame_for` sends one.

pub use bifrost_core::websocket::{
    accept_key, check_handshake_response, decode, encode, frame_for, handshake_request, Assembler, AssemblyError, Error,
    Frame, HandshakeError, Message, Opcode,
};
