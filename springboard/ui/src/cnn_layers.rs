//! Eager layers for small neural networks: 2D convolution, 2D pooling, a dense layer, relu, LSTM and GRU cells,
//! and a hook for user-defined layers. A `Network` chains them over a `Feature` (channels, height, width, f32 data).
//! Window geometry (output sizes and padding) comes from `nn`, so the same shape rules apply here and to graphs.
//! This runs each layer as it is called. It has no graph compiler, no GPU path and no serialised model format.

use crate::nn::{output_size, Window, WindowError};

/// Why a layer cannot run on its input or its weights are the wrong size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerError {
    /// A buffer or weight array has `got` values where the layer needs `expected`.
    Length { expected: usize, got: usize },
    Window(WindowError),
    /// A pooling window covers only padding, so there is no value to take.
    EmptyWindow,
    /// A recurrent step was called on weights built for the other cell kind.
    WrongCell,
    /// A custom layer rejected its input.
    Custom(&'static str),
}

impl From<WindowError> for LayerError {
    fn from(e: WindowError) -> Self {
        LayerError::Window(e)
    }
}

/// Channels, height and width of a feature map, stored channel by channel, then row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub channels: usize,
    pub height: usize,
    pub width: usize,
    pub data: Vec<f32>,
}

impl Feature {
    pub fn new(channels: usize, height: usize, width: usize, data: Vec<f32>) -> Result<Self, LayerError> {
        let expected = channels * height * width;
        if data.len() != expected {
            return Err(LayerError::Length { expected, got: data.len() });
        }
        Ok(Feature { channels, height, width, data })
    }

    pub fn dims(&self) -> [usize; 3] {
        [self.channels, self.height, self.width]
    }

    fn at(&self, c: usize, y: usize, x: usize) -> f32 {
        self.data[(c * self.height + y) * self.width + x]
    }
}

/// Position of kernel tap `k` for output index `o`, in input pixels. `None` when the tap falls in the padding.
fn tap(o: usize, k: usize, window: Window, pad_before: usize, input: usize) -> Option<usize> {
    let pos = (o * window.stride + k * window.dilation) as isize - pad_before as isize;
    if pos < 0 || pos >= input as isize {
        None
    } else {
        Some(pos as usize)
    }
}

/// Two-dimensional convolution with zero padding. `weights` is laid out `[out][in][kernel_h][kernel_w]`, and
/// `windows` is `[height, width]`, each giving the kernel size, stride, dilation and padding of that axis.
pub fn conv2d(
    input: &Feature,
    weights: &[f32],
    bias: &[f32],
    out_channels: usize,
    windows: [Window; 2],
) -> Result<Feature, LayerError> {
    let [oh, ow] = output_size([input.height, input.width], windows)?;
    let (kh, kw) = (windows[0].kernel, windows[1].kernel);
    let want = out_channels * input.channels * kh * kw;
    if weights.len() != want {
        return Err(LayerError::Length { expected: want, got: weights.len() });
    }
    if bias.len() != out_channels {
        return Err(LayerError::Length { expected: out_channels, got: bias.len() });
    }
    let (top, _) = windows[0].pads(input.height)?;
    let (left, _) = windows[1].pads(input.width)?;
    let mut out = Vec::with_capacity(out_channels * oh * ow);
    for oc in 0..out_channels {
        for oy in 0..oh {
            for ox in 0..ow {
                let mut acc = bias[oc];
                for ic in 0..input.channels {
                    for ky in 0..kh {
                        let Some(iy) = tap(oy, ky, windows[0], top, input.height) else { continue };
                        for kx in 0..kw {
                            let Some(ix) = tap(ox, kx, windows[1], left, input.width) else { continue };
                            let w = weights[((oc * input.channels + ic) * kh + ky) * kw + kx];
                            acc += w * input.at(ic, iy, ix);
                        }
                    }
                }
                out.push(acc);
            }
        }
    }
    Feature::new(out_channels, oh, ow, out)
}

/// The reduction a pooling window applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pooling {
    Max,
    /// The mean of the taps that land on the input. Padding is left out of the count.
    Average,
}

