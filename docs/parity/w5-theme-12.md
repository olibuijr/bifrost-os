# Wave 5, theme 12: machine learning passes

## What was built

`springboard/ui/src/cnn_layers.rs` (pure Rust, std only, no unsafe, no hardware). It is an eager forward-pass
library that runs one layer at a time over a `Feature` (channels, height, width, f32 data):

- `conv2d`: 2D convolution with zero padding, stride and dilation. Window geometry (output size and padding) comes
  from the existing `nn::Window` and `nn::output_size`, so the shape rules match the graph code.
- `pool2d` with `Pooling::Max` or `Pooling::Average`. Padding taps are skipped (average counts only real pixels).
- `dense`: fully connected layer over the flattened input, and `relu`.
- `RecurrentWeights` with `Cell::Lstm` (gates input, forget, candidate, output) and `Cell::Gru` (gates update,
  reset, candidate). `lstm_step` and `gru_step` run one time step. The cell kind is checked.
- `CustomLayer` trait (a user-defined layer that states its output dims and evaluates a feature map), and
  `Layer::Custom(Box<dyn CustomLayer>)`.
- `Network`: layers run in order. `output_dims` checks every weight size and propagates shapes before anything runs.

Not built: training or backpropagation, a graph compiler or deferred execution, MPS-style image kernels, sequence
encode APIs over several time steps (only single steps), bidirectional stacks, serialised models, and any SDK
exposure. The module is not wired into apps or the shell and has not been built or run on the VM or Midget.
The gate orders and GRU equations are the textbook forms of the code, not a claim of match with any reference
framework's internal layout.

Rules used (the tests pin them): padding taps are excluded from pooling and from convolution sums; the average
pool divides by the count of real taps; a pooling window with no real tap is `EmptyWindow`; a recurrent cell built
for the other kind is `WrongCell`.

## Official API names mirrored

- `MLCustomLayer` (Core ML): the custom-layer contract. Our `CustomLayer` covers the output-shape and evaluate
  steps only. There is no `init(parameters:)`, no weight-data hook and no GPU encode step.
- `MPSCNNPoolingNode`, `MPSCNNConvolutionNode` and the other CNN layer nodes (Metal Performance Shaders): the
  convolution and pooling operators, with kernel, stride and padding. These are eager functions, not nodes in a
  graph.
- `MPSRNNMatrixInferenceLayer` (Metal Performance Shaders): recurrent inference. Only one LSTM or GRU step over one
  vector is built, not the matrix sequence encode calls.
- `BNNS` (Accelerate): neural network inference only. There is no training API and no BNNS descriptor type.

## REA evidence

- `MLCustomLayer`: `ev_15904441b94847b62ce48c8593230d6cc2b6955a41edab670d492efcb996fd37`
- `MPSCNNPoolingNode`: `ev_a2feb4d9d3cd3404414c2e557d521a9000cd12d31fb7b20405051a581e5653fd`
- `MPSRNNMatrixInferenceLayer`: `ev_62a5e2cb02b7a41a878e9e38280eeffa67693d978a4140c0999aec5bad998a31`
- `BNNS`: `ev_3a6ba2fd7c2a5aae6215d49d0955dd433371a231ee364395f45ce1e9f7f1a6c4`

## Matrix rows closed

These rows move to `partial`. The matrix itself is updated by the lead.

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| metal-performance-shaders | Neural network graphs and CNN layers | partial | springboard/ui/src/cnn_layers.rs (conv2d, pool2d, dense, Network) |
| metal-performance-shaders | Recurrent neural network layers | partial | springboard/ui/src/cnn_layers.rs (RecurrentWeights, lstm_step, gru_step) |
| core-ml | Custom layers and custom models | partial | springboard/ui/src/cnn_layers.rs (CustomLayer, Layer::Custom, Network) |
| accelerate | Neural networks | partial | springboard/ui/src/cnn_layers.rs (inference only; no training) |

Tests: `#[cfg(test)] mod tests` in `cnn_layers.rs` has ten hand-checked behaviour tests (conv valid and same
padding, kernel too large, max and average pooling including padded average, dense, LSTM zero-weight and
forget-gate order, GRU zero-weight and wrong-cell refusal, network chaining with dim checks, relu). No builds
were run in this round.
