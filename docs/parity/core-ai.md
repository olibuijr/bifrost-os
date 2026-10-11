# Core AI parity: typed arrays and inference signatures

Framework: Core AI (macOS 27). Official landing page: https://developer.apple.com/documentation/coreai

This round ports the data side of Core AI to pure Rust with the standard library only. It adds no model runtime, no
compute stream and no model cache. Those rows stay `gap`.

## What was built

| Module | What it mirrors | Notes |
|---|---|---|
| `springboard/sdk/src/ndarray.rs` | `NDArray`, `NDArray.ScalarType`, `NDArrayDescriptor` | Row-major packed storage. `preferred_strides`, `minimum_byte_count`, `offset` with bounds checks, `get`, `set`, `reshaped`. Scalars: Int8, Int16, Int32, UInt8, Float32, Float64, through the `Scalar` trait. |
| `springboard/sdk/src/inference.rs` | `InferenceFunctionDescriptor`, `InferenceValue`, `ImageDescriptor` | Named inputs and outputs with unique names. `check_inputs` rejects a missing, unknown, repeated or mismatched input before anything runs. `ImageDescriptor::to_ndarray` maps packed Gray8, Rgb8 or Rgba8 bytes to a float32 array in `[height, width, channels]` order with `b * scale + bias`. |

Both modules are declared in `springboard/sdk/src/lib.rs` with one `pub mod` line each. Each has a `tests` module
with behaviour tests for strides, offsets, bounds, count checks, reshape, signature checks, duplicate names and image
normalisation.

## Official API names mirrored

- `NDArray`: `init(shape:scalarType:)`, `init(scalars:shape:)`, `shape`, `scalarType`, `strides`
- `NDArrayDescriptor`: `shape`, `scalarType`, `rank`, `minimumByteCount`, `preferredStrides`
- `InferenceFunctionDescriptor`: `name`, `inputCount`, `inputNames`, `inputDescriptor(of:)`, `outputCount`,
  `outputNames`, `outputDescriptor(of:)`
- `InferenceValue`: `ndArray`, `pixelBuffer`, the `init(_:)` for a pixel buffer, `descriptor`
- `ImageDescriptor`: `width`, `height`, `pixelFormatType` (as `PixelFormat`, without the four-character code)

Sources read: `docc.py coreai`, `coreai/ndarraydescriptor`, `coreai/ndarray`,
`coreai/inferencefunctiondescriptor`, `coreai/inferencevalue`, `coreai/imagedescriptor` (official DocC JSON, via
`tools/parity/docc.py`).

## REA evidence

- `ev_372a521dd406a876e0a1dd90a743919e1e501f6a757c89507f1e3dc53f7f5ce0`: rendered Core AI landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/coreai`). This id replaces
  `ev_d5a497b99b8bee740ff9134aa7978e7596ef3614efb76bbb2ec57ef802f765e0` in the matrix row.

## Status changes

| Row | Before | After |
|---|---|---|
| Inference functions and values | gap | partial |
| Multidimensional arrays | gap | partial |
| Image input descriptors | partial | partial (now with a conversion path) |
| Framework | gap | partial |

## Remaining gaps

- No on-device model runtime: nothing loads a model, specializes it or runs `InferenceFunction`.
- `NDArray` has no `Float16` or `Int64` scalars, no dynamic dimensions, no interleaved layout, and no typed or raw
  memory views (`view(as:)`, `mutableView(as:)`, `rawView()`).
- `InferenceValue` holds float32 arrays only. Other scalar types cannot be passed as values.
- No state descriptors (`stateNames`, `stateDescriptor(of:)`).
- No `ComputeStream`, no `AIModelCache`, no ahead-of-time compile step, no inference tracing in the debug tools.
- `ImageDescriptor` has no four-character pixel format code, and nothing connects its output to a model.

## Verification

No build or test was run in this round, as the task required. The modules are std-only, with no `unsafe`, no new
crates and no hardware or network dependency. `python3 tools/check-names.py` reports no hits.
