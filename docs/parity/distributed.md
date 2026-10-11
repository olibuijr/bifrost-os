# Distributed parity

Status: partial (was partial). Matrix row: `docs/parity-matrix/data/distributed.json`. Code: `springboard/sdk/src/distributed.rs`.

## What was built

The call side of the Distributed model, in pure Rust with std only. No new crate, no `unsafe`, no network.

- **Remote call targets**: `RemoteCallTarget` validates a target identifier (non-empty, at most 256 bytes, no whitespace or control characters).
- **Typed invocation encoding**: `InvocationEncoder` records labelled arguments in order. Each argument is encoded through the existing `coding::Encodable` trait, so the typed values are the same ones `Codable` parity uses. `done_recording` yields an `Invocation`.
- **Typed invocation decoding**: `InvocationDecoder::next::<T>(label)` reads arguments back in recorded order. It refuses a wrong label, a wrong type, a missing argument, and leftover arguments.
- **Wire shape**: `Invocation` implements `Encodable`/`Decodable`, so `JSONEncoder`/`JSONDecoder` carry it as `{"target": ..., "arguments": [{"label": ..., "value": ...}]}`.
- **Result handler protocol**: `InvocationResultHandler` with `on_return`, `on_return_void` and `on_throw`. `CollectingResultHandler` stores the outcome for tests and in-process callers.
- **Local testing actor system**: `LocalTestingSystem` creates actors (`make_actor_id`), registers one handler per target (`register`), and runs an invocation (`execute`) synchronously. Actor ids are unique in the process, so an id from another system is refused.
- **Errors**: `DistributedError` covers unknown actor, unknown target, coding failure and handler failure. `From<DistributedError> for RpcError` maps them onto the control socket's codes: INVALID_PARAMS, METHOD_NOT_FOUND and INTERNAL_ERROR.

`springboard/sdk/src/lib.rs` gained one line, `pub mod distributed;`. `control.rs` is unchanged; the mapping is a `From` impl in the new module.

## Official API names it mirrors

From the official Distributed framework page (`https://developer.apple.com/documentation/distributed`), read with `python3 -I tools/parity/docc.py distributed --depth 2`:

- Remote Calls: `RemoteCallTarget`, `RemoteCallArgument`, `DistributedTargetInvocationEncoder`, `DistributedTargetInvocationDecoder`, `DistributedTargetInvocationResultHandler`.
- Local Testing: `LocalTestingDistributedActorSystem`, `LocalTestingActorID`, `LocalTestingInvocationEncoder`, `LocalTestingInvocationDecoder`, `LocalTestingInvocationResultHandler`.
- Errors: `DistributedActorCodingError`, `DistributedActorSystemError`, `ExecuteDistributedTargetError`, `LocalTestingDistributedActorSystemError`.

The names are mirrored by role. The Rust names (`InvocationEncoder`, `InvocationDecoder`, `InvocationResultHandler`, `LocalTestingSystem`, `ActorId`, `DistributedError`) are our own.

## Evidence

- REA evidence of the rendered official page: `ev_1837066359cf436d6652c89ee3ff05fbbcf959b09703bb335ac1355f5835dae8` (`tools/parity/rea-page.sh https://developer.apple.com/documentation/distributed`).
- The matrix row's earlier REA evidence, from the original assessment, is `ev_a850e0bce106bf6bdcdabceab1374b66f8fb8c4e4b415cae3b6e765ee45a3d74`.

## Behaviour and tests

Unit tests in `springboard/sdk/src/distributed.rs` (`mod tests`, nine tests). They were written but not run in this stage, per the no-build rule:

- target identifiers are validated;
- typed arguments survive a JSON round trip and decode with the same types and labels;
- the decoder rejects a wrong label, a wrong type, and a missing argument;
- the encoder refuses an empty label and a malformed target;
- `execute` returns the handler's value;
- an unknown actor and an unknown target are reported through the result handler;
- an unread argument is a coding error;
- a void handler calls `on_return_void`, and a failing handler reports its message;
- the errors map to the right JSON-RPC codes.

## Remaining gaps

- No location-transparent remote actor reference, no actor system that carries calls between processes, and no SDK handle to another app's object. The control socket is JSON-RPC, not encoded invocations.
- Invocations are not sent over any transport. `LocalTestingSystem` runs them in process only.
- The encoder has no generic substitution record and no return-type record.
- No actor address type (`LocalTestingActorAddress` has no counterpart) and no recording of invocations for assertions.
- No separate protocol error type for conforming actor systems. One enum covers the protocol and execution errors.
- Cross-app notifications remain string to string, capped at 32 entries and 1024 characters per value, with no `suspensionBehavior`. These are unchanged.
