# Wave 5: Accelerate gap rows

Wave 5 builds two Accelerate features in pure Rust, std only, with no new crates and no unsafe code. Both were
`gap` or `os` rows in `docs/parity-matrix/data/accelerate.json`. The matrix itself is updated centrally by the lead.

## What was built

### Neural networks (BNNS), row was `gap`

- `springboard/ui/src/nn.rs` (SDK path `bifrost_app::controls::nn`) gains a fully connected inference path next to the
  existing shape rules:
  - `Activation` (identity, ReLU, leaky ReLU, sigmoid, tanh) with `eval`.
  - `DenseLayer::new(inputs, outputs, weights, bias, activation)`, `forward(&input)`, `inputs()`, `outputs()`. Weights
    are row-major, one row per output. Shapes are checked at construction and on every call (`DenseError`).
  - `softmax` (max-shifted, so large scores do not overflow) and `argmax` (first maximum wins, NaN skipped).
- Mirrors the official `BNNS.FullyConnectedLayer` and `BNNS.ActivationLayer` layer objects and their activation
  functions, as a CPU `f32` forward pass only.
- Not built: convolution, pooling, normalisation, training and optimisers, sparse layers, and any GPU or Neural Engine
  path. Those stay `gap`.

### Pixel format and colour conversion, row was `os`

- `springboard/render/src/pixel_layout.rs` gains byte-buffer conversions that were missing:
  - `cmyk_to_rgb_u8`: 8-bit CMYK to RGB, `(255 - ink) * (255 - k) / 255` per channel. No ICC profile.
  - `gray_to_rgb_u8` and `rgb_to_gray_u8` (Rec. 601 luma in integers, `(77 r + 150 g + 29 b + 128) / 256`).
  - `deinterleave_u8` and `interleave_u8` for planar and interleaved 8-bit buffers.
- `springboard/sdk/src/lib.rs` re-exports the module as `bifrost_app::kit::pixel_layout`, which gives apps the
  conversion API (the `os` row needed an app-facing API over the existing renderer code).
- The pre-existing `springboard/render/src/pixel_format.rs` (format descriptions, layout only) is unchanged.
- Not built: 16-bit and float conversions, YUV paths beyond the existing video code, and any colour-managed
  conversion. Those stay `partial` in the matrix.

## Official API references

- BNNS: https://developer.apple.com/documentation/accelerate/bnns
  (`BNNS.FullyConnectedLayer`, `BNNS.ActivationLayer`, `BNNS.ActivationFunction`)
- vImage: https://developer.apple.com/documentation/accelerate/vimage
  (`vImage.Planar8x4` for CMYK and RGBA planes, `vImage.Interleaved8x3`, `vImage.Planar8`)

## REA evidence

- BNNS landing page: `ev_5750a6472be06ea37e31b051355003bbf4dcfe4b082667bb90ff617aed1dcbd7`
- vImage landing page: `ev_d8790f9d165e57ff5c30fbbeae1007fd8c799679722e1c6316470bde1fdcf4d4`

## Matrix rows closed

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| accelerate | Neural networks | partial | springboard/ui/src/nn.rs |
| accelerate | Pixel format and colour conversion | partial | springboard/render/src/pixel_layout.rs, springboard/sdk/src/lib.rs (kit::pixel_layout) |

## Not done in this wave

- Audio DSP (biquad filters, spectrogram) stays `os`. The only biquad is `Biquad` in the DAW app's own `dsp` module,
  and moving it into a shared crate would duplicate it or touch the DAW, so it is left for a later change.
- Core Video pixel buffer interop stays `os`.

## Tests

Hand-worked behaviour tests were added to the existing `#[cfg(test)]` modules:

- `springboard/ui/src/nn.rs`: activation values, a 2x2 dense layer (`[3.5, 6.0]`), ReLU clipping (`[0.0, 7.0]`),
  shape refusals, softmax sums and overflow safety, argmax ties and NaN.
- `springboard/render/src/pixel_layout.rs`: CMYK corners and half values (`[127, 255, 255]`), grey and luma
  (`[77, 149, 29, 255]`), plane split and join with mismatch refusals.

No build or test was run in this wave, per the wave brief.