/// Pools each channel on its own. Padding taps are skipped, so a max never sees a padded zero.
pub fn pool2d(input: &Feature, kind: Pooling, windows: [Window; 2]) -> Result<Feature, LayerError> {
    let [oh, ow] = output_size([input.height, input.width], windows)?;
    let (top, _) = windows[0].pads(input.height)?;
    let (left, _) = windows[1].pads(input.width)?;
    let mut out = Vec::with_capacity(input.channels * oh * ow);
    for c in 0..input.channels {
        for oy in 0..oh {
            for ox in 0..ow {
                let mut best = f32::NEG_INFINITY;
                let mut sum = 0.0f32;
                let mut count = 0usize;
                for ky in 0..windows[0].kernel {
                    let Some(iy) = tap(oy, ky, windows[0], top, input.height) else { continue };
                    for kx in 0..windows[1].kernel {
                        let Some(ix) = tap(ox, kx, windows[1], left, input.width) else { continue };
                        let v = input.at(c, iy, ix);
                        best = best.max(v);
                        sum += v;
                        count += 1;
                    }
                }
                if count == 0 {
                    return Err(LayerError::EmptyWindow);
                }
                out.push(match kind {
                    Pooling::Max => best,
                    Pooling::Average => sum / count as f32,
                });
            }
        }
    }
    Feature::new(input.channels, oh, ow, out)
}

/// Fully connected layer over the flattened input. `weights` is `[outputs][input length]`. The result is
/// `outputs` channels of one pixel each.
pub fn dense(input: &Feature, weights: &[f32], bias: &[f32], outputs: usize) -> Result<Feature, LayerError> {
    let n = input.data.len();
    if weights.len() != outputs * n {
        return Err(LayerError::Length { expected: outputs * n, got: weights.len() });
    }
    if bias.len() != outputs {
        return Err(LayerError::Length { expected: outputs, got: bias.len() });
    }
    let out: Vec<f32> = (0..outputs)
        .map(|o| {
            let row = &weights[o * n..(o + 1) * n];
            bias[o] + row.iter().zip(&input.data).map(|(w, x)| w * x).sum::<f32>()
        })
        .collect();
    Feature::new(outputs, 1, 1, out)
}

/// Clamps negative values to zero. The shape is unchanged.
pub fn relu(input: &Feature) -> Feature {
    Feature {
        channels: input.channels,
        height: input.height,
        width: input.width,
        data: input.data.iter().map(|v| v.max(0.0)).collect(),
    }
}

/// Which recurrent cell a set of weights belongs to. The gate count is fixed by the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Four gates in the order input, forget, candidate, output.
    Lstm,
    /// Three gates in the order update, reset, candidate.
    Gru,
}

impl Cell {
    pub fn gates(self) -> usize {
        match self {
            Cell::Lstm => 4,
            Cell::Gru => 3,
        }
    }
}

/// Weights of one recurrent layer. `input_weights` is `[gates*hidden][input_len]`, `recurrent_weights` is
/// `[gates*hidden][hidden]`, and `bias` is `[gates*hidden]`, with the gates stacked in the order `Cell` lists.
#[derive(Clone, Debug, PartialEq)]
pub struct RecurrentWeights {
    pub cell: Cell,
    pub hidden: usize,
    pub input_len: usize,
    pub input_weights: Vec<f32>,
    pub recurrent_weights: Vec<f32>,
    pub bias: Vec<f32>,
}

impl RecurrentWeights {
    pub fn new(
        cell: Cell,
        hidden: usize,
        input_len: usize,
        input_weights: Vec<f32>,
        recurrent_weights: Vec<f32>,
        bias: Vec<f32>,
    ) -> Result<Self, LayerError> {
        let rows = cell.gates() * hidden;
        for (got, expected) in [
            (input_weights.len(), rows * input_len),
            (recurrent_weights.len(), rows * hidden),
            (bias.len(), rows),
        ] {
            if got != expected {
                return Err(LayerError::Length { expected, got });
            }
        }
        Ok(RecurrentWeights { cell, hidden, input_len, input_weights, recurrent_weights, bias })
    }

    /// The input and recurrent halves of every gate's pre-activation. The bias goes with the input half.
    fn pre_activations(&self, x: &[f32], h: &[f32]) -> Result<(Vec<f32>, Vec<f32>), LayerError> {
        if x.len() != self.input_len {
            return Err(LayerError::Length { expected: self.input_len, got: x.len() });
        }
        if h.len() != self.hidden {
            return Err(LayerError::Length { expected: self.hidden, got: h.len() });
        }
        let rows = self.cell.gates() * self.hidden;
        let mut from_x = Vec::with_capacity(rows);
        let mut from_h = Vec::with_capacity(rows);
        for r in 0..rows {
            let wx = &self.input_weights[r * self.input_len..(r + 1) * self.input_len];
            let wh = &self.recurrent_weights[r * self.hidden..(r + 1) * self.hidden];
            from_x.push(self.bias[r] + wx.iter().zip(x).map(|(w, v)| w * v).sum::<f32>());
            from_h.push(wh.iter().zip(h).map(|(w, v)| w * v).sum::<f32>());
        }
        Ok((from_x, from_h))
    }
}

