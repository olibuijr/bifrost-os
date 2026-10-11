# Metal Performance Shaders Graph: parity notes

Official reference: https://developer.apple.com/documentation/metalperformanceshadersgraph (the framework landing
page lists the classes, structures, type aliases and enumerations this note refers to).

REA evidence:
- `ev_4b44e964d2efb4d1becd85318af6d3edd8dd657e78207622ea2d6798129380ee`: rendered landing page, captured for this pass.
- `ev_7f2f052886ef363942cce17fd7f1f69890b6852d3a7c6de79925c44969f4201f`: the earlier capture recorded in the matrix row.

Official API shape was read with `docc.py` from the DocC JSON for `metalperformanceshadersgraph`, `mpsgraphfftscalingmode`,
`mpsgraphpaddingmode`, `mpsgraphtype` and `mpsgraphconvolution2dopdescriptor`.

## What was built

All three models are pure Rust with std only, in the `bifrost-ui` crate. They do not touch a device, a buffer or the GPU.

- `springboard/ui/src/tensor.rs`: `DataType` (element sizes), `Shape` (element count, row-major strides, NumPy-style
  broadcasting, reshape rule), `TensorType` and `TensorData` (dense little-endian payload, byte length checked at
  construction, Float32 read-back). Mirrors `MPSGraphType`, `MPSGraphShapedType` and `MPSGraphTensorData`.
- `springboard/ui/src/graph.rs`: `Graph` records placeholders, constants, element-wise binary operators (with broadcast
  and matching element types), relu and reshape. Each node's type is inferred when it is added, so bad shapes fail at
  construction. `Graph::schedule(outputs)` returns the dependency-ordered list of nodes an output needs, each once.
  Mirrors `MPSGraph`, `MPSGraphTensor`, `MPSGraphOperation`, and the scheduling part of the compilation flow.
- `springboard/ui/src/nn.rs`: `Window` and `Padding` give the output length and the before and after pads of a
  convolution or pooling dimension (explicit, same and valid styles, dilation and stride), `output_size` does two
  dimensions, and `FftScaling` gives the none, size and unitary factors. Mirrors `MPSGraphConvolution2DOpDescriptor`,
  `MPSGraphPooling2DOpDescriptor`, and `MPSGraphFFTScalingMode` (size is 1/N, unitary is 1/sqrt(N) over all transformed
  dimensions, per the official enumeration).

Each module has unit tests for its behaviour (see the `tests` modules in the three files).

## Matrix rows changed

| Row | Status | Bifrost path |
|---|---|---|
| Graph construction | partial | `springboard/ui/src/graph.rs` |
| Tensor types and data | partial | `springboard/ui/src/tensor.rs` |
| Compilation and executables | partial | `springboard/ui/src/graph.rs` (`Graph::schedule`) |
| Convolution, pooling and RNN operators | partial | `springboard/ui/src/nn.rs` (shape rules only) |
| FFT and random operations | partial | `springboard/ui/src/nn.rs` (`FftScaling` only) |

The framework status moves from gap to partial.

## Remaining gaps

- Nothing runs. No operator computes values, and no transform, random number or RNN exists.
- No app-facing compute API in `springboard/sdk/src`. Apps cannot build or submit a graph, and no completion or
  scheduled callback exists for compute work. `springboard/src/gpu/` serves only the compositor.
- No compiled executable, optimization levels, compilation cache or serialized executable format.
- Graph control flow (for loop, if-then-else, while), variables, sparse, stencil and scaled dot-product attention
  descriptors are not modelled.
- Reduced precision fast math has no compute path to select.
- The Neural Engine has no Linux or BifrOSt counterpart. `springboard/src/gpu/plan.rs` answers the device-capability
  question internally, and apps cannot ask it.
