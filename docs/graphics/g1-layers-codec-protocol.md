# G1: layers codec and `bifrost_layer_v1` bindings

Wave G1 part `layers-codec-protocol` of the graphics core (`docs/graphics-architecture.md`). Branch
`g1/layers-codec-protocol`, crate `springboard/layers` (`bifrost-layers`).

## What is implemented

### Codec (`springboard/layers/src/codec.rs`)

`encode(&Transaction) -> Vec<u8>` and `decode(&[u8]) -> Result<Transaction, DecodeError>`, the blob carried by
`bifrost_layer_context_v1.commit`.

- Header `BLT1`, `u64 seq`, optional implicit animation (`f64 duration` plus a timing function), then a `u32` count
  and the changes. Little endian, `f32`/`f64` as IEEE bits, enums as `u8` tags. The full tag table is in the
  module doc comment and is the wire contract (stable from version 1).
- Covers every `Change` (16 variants), every `Value` (6 kinds), every `Animation` (Basic, Keyframe, Spring,
  Group), `MediaTiming` (begin time option, duration, speed, time offset, repeat count, autoreverse, fill mode),
  every `TimingFunction` (6 variants including cubic Bezier), `Property` (15 codes), `Border`, `Shadow`, `Material`,
  `LayerContents`, `ContentsGravity`, `Affine`, `Rect`, `Point`, `Color`.
- Hostile input: `MAX_LEN` (1 MiB) refused up front; magic checked; every read bounds-checked (`Truncated`);
  list allocations capped by the bytes left, so a count of `u32::MAX` cannot allocate; unknown tags are
  `UnknownTag`; invalid bools, options, fill modes, gravities and enum codes are `BadValue`; trailing bytes are
  `BadValue`; layer id 0 is refused (the tree never uses it); time fields must be finite and durations not negative;
  animation groups nest at most `MAX_DEPTH` (32) deep so the recursive decoder cannot overflow the stack.
- The decoder contains no `unwrap`, no indexing that can panic and no `unsafe`.

### Protocol bindings (`springboard/layers/src/protocol/mod.rs`)

`client` and `server` modules generated at compile time from `protocol/bifrost-layer-v1.xml` by
`wayland_scanner::generate_interfaces!`, `generate_client_code!` and `generate_server_code!`, laid out as the
wayland-protocols 0.32 crate does (`__interfaces` submodule, then the code generators, `use wayland_client;` /
`use wayland_server;` in scope). Feature gated: `client` for the SDK, `server` for bifrost-wm. The XML path is
relative to `CARGO_MANIFEST_DIR`.

Generated names used by the rest of the code: `bifrost_layer_manager_v1::{Request, Error::AlreadyHasContext}`,
`bifrost_layer_context_v1::{Event, Error::{BadTransaction, BadBuffer}, EVT_PRESENTED_OPCODE,
EVT_ANIMATION_FINISHED_OPCODE}`.

## Mapping to the contract

| Contract item | Where |
|---|---|
| `codec`: compact little-endian, versioned, hostile-input safe | `codec.rs` (`encode`, `decode`, `DecodeError`, `MAX_LEN`, `MAGIC`) |
| `bifrost_layer_v1`: manager `get_context`, context `bind_contents`, `unbind_contents`, `damage_contents`, `commit(array)`, events `presented`, `animation_finished` | `protocol/bifrost-layer-v1.xml`, bindings in `protocol/mod.rs` |

The skeleton's public signatures are unchanged. No Cargo.toml edits, no new crates.

Contract change: none. The XML was not changed. Decoder rules beyond the skeleton's `DecodeError` variants (no new
variant was added): layer id 0, non-finite time values and negative durations are `BadValue`; trailing bytes are
`BadValue`.

## Verification

Done in this change:

- `cargo check --offline -p bifrost-layers --tests` with no features, `--features client --tests` and
  `--features server --tests`: all pass with no warnings (the protocol module compiles in both feature sets).
- `python3 tools/check-names.py`: no hits.
- Tests in `codec.rs` (`mod tests`): hand-worked bytes for an empty transaction (17 bytes) and for
  `Create(LayerId(1))` (22 bytes); round trips for every change variant, every value kind, every property code,
  every animation kind (including nested groups), every timing function, fill mode, gravity, material and contents;
  every strict prefix of a valid blob fails; hostile inputs (bad magic, too long, huge count, trailing bytes,
  unknown tag, bad bool, layer id 0, NaN and negative durations); deep group nesting at and past `MAX_DEPTH`;
  2000 pseudo-random blobs and every single-byte corruption decode without panicking.
- Protocol tests (`protocol/mod.rs`): the generated enums, opcodes and error codes match the XML order.

Not run here: `cargo test` (this round forbids running tests in the worktree). The integrator runs
`cargo test -p bifrost-layers --features client` and `--features server`. The hand-worked byte counts were checked
by script against the literals.

## What is left (todo_left)

- Run the tests (see above); no GPU or Midget step is needed for this part.
- The wire format has no checksum; the window server must still verify the transaction against the tree
  (`Tree::apply`), which belongs to `layers-model`.
- Versioning: the header is `BLT1`; a future format needs a new magic, not an in-place change.
- The protocol has no `wl_surface` damage or sync events yet; sync goes through `wp_linux_drm_syncobj` (wm side).

## Verifying on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

This part has no GPU code; on Midget it is verified through the window server once `layers-*` and `wm` are merged:

1. Build the server and SDK with the `fast` profile (`cargo build --profile fast`, per `docs/bifrost-pc-workflow.md`).
2. Start the window server with `BIFROST_DIRECT=1` and a test app that commits a transaction (for example a layer
   with a Basic opacity animation). `bifrost pc midget state` must show the context and the animation.
3. Send a hostile blob (a bad tag, written with a test client) and check that `commit` raises the `bad_transaction`
   protocol error and the window server does not crash.
4. Record `presented` and `animation_finished` events against the frame clock in `tools/bench.py --midget`.