fn sigmoid(v: f32) -> f32 {
    1.0 / (1.0 + (-v).exp())
}

/// One LSTM step. Returns the new hidden state and the new cell state:
/// `c' = f * c + i * g` and `h' = o * tanh(c')`.
pub fn lstm_step(w: &RecurrentWeights, x: &[f32], h: &[f32], c: &[f32]) -> Result<(Vec<f32>, Vec<f32>), LayerError> {
    if w.cell != Cell::Lstm {
        return Err(LayerError::WrongCell);
    }
    if c.len() != w.hidden {
        return Err(LayerError::Length { expected: w.hidden, got: c.len() });
    }
    let (from_x, from_h) = w.pre_activations(x, h)?;
    let n = w.hidden;
    let mut h_new = Vec::with_capacity(n);
    let mut c_new = Vec::with_capacity(n);
    for k in 0..n {
        let pre = |gate: usize| from_x[gate * n + k] + from_h[gate * n + k];
        let i = sigmoid(pre(0));
        let f = sigmoid(pre(1));
        let g = pre(2).tanh();
        let o = sigmoid(pre(3));
        let cell = f * c[k] + i * g;
        c_new.push(cell);
        h_new.push(o * cell.tanh());
    }
    Ok((h_new, c_new))
}

/// One GRU step: `z` and `r` gate the recurrent term, the candidate `n` uses `r` times the recurrent product, and
/// `h' = (1 - z) * n + z * h`.
pub fn gru_step(w: &RecurrentWeights, x: &[f32], h: &[f32]) -> Result<Vec<f32>, LayerError> {
    if w.cell != Cell::Gru {
        return Err(LayerError::WrongCell);
    }
    let (from_x, from_h) = w.pre_activations(x, h)?;
    let n = w.hidden;
    Ok((0..n)
        .map(|k| {
            let z = sigmoid(from_x[k] + from_h[k]);
            let r = sigmoid(from_x[n + k] + from_h[n + k]);
            let cand = (from_x[2 * n + k] + r * from_h[2 * n + k]).tanh();
            (1.0 - z) * cand + z * h[k]
        })
        .collect())
}

/// A user-defined layer. It states its output shape from its input shape and runs on a whole feature map.
pub trait CustomLayer {
    fn name(&self) -> &str;
    fn output_dims(&self, input: [usize; 3]) -> Result<[usize; 3], LayerError>;
    fn evaluate(&self, input: &Feature) -> Result<Feature, LayerError>;
}

/// One step of a network.
pub enum Layer {
    Conv2d { out_channels: usize, weights: Vec<f32>, bias: Vec<f32>, windows: [Window; 2] },
    Pool { kind: Pooling, windows: [Window; 2] },
    Dense { outputs: usize, weights: Vec<f32>, bias: Vec<f32> },
    Relu,
    Custom(Box<dyn CustomLayer>),
}

impl Layer {
    /// The shape this layer gives an input of `input` dims, checking the weight sizes on the way.
    pub fn output_dims(&self, input: [usize; 3]) -> Result<[usize; 3], LayerError> {
        let [c, h, w] = input;
        match self {
            Layer::Conv2d { out_channels, weights, bias, windows } => {
                let want = out_channels * c * windows[0].kernel * windows[1].kernel;
                if weights.len() != want {
                    return Err(LayerError::Length { expected: want, got: weights.len() });
                }
                if bias.len() != *out_channels {
                    return Err(LayerError::Length { expected: *out_channels, got: bias.len() });
                }
                let [oh, ow] = output_size([h, w], *windows)?;
                Ok([*out_channels, oh, ow])
            }
            Layer::Pool { windows, .. } => {
                let [oh, ow] = output_size([h, w], *windows)?;
                Ok([c, oh, ow])
            }
            Layer::Dense { outputs, weights, bias } => {
                let n = c * h * w;
                if weights.len() != outputs * n {
                    return Err(LayerError::Length { expected: outputs * n, got: weights.len() });
                }
                if bias.len() != *outputs {
                    return Err(LayerError::Length { expected: *outputs, got: bias.len() });
                }
                Ok([*outputs, 1, 1])
            }
            Layer::Relu => Ok(input),
            Layer::Custom(layer) => layer.output_dims(input),
        }
    }

