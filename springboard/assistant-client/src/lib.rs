//! The Anthropic Messages client shared by the Assistant app (`apps/assistant-desktop`, streaming with tools) and the
//! shell (`src/shell/assistant.rs`, one intent call on a worker thread). This crate holds what both need and nothing
//! that is app-specific: the wire format ([`wire`]), the stream reader ([`sse`]) and, in later modules, the intent
//! protocol and the blocking transport. It depends on `bifrost-core` only, so the shell may depend on it.

pub mod sse;
pub mod wire;

/// The keychain item that holds the Anthropic API key: the service and the account of one generic password.
pub const KEY_SERVICE: &str = "is.olibuijr.assistant";
pub const KEY_ACCOUNT: &str = "anthropic-api-key";
