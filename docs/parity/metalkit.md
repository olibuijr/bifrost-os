# MetalKit parity: pure texture and mesh models

Status: partial (was gap). Matrix row: `docs/parity-matrix/data/metalkit.json`.
Official reference: https://developer.apple.com/documentation/metalkit

## What was built

Two device-free modules in `springboard/render` (`bifrost-render`), std only, no unsafe, no new crates:

- `springboard/render/src/texture.rs`: the pure half of an image loader.
  - `LoadOptions` mirrors the texture loader option keys: sRGB, allocate mipmaps, generate mipmaps,
    origin (`Origin`), usage (`Usage`), CPU cache mode (`CacheMode`) and storage (`Storage`).
  - `TextureDescriptor::new` validates the size (zero and sides past `MAX_SIDE` are refused) and sets the level count.
  - `mip_count`, `mip_layout` and `TextureDescriptor::mip_chain` give the packed byte layout of the chain.
  - `generate_mips` box-filters the base to 1x1 on premultiplied pixels, rounding to nearest.
  - `apply_origin` reverses the rows for a bottom-left origin. Decoded images are stored top row first.
  - `prepare` and `load_bytes` combine the above with `imgdec::decode` into a `Prepared` value for the caller to upload.
- `springboard/render/src/mesh.rs`: the data half of model handling.
  - `VertexLayout` validates attribute names, buffer indices, stride fit and overlap.
  - `BufferAllocator` hands out `Buffer` values under a byte budget and takes bytes back on `release`.
  - `Submesh` pairs a `Primitive` and `IndexType` with an index buffer. Its count must make whole primitives.
  - `Mesh::new` checks buffer sizes and that every index names an existing vertex. `Mesh::position` and `Mesh::bounds`
    read the `position` attribute (three floats) into an `Aabb`.

Tests: 8 in `texture.rs` and 7 in `mesh.rs`. They cover mip counts, descriptor limits, mip layout offsets,
averaging, origin flips, PNG decoding and rejection, layout validation, index checks, bounds and the budget.
Nothing was built or run in this round, as the round's rules require.

## Official API names mirrored

- `MTKTextureLoader`, `MTKTextureLoader.Option` (`allocateMipmaps`, `generateMipmaps`, `textureUsage`,
  `textureCPUCacheMode`, `textureStorageMode`, `origin`, `SRGB`). Sources: https://developer.apple.com/documentation/metalkit/mtktextureloader
  and https://developer.apple.com/documentation/metalkit/mtktextureloader/option
- `MTKMesh` (`vertexBuffers`, `vertexCount`, `vertexDescriptor`, `submeshes`), `MTKSubmesh` (`indexBuffer`,
  `indexCount`, `indexType`, `primitiveType`), `MTKMeshBufferAllocator`. Sources: https://developer.apple.com/documentation/metalkit/mtkmesh
  and https://developer.apple.com/documentation/metalkit/mtksubmesh and https://developer.apple.com/documentation/metalkit/mtkmeshbufferallocator
- The type-level names are kept in shape only. Code names are generic (`LoadOptions`, `Mesh`, `Submesh`, `BufferAllocator`)
  because the owner rule bars platform names from code.

## REA evidence

- `ev_66624be43980f5d880343c0ef4b41cc7ae2599db80e78cbc1cf065b3850e9ae5`: rendered MetalKit landing page
  (`https://developer.apple.com/documentation/metalkit`), with the View Management, Texture Loading, Model Handling,
  Conversion Functions, Model Errors and Reference topics.
- `ev_6daab09477ece3392a006798a03c8f877cc58a29a34b3ff5363ab2d632b075ca`: the evidence recorded in the matrix before this round.

DocC reads (`tools/parity/docc.py`) of `metalkit`, `metalkit/mtktextureloader`, `metalkit/mtktextureloader/option`,
`metalkit/mtkmesh`, `metalkit/mtksubmesh`, `metalkit/mtkmeshbufferallocator` and `modelio/mdlmesh` supplied the API shape.

## Remaining gaps

- No GPU texture object. `Prepared` is not uploaded to the presenter; `springboard/src/gpu/` has no texture-from-prepared path.
- No asynchronous load, no asset catalog lookup, no cube layouts, no display gamut.
- No device-backed mesh buffers. `Buffer` owns CPU bytes only.
- No mesh loading from files, no normal, tangent or subdivision generation, no attribute add or replace.
- The GPU view with a per-frame draw delegate (MTKView equivalent) is still missing from `springboard/sdk/src/`.
- Generating mipmaps implies allocation in this model. Whether the reference loader does the same is not stated in the
  official pages read here, so that choice is ours and should be checked against the reference behaviour.
