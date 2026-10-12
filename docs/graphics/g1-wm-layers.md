# G1 wm-layers: the layer context store and the bifrost_layer_v1 request semantics

Owner: wave G1 agent `wm-layers`. Contract: `docs/graphics-architecture.md`, "Window server" (`render_server/layers.rs`)
and "bifrost_layer_v1".

## What is implemented

`springboard/wm/src/render_server/layers.rs` (`ContextStore`, Smithay-free):

- `create(ctx)`: `get_context`. Returns false for a context that exists (the glue posts `already_has_context`).
- `remove(ctx)`: the context is destroyed with its surface.
- `bind(ctx, contents, Some(key) | None)` and `damage(ctx, contents, rect)`: queued as pending. They take effect only
  with the next applied commit, as the protocol says.
- `commit(ctx, tx)`: queues a decoded `Transaction`. An unknown context is created.
- `apply_pending(now)`: per context, in id order. The batch is applied to a clone of the tree. On success the pending
  bindings (insert or remove), damage and the tree are committed together, and the seqs are recorded in `applied`.
  On the first failing transaction the whole batch of that context is discarded (bindings and damage too), the old
  tree is kept, and `(ctx, ApplyError)` is returned. Other contexts still apply.
- `presentation(ctx, t)`, `take_finished(ctx, t)`, `animating(t)`: delegate to `Tree` (empty for unknown contexts).
- `binding(ctx, contents)`: the import key bound to a contents id.

`springboard/wm/src/render_server/layers_dispatch.rs` (protocol semantics on plain values):

- `get_context`, `bind_contents`, `unbind_contents`, `damage_contents` (zero or negative area is dropped), `commit`
  (decode with `codec::decode`; a bad blob is `BadTransaction` and nothing is queued).
- `RequestError::{AlreadyHasContext, BadTransaction, BadBuffer}` with `code()` returning the protocol values from
  `bifrost-layer-v1.xml` (already_has_context = 0, bad_transaction = 0, bad_buffer = 1).

`render_server/mod.rs` gained `pub mod layers_dispatch;`.

## Contract changes

- `Context` gained `pending_damage` and `damage` (the contract says `damage_contents` applies with the next commit, and
  compose needs it). `ContextStore` gained the methods listed above. The skeleton signatures are unchanged.

## Not done (todo_left)

- The Wayland `Dispatch` impls and global creation for `bifrost_layer_manager_v1` and `bifrost_layer_context_v1`, and
  sending `presented` and `animation_finished`. `bifrost_layers::protocol` (layers-protocol) is still an empty module on
  this branch, so there are no generated server types to dispatch on. The glue functions above are what those impls
  call. `bind_contents` maps to `import` (the `wl_buffer` becomes a key; an import failure is `RequestError::BadBuffer`).
- Relies on `Tree::apply`, `presentation`, `take_finished` and `animating` (layers-model and layers-animation) and on
  `codec::decode` (layers-codec). Those are `todo!()` on this branch, so the tests that reach them are `#[ignore]`d
  with a reason. Remove the ignores once those land.
- Nothing calls `apply_pending` yet. The frame loop must call it at frame start and send `presented` after the flip.

## Tests

Pure tests in `layers.rs` and `layers_dispatch.rs` cover create/remove, queue order, bind/unbind and damage queueing,
unknown-context reads, and the no-commit case. The ignored tests cover batch apply order, atomic bindings, and a failing
context keeping its old state while another applies. Expected values were worked out by hand. `cargo check -p bifrost-wm
--tests` passes. The tests were not run, because `cargo test` is not allowed for this worktree.

## Verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

After the layers-model, layers-codec and layers-protocol parts and the Dispatch impls land:

1. `cargo test -p bifrost-wm render_server::layers` on the host (un-ignore the codec- and tree-backed tests).
2. Run the window server with `BIFROST_WM_RENDERER=vulkan`, then start an SDK app. Its `get_context` and `commit` must
   succeed, a bad blob must disconnect it with `bad_transaction`, and a second `get_context` on the same surface must
   give `already_has_context`.
3. `bifrost pc midget` screenshot of an open-animation layer. It should advance without app redraws.
