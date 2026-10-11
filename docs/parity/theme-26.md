# Theme 26: graph construction, tensor types, compilation

Cross-framework theme: symbolic compute graphs (the reference Metal Performance Shaders Graph API and its tensor,
operation, type, descriptor and executable classes).

## What was built

Extensions to the existing pure-Rust graph and tensor models in `bifrost_ui`. No new modules.

- `springboard/ui/src/graph.rs`
  - New operators beside the existing binary, relu and reshape: `Graph::unary` (`Unary::Negate`, `Absolute`,
    `Square`, and the float-only `Sqrt`, `Exponential`), `Graph::cast`, `Graph::transpose` (permutation checked),
    `Graph::matmul` (batched over the last two dimensions, leading dimensions broadcast), `Graph::concat` (axis join
    with every other dimension checked).
  - `Options { reduced_precision_fast_math }` on the graph (`options` / `set_options`).
  - `Graph::compile(outputs)` returns an `Executable`: the schedule (`order`), the placeholders it needs with their
    types (`feeds`), the output types (`outputs`) and the fast-math flag. `Executable::check_feeds` refuses a missing
    feed, a feed with the wrong type, and a feed that names a non-placeholder.
  - New `GraphError` variants: `Unsupported`, `MissingFeed`, `NotAFeed`, `FeedType`.
- `springboard/ui/src/tensor.rs`: `DataType::is_float`; new `ShapeError` variants `Rank`, `Axis`, `Permutation`, `Empty`.
- Tests: 7 new behaviour tests in `graph.rs` (matmul broadcast and refusal, transpose, concat, cast, unary element
  types, compile signature and option, feed checking). The existing tests are unchanged.

Nothing is executed: the graph records and types operators, and the executable describes what a run needs. The
fast-math flag is recorded, not applied.

## Official API names mirrored

Source: developer.apple.com DocC index (`python3 -I tools/parity/docc.py metalperformanceshadersgraph`), which lists
MPSGraph, MPSGraphTensor (`dataType`, `shape`, `operation`), MPSGraphOperation, MPSGraphType, MPSGraphShapedType,
MPSGraphTensorData, MPSGraphCompilationDescriptor (`reducedPrecisionFastMath`), MPSGraphExecutable and
MPSGraphReducedPrecisionFastMath.

REA evidence (rendered official landing page, one run): `ev_f681e5b6e368c00d82b94e5d3e710cfb5313ff54359070fcef2d3740d899c9bf`.

Not mirrored: serialization (`MPSGraphExecutableSerializationDescriptor`), optimization levels and profiles,
compilation completion handlers, execution descriptors, dynamic (unknown-size) shapes, and any operator beyond the
set above (convolution, pooling, RNN, FFT, random).

## Matrix rows closed

Status is `partial` for all four: the construction and typing side is covered, but nothing runs and the Apple
operator catalogue is far from complete.

| Framework slug | Feature (as written in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| metal-performance-shaders-graph | Graph construction (MPSGraph, MPSGraphTensor, MPSGraphOperation) | partial | springboard/ui/src/graph.rs (`Graph`, `TensorId`, `Kind`) |
| metal-performance-shaders-graph | Tensor types and data (MPSGraphType, MPSGraphShapedType, MPSGraphTensorData) | partial | springboard/ui/src/tensor.rs (`TensorType`, `Shape`, `TensorData`) |
| metal-performance-shaders-graph | Fast math and reduced precision (MPSGraphReducedPrecisionFastMath) | partial | springboard/ui/src/graph.rs (`Options`, `Executable::reduced_precision_fast_math`) |
| metal-performance-shaders-graph | Compilation and executables (MPSGraphCompilationDescriptor, MPSGraphExecutable, serialization) | partial | springboard/ui/src/graph.rs (`Graph::compile`, `Executable`, `check_feeds`) |

Rows in the theme that stay open: the convolution, pooling and RNN operator row, the FFT and random row, and the
`metal` row for machine learning passes. The `metal-performance-shaders` rows are outside this capability.

## Verification

Builds and tests were not run (per the theme rules). `python3 tools/check-names.py` exits 0.