    fn apply(&self, x: &Feature) -> Result<Feature, LayerError> {
        match self {
            Layer::Conv2d { out_channels, weights, bias, windows } => {
                conv2d(x, weights, bias, *out_channels, *windows)
            }
            Layer::Pool { kind, windows } => pool2d(x, *kind, *windows),
            Layer::Dense { outputs, weights, bias } => dense(x, weights, bias, *outputs),
            Layer::Relu => Ok(relu(x)),
            Layer::Custom(layer) => layer.evaluate(x),
        }
    }
}

/// Layers run in order. Each layer's output is the next layer's input.
#[derive(Default)]
pub struct Network {
    pub layers: Vec<Layer>,
}

impl Network {
    pub fn new() -> Self {
        Network { layers: Vec::new() }
    }

    pub fn push(&mut self, layer: Layer) -> &mut Self {
        self.layers.push(layer);
        self
    }

    /// The output dims for an input of `input` dims, without running anything.
    pub fn output_dims(&self, input: [usize; 3]) -> Result<[usize; 3], LayerError> {
        self.layers.iter().try_fold(input, |dims, layer| layer.output_dims(dims))
    }

    pub fn forward(&self, input: Feature) -> Result<Feature, LayerError> {
        self.layers.iter().try_fold(input, |x, layer| layer.apply(&x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::Padding;

    fn win(kernel: usize, stride: usize, padding: Padding) -> Window {
        Window { kernel, stride, dilation: 1, padding }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn conv2d_valid_sums_the_diagonal_taps() {
        // 1 channel, 3x3 input holding 1..9; a 2x2 kernel [1 0; 0 1] adds each window's top-left and bottom-right.
        let input = Feature::new(1, 3, 3, (1..=9).map(|v| v as f32).collect()).unwrap();
        let out = conv2d(&input, &[1.0, 0.0, 0.0, 1.0], &[0.0], 1, [win(2, 1, Padding::Valid); 2]).unwrap();
        assert_eq!(out.dims(), [1, 2, 2]);
        assert_eq!(out.data, vec![6.0, 8.0, 12.0, 14.0]);
    }

    #[test]
    fn conv2d_same_padding_adds_bias_and_counts_only_real_pixels() {
        // 2x2 input of 1, 2, 3, 4 and an all-ones 3x3 kernel. Same padding gives 2x2 output. Each window covers
        // the whole input once the zero border is skipped, so every output is 10, plus the bias 0.5.
        let input = Feature::new(1, 2, 2, vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        let out = conv2d(&input, &[1.0; 9], &[0.5], 1, [win(3, 1, Padding::Same); 2]).unwrap();
        assert_eq!(out.dims(), [1, 2, 2]);
        assert!(out.data.iter().all(|&v| close(v, 10.5)), "{:?}", out.data);
    }

    #[test]
    fn conv2d_rejects_a_kernel_larger_than_the_input() {
        let input = Feature::new(1, 2, 2, vec![0.0; 4]).unwrap();
        let err = conv2d(&input, &[0.0; 9], &[0.0], 1, [win(3, 1, Padding::Valid); 2]).unwrap_err();
        assert_eq!(err, LayerError::Window(WindowError::TooSmall));
    }

    #[test]
    fn max_and_average_pool_over_2x2_blocks() {
        let input = Feature::new(1, 4, 4, (1..=16).map(|v| v as f32).collect()).unwrap();
        let max = pool2d(&input, Pooling::Max, [win(2, 2, Padding::Valid); 2]).unwrap();
        assert_eq!(max.data, vec![6.0, 8.0, 14.0, 16.0]);

        // Average pooling with one padded row before and one padded column before. The padded length is 3, so
        // the single window starts in the padding. Its only tap on the input is pixel (0, 0), which is 1. The
        // padded taps are not counted, so the mean is 1.0 and not 0.25.
        let small = Feature::new(1, 2, 2, vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        let padded = Window { kernel: 2, stride: 2, dilation: 1, padding: Padding::Explicit { before: 1, after: 0 } };
        let avg = pool2d(&small, Pooling::Average, [padded; 2]).unwrap();
        assert_eq!(avg.dims(), [1, 1, 1]);
        assert!(close(avg.data[0], 1.0), "{:?}", avg.data);
    }

    #[test]
    fn dense_multiplies_the_flattened_input() {
        // Input 1, 2, 3 flattened; row 0 gives 1 - 3 = -2, row 1 gives 2 * 6 + 1 = 13.
        let input = Feature::new(1, 1, 3, vec![1.0, 2.0, 3.0]).unwrap();
        let out = dense(&input, &[1.0, 0.0, -1.0, 2.0, 2.0, 2.0], &[0.0, 1.0], 2).unwrap();
        assert_eq!(out.dims(), [2, 1, 1]);
        assert_eq!(out.data, vec![-2.0, 13.0]);
    }

    #[test]
    fn lstm_step_with_zero_weights_halves_the_cell_and_gates_at_one_half() {
        // Every pre-activation is 0, so i = f = o = 0.5 and g = 0. Then c' = 0.5 * 2 = 1 and h' = 0.5 * tanh(1).
        let w = RecurrentWeights::new(Cell::Lstm, 1, 1, vec![0.0; 4], vec![0.0; 4], vec![0.0; 4]).unwrap();
        let (h, c) = lstm_step(&w, &[0.0], &[0.0], &[2.0]).unwrap();
        assert!(close(c[0], 1.0), "{c:?}");
        assert!(close(h[0], 0.5 * 1.0f32.tanh()), "{h:?}");
    }

    #[test]
    fn lstm_bias_in_the_second_gate_sets_the_forget_gate() {
        // Gate order is input, forget, candidate, output. A bias of 1 on the forget row makes f = sigmoid(1),
        // so c' = sigmoid(1) * 2 with i = 0.5 and g = 0.
        let w = RecurrentWeights::new(Cell::Lstm, 1, 1, vec![0.0; 4], vec![0.0; 4], vec![0.0, 1.0, 0.0, 0.0]).unwrap();
        let (_, c) = lstm_step(&w, &[0.0], &[0.0], &[2.0]).unwrap();
        assert!(close(c[0], 2.0 / (1.0 + (-1.0f32).exp())), "{c:?}");
    }

    #[test]
    fn gru_step_with_zero_weights_keeps_half_the_state() {
        // z = r = 0.5 and the candidate is tanh(0) = 0, so h' = (1 - 0.5) * 0 + 0.5 * h.
        let w = RecurrentWeights::new(Cell::Gru, 1, 1, vec![0.0; 3], vec![0.0; 3], vec![0.0; 3]).unwrap();
        let h = gru_step(&w, &[0.0], &[2.0]).unwrap();
        assert!(close(h[0], 1.0), "{h:?}");
        // A cell built for the other kind is refused.
        assert_eq!(lstm_step(&w, &[0.0], &[0.0], &[0.0]).unwrap_err(), LayerError::WrongCell);
    }

    struct Scale(f32);

    impl CustomLayer for Scale {
        fn name(&self) -> &str {
            "scale"
        }
        fn output_dims(&self, input: [usize; 3]) -> Result<[usize; 3], LayerError> {
            Ok(input)
        }
        fn evaluate(&self, input: &Feature) -> Result<Feature, LayerError> {
            Feature::new(
                input.channels,
                input.height,
                input.width,
                input.data.iter().map(|v| v * self.0).collect(),
            )
        }
    }

    #[test]
    fn network_chains_layers_and_reports_dims_before_running() {
        // 1x4x4 input, a 3x3 valid convolution to 2 channels (2x2 each), a 2x2 max pool that reduces each channel
        // to 1x1, then a custom layer that doubles the values.
        let mut net = Network::new();
        net.push(Layer::Conv2d {
            out_channels: 2,
            weights: vec![0.0; 2 * 9],
            bias: vec![1.0, 2.0],
            windows: [win(3, 1, Padding::Valid); 2],
        })
        .push(Layer::Pool { kind: Pooling::Max, windows: [win(2, 2, Padding::Valid); 2] })
        .push(Layer::Custom(Box::new(Scale(2.0))));
        assert_eq!(net.output_dims([1, 4, 4]).unwrap(), [2, 1, 1]);
        let out = net.forward(Feature::new(1, 4, 4, vec![0.0; 16]).unwrap()).unwrap();
        // With zero weights the convolution is its bias, so channel 0 is 1 and channel 1 is 2, then doubled.
        assert_eq!(out.data, vec![2.0, 4.0]);
        // A dense layer built for a different input length is refused before anything runs.
        net.push(Layer::Dense { outputs: 1, weights: vec![0.0; 3], bias: vec![0.0] });
        assert_eq!(
            net.output_dims([1, 4, 4]).unwrap_err(),
            LayerError::Length { expected: 2, got: 3 }
        );
    }

    #[test]
    fn relu_clamps_negatives_and_keeps_the_shape() {
        let input = Feature::new(1, 1, 3, vec![-1.0, 0.0, 2.5]).unwrap();
        let out = relu(&input);
        assert_eq!(out.dims(), [1, 1, 3]);
        assert_eq!(out.data, vec![0.0, 0.0, 2.5]);
    }
}
