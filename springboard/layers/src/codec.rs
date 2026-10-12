//! The wire encoding of a `Transaction` for `bifrost_layer_context_v1.commit` (a Wayland `array`).
//!
//! Format: little endian; a header `b"BLT1"`, `u64 seq`, `u8 implicit` (0 none, 1 then `f64 duration` and a timing
//! function), `u32 count`, then each change as `u8 tag` and its fields. Floats are IEEE `f32`/`f64`; enums are `u8`.
//! A decoder never panics on hostile input: lengths are checked, unknown tags are an error, the whole blob is at most
//! `MAX_LEN` bytes.
//!
//! Owner: wave G1 agent `layers-codec`.

use crate::transaction::Transaction;

/// The largest accepted transaction blob (the Wayland array limit leaves room; bigger trees commit in several).
pub const MAX_LEN: usize = 1 << 20;
pub const MAGIC: &[u8; 4] = b"BLT1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLong,
    BadMagic,
    Truncated,
    UnknownTag(u8),
    BadValue,
}

/// Encodes a transaction.
pub fn encode(tx: &Transaction) -> Vec<u8> {
    let _ = tx;
    todo!("layers-codec")
}

/// Decodes a transaction; `decode(&encode(tx)) == Ok(tx)` for every transaction.
pub fn decode(bytes: &[u8]) -> Result<Transaction, DecodeError> {
    let _ = bytes;
    todo!("layers-codec")
}
