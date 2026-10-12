//! Protocol glue for `bifrost_layer_v1` (docs/graphics-architecture.md, "bifrost_layer_v1"): what each request does to
//! the `ContextStore`, and which protocol error a bad request maps to. Every function here takes plain values, so the
//! request semantics are host-tested without a client. The Wayland `Dispatch` impls for `bifrost_layer_manager_v1`
//! and `bifrost_layer_context_v1` (global creation, object data, sending `presented` and `animation_finished`) are
//! thin wrappers around these functions; they need the server bindings `bifrost_layers::protocol` generates from
//! the XML, which are not generated yet (see docs/graphics/g1-wm-layers.md).
//!
//! Owner: wave G1 agent `wm-layers`.

use super::layers::{ContextStore, DamageRect};
use bifrost_layers::codec::{self, DecodeError};
use bifrost_layers::ContentsId;

/// Protocol error codes, from the `error` enums of `bifrost_layer_v1`.
pub const ERROR_ALREADY_HAS_CONTEXT: u32 = 0;
pub const ERROR_BAD_TRANSACTION: u32 = 0;
pub const ERROR_BAD_BUFFER: u32 = 1;

/// A request the client must be disconnected for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// `get_context` on a surface that already has a context (manager error `already_has_context`).
    AlreadyHasContext,
    /// `commit` whose blob does not decode (context error `bad_transaction`).
    BadTransaction(DecodeError),
    /// `bind_contents` with a buffer that cannot be imported (context error `bad_buffer`).
    BadBuffer,
}

impl RequestError {
    /// The protocol error value to post for this error, on the interface the error belongs to.
    pub fn code(&self) -> u32 {
        match self {
            RequestError::AlreadyHasContext => ERROR_ALREADY_HAS_CONTEXT,
            RequestError::BadTransaction(_) => ERROR_BAD_TRANSACTION,
            RequestError::BadBuffer => ERROR_BAD_BUFFER,
        }
    }
}

/// `bifrost_layer_manager_v1.get_context`: creates the context for a surface.
pub fn get_context(store: &mut ContextStore, ctx: u32) -> Result<(), RequestError> {
    if store.create(ctx) {
        Ok(())
    } else {
        Err(RequestError::AlreadyHasContext)
    }
}

/// `bifrost_layer_context_v1.bind_contents` after the glue imported the buffer under `key`.
/// Returns `false` when the context is unknown (the glue then ignores the request).
pub fn bind_contents(store: &mut ContextStore, ctx: u32, contents: u32, key: u64) -> bool {
    store.bind(ctx, ContentsId(contents), Some(key))
}

/// `bifrost_layer_context_v1.unbind_contents`.
pub fn unbind_contents(store: &mut ContextStore, ctx: u32, contents: u32) -> bool {
    store.bind(ctx, ContentsId(contents), None)
}

/// `bifrost_layer_context_v1.damage_contents`. A rectangle with no area is dropped (it changes nothing); the return
/// value says whether it was queued.
pub fn damage_contents(store: &mut ContextStore, ctx: u32, contents: u32, rect: DamageRect) -> bool {
    if rect.2 <= 0 || rect.3 <= 0 {
        return false;
    }
    store.damage(ctx, ContentsId(contents), rect)
}

/// `bifrost_layer_context_v1.commit`: decodes the blob and queues it. A blob that does not decode is
/// `bad_transaction`, and nothing is queued.
pub fn commit(store: &mut ContextStore, ctx: u32, blob: &[u8]) -> Result<(), RequestError> {
    let tx = codec::decode(blob).map_err(RequestError::BadTransaction)?;
    store.commit(ctx, tx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_layers::Transaction;

    #[test]
    fn get_context_twice_is_already_has_context() {
        let mut s = ContextStore::default();
        assert_eq!(get_context(&mut s, 5), Ok(()));
        let err = get_context(&mut s, 5).unwrap_err();
        assert_eq!(err, RequestError::AlreadyHasContext);
        assert_eq!(err.code(), 0);
        assert!(s.contains(5));
    }

    #[test]
    fn error_codes_match_the_protocol_xml() {
        // bifrost_layer_v1.xml: manager error already_has_context = 0; context errors bad_transaction = 0,
        // bad_buffer = 1.
        assert_eq!(RequestError::BadBuffer.code(), 1);
        assert_eq!(RequestError::BadTransaction(DecodeError::BadMagic).code(), 0);
    }

    #[test]
    fn bind_and_unbind_need_a_known_context() {
        let mut s = ContextStore::default();
        assert!(!bind_contents(&mut s, 1, 2, 3));
        assert!(!unbind_contents(&mut s, 1, 2));
        get_context(&mut s, 1).unwrap();
        assert!(bind_contents(&mut s, 1, 2, 3));
        assert!(unbind_contents(&mut s, 1, 2));
        assert_eq!(
            s.contexts[&1].pending_bindings,
            vec![(ContentsId(2), Some(3)), (ContentsId(2), None)]
        );
    }

    #[test]
    fn damage_with_no_area_is_dropped() {
        let mut s = ContextStore::default();
        get_context(&mut s, 1).unwrap();
        assert!(!damage_contents(&mut s, 1, 2, (0, 0, 0, 10)));
        assert!(!damage_contents(&mut s, 1, 2, (0, 0, 10, -1)));
        assert!(damage_contents(&mut s, 1, 2, (1, 2, 3, 4)));
        assert_eq!(s.contexts[&1].pending_damage, vec![(ContentsId(2), (1, 2, 3, 4))]);
    }

    #[test]
    fn commit_of_a_bad_blob_is_bad_transaction_and_queues_nothing() {
        let mut s = ContextStore::default();
        get_context(&mut s, 1).unwrap();
        let err = commit(&mut s, 1, b"not a transaction").unwrap_err();
        assert!(matches!(err, RequestError::BadTransaction(_)));
        assert_eq!(err.code(), ERROR_BAD_TRANSACTION);
        assert!(s.contexts[&1].pending.is_empty());
    }

    #[test]
    fn commit_of_an_encoded_transaction_queues_it() {
        let mut s = ContextStore::default();
        get_context(&mut s, 1).unwrap();
        let blob = codec::encode(&Transaction::new(12));
        commit(&mut s, 1, &blob).unwrap();
        assert_eq!(s.contexts[&1].pending[0].seq, 12);
    }
}
