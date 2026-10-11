# Core ML parity

Framework: Core ML (https://developer.apple.com/documentation/coreml), macOS 10.13 and later.
Status after this pass: **partial**. The data types a model would use exist in `bifrost-ui`; running a model does not.

## What was built

All three modules are pure Rust with std only, live in `springboard/ui/src` (bifrost-ui), and have unit tests.

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/tensor.rs` | `MLTensor`, `MLTensorRangeExpression`, `pointwiseMin`, `pointwiseMax` | A row-major `f32` tensor with a shape. Reshape, slicing along one axis, pointwise min and max on equal shapes. Shape and count errors are typed (`TensorError`). |
| `springboard/ui/src/features.rs` | `MLFeatureValue`, `MLFeatureProvider`, `MLDictionaryFeatureProvider`, `MLBatchProvider`, `MLArrayBatchProvider` | Named values of kind `Int`, `Double`, `Text` or `Tensor`. A dictionary provider with sorted names. `check` refuses a provider that lacks a declared input or holds the wrong kind, and ignores extra names. An array batch of providers. |
| `springboard/ui/src/compute.rs` | `MLComputeDevice`, `MLCPUComputeDevice`, `MLGPUComputeDevice`, `MLComputePolicy` | `Device` (CPU, GPU), `Policy` (automatic: GPU first with CPU fallback; CPU only), and `place`, which returns the first allowed device an operation supports. |

Each module has a `#[cfg(test)] mod tests` with behaviour tests. `springboard/ui/src/lib.rs` gained three `pub mod` lines.
No builds or tests were run in this pass; the build stage is responsible for them.

## Evidence

- Official overview: `https://developer.apple.com/documentation/coreml`, read with `tools/parity/docc.py coreml` and rendered through REA as `ev_243d4d7b74a7269eeb3bb25ea72805f637b0b17019fa9a4ace0b163bb9dfc9e0`.
- The overview says the framework runs on the CPU, GPU and Neural Engine and that it supports Vision, Natural Language, Speech and Sound Analysis. It also says models are created with Create ML and Core ML Tools. Those are Apple developer tools and are marked `n/a`.
- The earlier matrix evidence `ev_61bb5baae834f9b3d498207c8aefac9606a9eefacd2c879fbc0c612a09006428` is kept in the matrix history.

## Remaining gaps

- No SDK API (`springboard/sdk/src`) exposes the models. Nothing loads a model file or runs a prediction (MLModel, MLModelAsset, MLModelError).
- No model description. `features::check` takes its declared inputs from the caller, not from a model file.
- Tensors have no broadcasting, arithmetic, Boolean or integer scalars, and no matrix operations.
- Compute placement decides a device but runs nothing on one. There is no Neural Engine on this platform.
- Stateful models (MLState), on-device updates (MLUpdateTask), runtime download and compile (MLModelCollection), model encryption, and custom layers are all missing.
- Domain APIs built on inference (image classification, speech, sound analysis, natural language) are missing. The keyboard predictor (`springboard/apps/keyboard/src/predict.rs`) is a trie with bigram counts, not a learned model.
