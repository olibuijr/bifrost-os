# Theme 09: Sharing buffers with the GPU framework

Scope: the buffer-sharing layer that sits between a mesh and a GPU queue. This wave adds two pure-Rust models
in `springboard/ui/src` (bifrost-ui). Neither owns GPU memory or touches hardware; a renderer maps the
layouts and commands onto its own buffers. No builds were run for this wave.

## What was built

- `springboard/ui/src/vertex_descriptor.rs` (`bifrost_ui::vertex_descriptor`): the vertex description
  behind a mesh. `ScalarKind` and `VertexFormat` (1 to 4 components, optional normalization, rejected for
  floats), `VertexAttribute` (name, format, offset, buffer slot), `VertexBufferLayout` (stride) and
  `VertexDescriptor`. The descriptor adds or replaces attributes by name, removes them, sets slot strides,
  packs offsets and strides per slot, computes the byte size of a slot for a vertex count, and validates that
  every attribute has a non-zero stride and ends inside it.
- `springboard/ui/src/blit.rs` (`bifrost_ui::blit`): a recorded `BlitPass` of `Fill` and `Copy` commands over
  numbered buffers. `validate` checks every command against buffer lengths without writing. `execute` validates
  the whole pass first, so a failing command leaves every buffer unchanged, then applies the commands in order.
  Copies inside one buffer are allowed when the ranges are disjoint; overlapping copies are refused.
- `springboard/ui/src/lib.rs`: one `pub mod` line for each new module.

The existing `springboard/ui/src/mesh.rs` (mesh, submesh, element buffer allocator) is unchanged and is the
layer these models sit under.

## Official APIs mirrored

Official pages (developer.apple.com DocC, read with `tools/parity/docc.py`):

- Model I/O: `MDLVertexDescriptor` (attributes, `attributeNamed(_:)`, `addOrReplaceAttribute(_:)`,
  `removeAttributeNamed(_:)`, `layouts`, `setPackedOffsets()`, `setPackedStrides()`, `reset()`),
  `MDLVertexAttribute` (`init(name:format:offset:bufferIndex:)`, `name`, `format`, `offset`, `bufferIndex`),
  `MDLVertexBufferLayout` (`init(stride:)`, `stride`), `MDLVertexFormat` (scalar, vector and normalized
  cases, used here as `ScalarKind` plus component count plus normalization flag).
  https://developer.apple.com/documentation/modelio/mdlvertexdescriptor
- Metal: `MTLBlitCommandEncoder` (`fill(buffer:range:value:)`, `copy(from:sourceOffset:to:destinationOffset:size:)`).
  https://developer.apple.com/documentation/metal/mtlblitcommandencoder

Deliberate differences: our `validate` refuses overlapping copies inside one buffer, because the official page
does not define the byte order for that case. The descriptor has no `reset()`; a new `VertexDescriptor::new()`
does the same. The model has no per-attribute `initializationValue` and no `MDLVertexAttributeData` pointer
view; both need the data-pointer side, which is out of scope for a pure model.

## REA evidence

- Model I/O landing page: `ev_247febfb8bf4997d6cbf9b12676fbbc995b5ee2c5cad01ffc9974e719f7ae84d`
  (lists MDLVertexDescriptor, MDLVertexAttribute, MDLVertexBufferLayout and the buffer protocols).
- MTLBlitCommandEncoder page: `ev_be4a91768369437ba2a2909408d31172eff04ff0348ab5d6d1c229778511fde8`
  (fill and copy signatures under "Filling buffers" and "Copying buffer data to another buffer").

## Matrix rows closed

| framework slug | feature (as in data/<slug>.json) | new status | BifrOSt path |
|---|---|---|---|
| model-i-o | Mesh and vertex buffers | partial | springboard/ui/src/vertex_descriptor.rs (with the existing springboard/ui/src/mesh.rs) |
| metal | Blit passes and copies | partial | springboard/ui/src/blit.rs |

Still open in these rows: no app-facing GPU buffer object, no encoder that records into a real command queue,
and no zero-copy binding to the presenter (`springboard/src/gpu/`). The "Mesh buffer allocation and mapping"
row (model-i-o) stays a gap; this wave does not add zones or mapping.

## Tests

`cargo` was not run (build-free wave). The modules carry `#[cfg(test)]` suites: 6 in `vertex_descriptor.rs`
and 7 in `blit.rs`, covering packing, validation errors, atomic failure, overlap refusal and missing buffers.
`python3 tools/check-names.py` exits 0.
