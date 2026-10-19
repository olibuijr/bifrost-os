//! Output sizes of windowed operators (convolution and pooling) and the scale factor of a Fourier transform.
//! These are the shape rules a graph needs before such an operator is recorded. Nothing is convolved here.
//!
//! The second part is a small inference path for fully connected networks: [`DenseLayer`] applies a weight matrix,
//! a bias and an [`Activation`] to one input vector, and [`softmax`] and [`argmax`] turn the last layer's scores into
//! probabilities and a class index. It runs on the CPU in `f32`, allocates only the output vector, and has no training.

/// How a window is laid over the input. `Explicit` names the pixels added before and after; `Same` and `Valid`
/// derive them from the input size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Padding {
    Explicit { before: usize, after: usize },
    /// The output is `ceil(input / stride)`; the padding that makes it so is split with the odd pixel after.
    Same,
    /// No padding: the window must fit inside the input.
    Valid,
}

/// Why a window cannot be applied to an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    ZeroKernel,
    ZeroStride,
    ZeroDilation,
    /// The input is smaller than the dilated kernel with no padding to cover the difference.
    TooSmall,
}

/// One spatial dimension of a windowed operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub kernel: usize,
    pub stride: usize,
    /// Spacing between kernel taps: 1 is dense, 2 skips every other input pixel.
    pub dilation: usize,
    pub padding: Padding,
}

impl Window {
    /// Extent of the kernel once its taps are spread by the dilation.
    pub fn effective_kernel(&self) -> usize {
        self.dilation * self.kernel.saturating_sub(1) + 1
    }

    fn check(&self) -> Result<(), WindowError> {
        if self.kernel == 0 {
            return Err(WindowError::ZeroKernel);
        }
        if self.stride == 0 {
            return Err(WindowError::ZeroStride);
        }
        if self.dilation == 0 {
            return Err(WindowError::ZeroDilation);
        }
        Ok(())
    }

    /// Pixels added before and after the input along this dimension.
    pub fn pads(&self, input: usize) -> Result<(usize, usize), WindowError> {
        let out = self.output_len(input)?;
        Ok(match self.padding {
            Padding::Explicit { before, after } => (before, after),
            Padding::Valid => (0, 0),
            Padding::Same => {
                let total = (out.saturating_sub(1) * self.stride + self.effective_kernel()).saturating_sub(input);
                let before = total / 2;
                (before, total - before)
            }
        })
    }

    /// Number of window positions along this dimension.
    pub fn output_len(&self, input: usize) -> Result<usize, WindowError> {
        self.check()?;
        match self.padding {
            Padding::Same => Ok(input.div_ceil(self.stride)),
            Padding::Explicit { before, after } => self.positions(input + before + after),
            Padding::Valid => self.positions(input),
        }
    }

    fn positions(&self, padded: usize) -> Result<usize, WindowError> {
        let span = self.effective_kernel();
        if padded < span {
            return Err(WindowError::TooSmall);
        }
        Ok((padded - span) / self.stride + 1)
    }
}

/// Output height and width of a two-dimensional windowed operator, given the input's `[height, width]`.
pub fn output_size(input: [usize; 2], windows: [Window; 2]) -> Result<[usize; 2], WindowError> {
    Ok([windows[0].output_len(input[0])?, windows[1].output_len(input[1])?])
}

/// Scale applied to the outputs of a Fourier transform. `n` is the number of points over all transformed dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FftScaling {
    None,
    /// Divide by `n`.
    Size,
    /// Divide by `sqrt(n)`, which keeps the signal energy the same before and after the transform.
    Unitary,
}

impl FftScaling {
    pub fn factor(self, n: usize) -> f64 {
        match self {
            FftScaling::None => 1.0,
            FftScaling::Size => 1.0 / n as f64,
            FftScaling::Unitary => 1.0 / (n as f64).sqrt(),
        }
    }
}

/// Activation applied to each output of a dense layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activation {
    Identity,
    /// `max(x, 0)`.
    Relu,
    /// `x` for `x >= 0`, and `slope * x` below zero.
    LeakyRelu(f32),
    /// `1 / (1 + e^-x)`, which maps every value into (0, 1).
    Sigmoid,
    /// Hyperbolic tangent, which maps every value into (-1, 1).
    Tanh,
}

impl Activation {
    /// The activation's value at `x`.
    pub fn eval(self, x: f32) -> f32 {
        match self {
            Activation::Identity => x,
            Activation::Relu => {
                if x > 0.0 {
                    x
                } else {
                    0.0
                }
            }
            Activation::LeakyRelu(slope) => {
                if x >= 0.0 {
                    x
                } else {
                    slope * x
                }
            }
            Activation::Sigmoid => 1.0 / (1.0 + (-x).exp()),
            Activation::Tanh => x.tanh(),
        }
    }
}

/// Why a dense layer cannot be built or applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenseError {
    /// The layer has no inputs or no outputs.
    EmptyLayer,
    /// The weights, the bias or the input do not match the layer's shape.
    ShapeMismatch,
}

/// A fully connected layer: `outputs` neurons, each reading all `inputs` values.
///
/// The weights are row-major, one row of `inputs` values per output, so output `o` is
/// `activation(bias[o] + sum over i of weights[o * inputs + i] * input[i])`.
#[derive(Clone, Debug, PartialEq)]
pub struct DenseLayer {
    inputs: usize,
    outputs: usize,
    weights: Vec<f32>,
    bias: Vec<f32>,
    activation: Activation,
}

impl DenseLayer {
    /// Builds a layer from its weights and bias. The weights must hold `inputs * outputs` values and the bias
    /// one value per output.
    pub fn new(
        inputs: usize,
        outputs: usize,
        weights: Vec<f32>,
        bias: Vec<f32>,
        activation: Activation,
    ) -> Result<Self, DenseError> {
        if inputs == 0 || outputs == 0 {
            return Err(DenseError::EmptyLayer);
        }
        let expected = inputs.checked_mul(outputs).ok_or(DenseError::ShapeMismatch)?;
        if weights.len() != expected || bias.len() != outputs {
            return Err(DenseError::ShapeMismatch);
        }
        Ok(DenseLayer { inputs, outputs, weights, bias, activation })
    }

    /// Number of values the layer reads.
    pub fn inputs(&self) -> usize {
        self.inputs
    }

    /// Number of values the layer produces.
    pub fn outputs(&self) -> usize {
        self.outputs
    }

    /// Applies the layer to one input vector of exactly `inputs` values.
    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>, DenseError> {
        if input.len() != self.inputs {
            return Err(DenseError::ShapeMismatch);
        }
        let out = self
            .weights
            .chunks_exact(self.inputs)
            .zip(&self.bias)
            .map(|(row, b)| {
                let sum: f32 = row.iter().zip(input).map(|(w, x)| w * x).sum();
                self.activation.eval(b + sum)
            })
            .collect();
        Ok(out)
    }
}

/// Softmax of a vector: `e^(v - max) / sum`, so the outputs are positive and sum to 1. Subtracting the maximum keeps
/// large scores from overflowing. An empty input gives an empty output.
pub fn softmax(values: &[f32]) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = values.iter().map(|v| (v - max).exp()).collect();
    let total: f32 = exps.iter().sum();
    exps.iter().map(|e| e / total).collect()
}

/// Index of the largest value. The first of several equal maxima wins, NaN values are skipped, and an input with no
/// usable value (including an empty one) gives None.
pub fn argmax(values: &[f32]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, &v) in values.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        match best {
            Some((_, b)) if v <= b => {}
            _ => best = Some((i, v)),
        }
    }
    best.map(|(i, _)| i)
}

/// Row-major matrix product C = A * B for A (m x k), B (k x n), giving C (m x n). This is the CPU reference for the
/// `gpu/shaders/matmul.comp` compute kernel. None when a slice length does not match its dimensions.
pub fn matmul(a: &[f32], b: &[f32], m: usize, k: usize, n: usize) -> Option<Vec<f32>> {
    if a.len() != m.checked_mul(k)? || b.len() != k.checked_mul(n)? {
        return None;
    }
    let mut c = vec![0.0f32; m * n];
    for i in 0..m {
        for p in 0..k {
            let av = a[i * k + p];
            for j in 0..n {
                c[i * n + j] += av * b[p * n + j];
            }
        }
    }
    Some(c)
}

// ---- A model file and a runtime for chains of dense layers --------------------------------------------------------

/// First four bytes of a model file.
pub const MODEL_MAGIC: &[u8; 4] = b"BNN1";
/// Most layers a model file may hold, and the most values in one dimension. Both keep a hostile file from asking for
/// a huge allocation before its length has been checked.
pub const MODEL_MAX_LAYERS: usize = 256;
pub const MODEL_MAX_DIM: usize = 1 << 20;

/// Why a model cannot be loaded or run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// The file does not start with [`MODEL_MAGIC`].
    BadMagic,
    /// The file ends before the layers it announces.
    Truncated,
    /// Bytes remain after the last layer.
    TrailingBytes,
    /// A layer names an activation the runtime does not know.
    BadActivation,
    /// The layer count or a dimension is over the limit, or the model has no layer.
    BadSize,
    /// A layer's weights do not fit its shape.
    Layer(DenseError),
    /// A layer's inputs are not the outputs of the layer before it.
    Chain,
    /// The input is not as long as the first layer's inputs.
    Input,
}

/// A trained network of dense layers in the order they run: the weights are read from a file, the input goes through
/// each layer and the last layer's scores come out. It runs on the CPU in `f32` and does no training.
///
/// File layout, little-endian: `"BNN1"`, `u32` layer count, then per layer `u32 inputs`, `u32 outputs`, `u8` activation
/// (0 identity, 1 relu, 2 leaky relu, 3 sigmoid, 4 tanh), `f32` leaky slope (0 when unused), `inputs * outputs` `f32`
/// weights row by row, and `outputs` `f32` biases.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    layers: Vec<DenseLayer>,
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], ModelError> {
    let end = at.checked_add(n).ok_or(ModelError::Truncated)?;
    let s = bytes.get(*at..end).ok_or(ModelError::Truncated)?;
    *at = end;
    Ok(s)
}

fn take_u32(bytes: &[u8], at: &mut usize) -> Result<u32, ModelError> {
    let b = take(bytes, at, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn take_f32s(bytes: &[u8], at: &mut usize, n: usize) -> Result<Vec<f32>, ModelError> {
    let len = n.checked_mul(4).ok_or(ModelError::Truncated)?;
    let b = take(bytes, at, len)?;
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

impl Model {
    /// A model from layers that already chain: at least one layer, each reading what the one before it wrote.
    pub fn new(layers: Vec<DenseLayer>) -> Result<Model, ModelError> {
        if layers.is_empty() || layers.len() > MODEL_MAX_LAYERS {
            return Err(ModelError::BadSize);
        }
        if layers.windows(2).any(|w| w[0].outputs() != w[1].inputs()) {
            return Err(ModelError::Chain);
        }
        Ok(Model { layers })
    }

    /// Reads a model file. Every length is checked against the bytes present before anything is allocated.
    pub fn from_bytes(bytes: &[u8]) -> Result<Model, ModelError> {
        let mut at = 0;
        if take(bytes, &mut at, 4)? != MODEL_MAGIC {
            return Err(ModelError::BadMagic);
        }
        let count = take_u32(bytes, &mut at)? as usize;
        if count == 0 || count > MODEL_MAX_LAYERS {
            return Err(ModelError::BadSize);
        }
        let mut layers = Vec::with_capacity(count);
        for _ in 0..count {
            let inputs = take_u32(bytes, &mut at)? as usize;
            let outputs = take_u32(bytes, &mut at)? as usize;
            if inputs == 0 || outputs == 0 || inputs > MODEL_MAX_DIM || outputs > MODEL_MAX_DIM {
                return Err(ModelError::BadSize);
            }
            let tag = take(bytes, &mut at, 1)?[0];
            let slope = f32::from_bits(take_u32(bytes, &mut at)?);
            let activation = match tag {
                0 => Activation::Identity,
                1 => Activation::Relu,
                2 => Activation::LeakyRelu(slope),
                3 => Activation::Sigmoid,
                4 => Activation::Tanh,
                _ => return Err(ModelError::BadActivation),
            };
            let weights = take_f32s(bytes, &mut at, inputs.checked_mul(outputs).ok_or(ModelError::BadSize)?)?;
            let bias = take_f32s(bytes, &mut at, outputs)?;
            layers.push(DenseLayer::new(inputs, outputs, weights, bias, activation).map_err(ModelError::Layer)?);
        }
        if at != bytes.len() {
            return Err(ModelError::TrailingBytes);
        }
        Model::new(layers)
    }

    /// The model file for this model; [`Model::from_bytes`] reads it back to an equal model.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MODEL_MAGIC.to_vec();
        out.extend_from_slice(&(self.layers.len() as u32).to_le_bytes());
        for l in &self.layers {
            out.extend_from_slice(&(l.inputs as u32).to_le_bytes());
            out.extend_from_slice(&(l.outputs as u32).to_le_bytes());
            let (tag, slope) = match l.activation {
                Activation::Identity => (0u8, 0.0f32),
                Activation::Relu => (1, 0.0),
                Activation::LeakyRelu(s) => (2, s),
                Activation::Sigmoid => (3, 0.0),
                Activation::Tanh => (4, 0.0),
            };
            out.push(tag);
            out.extend_from_slice(&slope.to_bits().to_le_bytes());
            for w in &l.weights {
                out.extend_from_slice(&w.to_le_bytes());
            }
            for b in &l.bias {
                out.extend_from_slice(&b.to_le_bytes());
            }
        }
        out
    }

    /// Number of values the model reads.
    pub fn inputs(&self) -> usize {
        self.layers[0].inputs()
    }

    /// Number of values the model writes.
    pub fn outputs(&self) -> usize {
        self.layers[self.layers.len() - 1].outputs()
    }

    /// Runs the input through every layer and returns the last layer's scores.
    pub fn run(&self, input: &[f32]) -> Result<Vec<f32>, ModelError> {
        if input.len() != self.inputs() {
            return Err(ModelError::Input);
        }
        let mut v = input.to_vec();
        for l in &self.layers {
            v = l.forward(&v).map_err(ModelError::Layer)?;
        }
        Ok(v)
    }

    /// The most likely class and its probability: softmax over the scores, then the largest. None when every score
    /// is NaN.
    pub fn classify(&self, input: &[f32]) -> Result<Option<(usize, f32)>, ModelError> {
        let p = softmax(&self.run(input)?);
        Ok(argmax(&p).map(|i| (i, p[i])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(kernel: usize, stride: usize, dilation: usize, padding: Padding) -> Window {
        Window { kernel, stride, dilation, padding }
    }

    #[test]
    fn valid_windows_count_positions_that_fit() {
        let w = win(3, 1, 1, Padding::Valid);
        assert_eq!(w.output_len(5), Ok(3));
        assert_eq!(win(3, 2, 1, Padding::Valid).output_len(5), Ok(2), "the last stride would run past the input");
        assert_eq!(w.pads(5), Ok((0, 0)));
    }

    #[test]
    fn same_padding_gives_ceil_of_input_over_stride_and_splits_the_pad() {
        let w = win(3, 2, 1, Padding::Same);
        assert_eq!(w.output_len(5), Ok(3));
        assert_eq!(w.pads(5), Ok((1, 1)), "total pad is (3-1)*2+3-5 = 2");
        assert_eq!(win(2, 1, 1, Padding::Same).pads(4), Ok((0, 1)), "an odd total puts the extra pixel after");
    }

    #[test]
    fn explicit_padding_adds_to_the_input_before_counting() {
        let w = win(3, 1, 1, Padding::Explicit { before: 1, after: 1 });
        assert_eq!(w.output_len(4), Ok(4), "a 3-tap window over 4+2 pixels has 4 positions");
        assert_eq!(w.pads(4), Ok((1, 1)));
    }

    #[test]
    fn dilation_widens_the_kernel_it_is_checked_against() {
        let w = win(3, 1, 2, Padding::Valid);
        assert_eq!(w.effective_kernel(), 5);
        assert_eq!(w.output_len(5), Ok(1));
        assert_eq!(w.output_len(4), Err(WindowError::TooSmall), "a 5-tap span does not fit in 4 pixels");
    }

    #[test]
    fn zero_kernel_stride_or_dilation_is_refused() {
        assert_eq!(win(0, 1, 1, Padding::Valid).output_len(5), Err(WindowError::ZeroKernel));
        assert_eq!(win(3, 0, 1, Padding::Same).output_len(5), Err(WindowError::ZeroStride));
        assert_eq!(win(3, 1, 0, Padding::Valid).output_len(5), Err(WindowError::ZeroDilation));
    }

    #[test]
    fn two_dimensional_output_applies_each_axis_on_its_own() {
        let rows = win(2, 2, 1, Padding::Valid);
        let cols = win(3, 1, 1, Padding::Same);
        assert_eq!(output_size([8, 5], [rows, cols]), Ok([4, 5]));
    }

    #[test]
    fn fft_scaling_factors_follow_the_total_point_count() {
        assert_eq!(FftScaling::None.factor(16), 1.0);
        assert_eq!(FftScaling::Size.factor(16), 0.0625);
        assert_eq!(FftScaling::Unitary.factor(16), 0.25);
        assert_eq!(FftScaling::Unitary.factor(4), 0.5);
    }

    #[test]
    fn activations_follow_their_definitions() {
        assert_eq!(Activation::Relu.eval(-2.0), 0.0);
        assert_eq!(Activation::Relu.eval(3.0), 3.0);
        assert!((Activation::LeakyRelu(0.1).eval(-2.0) + 0.2).abs() < 1e-6);
        assert_eq!(Activation::Identity.eval(-7.5), -7.5);
        assert!((Activation::Sigmoid.eval(0.0) - 0.5).abs() < 1e-6);
        assert!((Activation::Tanh.eval(0.0)).abs() < 1e-6);
    }

    #[test]
    fn dense_layer_computes_weighted_sums_plus_bias() {
        // Row 0 is [1, 2], row 1 is [3, 4]; with input [1, 1] the sums are 3 and 7.
        let layer = DenseLayer::new(2, 2, vec![1.0, 2.0, 3.0, 4.0], vec![0.5, -1.0], Activation::Identity).unwrap();
        assert_eq!(layer.forward(&[1.0, 1.0]).unwrap(), vec![3.5, 6.0]);
        assert_eq!((layer.inputs(), layer.outputs()), (2, 2));
    }

    #[test]
    fn relu_clips_the_negative_output_of_a_dense_layer() {
        let layer = DenseLayer::new(2, 2, vec![1.0, 2.0, 3.0, 4.0], vec![-10.0, 0.0], Activation::Relu).unwrap();
        // Outputs before the activation are 3 - 10 = -7, which clips to 0, and 7 + 0 = 7, which passes through.
        assert_eq!(layer.forward(&[1.0, 1.0]).unwrap(), vec![0.0, 7.0]);
    }

    #[test]
    fn dense_layer_refuses_bad_shapes() {
        assert_eq!(DenseLayer::new(0, 2, vec![], vec![0.0; 2], Activation::Identity), Err(DenseError::EmptyLayer));
        assert_eq!(
            DenseLayer::new(2, 2, vec![1.0; 3], vec![0.0; 2], Activation::Identity),
            Err(DenseError::ShapeMismatch),
            "2 x 2 weights need four values"
        );
        let layer = DenseLayer::new(2, 1, vec![1.0, 1.0], vec![0.0], Activation::Identity).unwrap();
        assert_eq!(layer.forward(&[1.0, 2.0, 3.0]), Err(DenseError::ShapeMismatch));
    }

    #[test]
    fn softmax_sums_to_one_and_survives_large_scores() {
        let even = softmax(&[0.0, 0.0]);
        assert_eq!(even, vec![0.5, 0.5]);
        let big = softmax(&[1000.0, 1000.0, 999.0]);
        assert!(big.iter().all(|p| p.is_finite()));
        assert!((big[0] - big[1]).abs() < 1e-6);
        assert!((big.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(softmax(&[]).is_empty());
    }

    #[test]
    fn argmax_picks_the_first_largest_value() {
        assert_eq!(argmax(&[1.0, 3.0, 3.0, 2.0]), Some(1), "a tie goes to the first index");
        assert_eq!(argmax(&[f32::NAN, -1.0]), Some(1), "NaN is skipped");
        assert_eq!(argmax(&[]), None);
        assert_eq!(argmax(&[f32::NAN]), None);
    }
}

#[cfg(test)]
mod matmul_tests {
    use super::*;

    #[test]
    fn two_by_two_times_two_by_one() {
        // [1 2; 3 4] * [5; 6] = [17; 39]
        assert_eq!(matmul(&[1.0, 2.0, 3.0, 4.0], &[5.0, 6.0], 2, 2, 1), Some(vec![17.0, 39.0]));
    }

    #[test]
    fn identity_keeps_the_matrix() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(matmul(&a, &[1.0, 0.0, 0.0, 1.0], 3, 2, 2), Some(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
    }

    #[test]
    fn mismatched_lengths_are_refused() {
        assert_eq!(matmul(&[1.0, 2.0, 3.0], &[1.0, 2.0], 2, 2, 1), None);
        assert_eq!(matmul(&[1.0, 2.0], &[1.0], 1, 2, 1), None);
    }

    fn xor_model() -> Model {
        // hidden: h1 = relu(x + y), h2 = relu(x + y - 1); the scores are 1 - h1 + 2 * h2 and h1 - 2 * h2
        let hidden = DenseLayer::new(2, 2, vec![1.0, 1.0, 1.0, 1.0], vec![0.0, -1.0], Activation::Relu).unwrap();
        let out = DenseLayer::new(2, 2, vec![-1.0, 2.0, 1.0, -2.0], vec![1.0, 0.0], Activation::Identity).unwrap();
        Model::new(vec![hidden, out]).unwrap()
    }

    #[test]
    fn a_model_runs_its_layers_in_order() {
        let m = xor_model();
        assert_eq!((m.inputs(), m.outputs()), (2, 2));
        // class 1 wins exactly when one of the two inputs is on
        assert_eq!(m.classify(&[0.0, 0.0]).unwrap().map(|c| c.0), Some(0));
        assert_eq!(m.classify(&[1.0, 0.0]).unwrap().map(|c| c.0), Some(1));
        assert_eq!(m.classify(&[0.0, 1.0]).unwrap().map(|c| c.0), Some(1));
        assert_eq!(m.classify(&[1.0, 1.0]).unwrap().map(|c| c.0), Some(0));
        let (_, p) = m.classify(&[1.0, 0.0]).unwrap().unwrap();
        assert!(p > 0.5 && p < 1.0);
    }

    #[test]
    fn a_model_survives_its_file() {
        let m = Model::new(vec![
            DenseLayer::new(3, 2, vec![0.5, -1.0, 2.0, 0.25, 0.0, -0.75], vec![0.1, -0.2], Activation::LeakyRelu(0.1)).unwrap(),
            DenseLayer::new(2, 1, vec![1.0, 1.0], vec![0.0], Activation::Sigmoid).unwrap(),
        ])
        .unwrap();
        let bytes = m.to_bytes();
        assert_eq!(&bytes[..4], MODEL_MAGIC);
        assert_eq!(Model::from_bytes(&bytes), Ok(m.clone()));
        assert_eq!(Model::from_bytes(&bytes).unwrap().run(&[1.0, 2.0, 3.0]), m.run(&[1.0, 2.0, 3.0]));
    }

    #[test]
    fn a_bad_model_file_is_refused_without_a_large_allocation() {
        let good = xor_model().to_bytes();
        assert_eq!(Model::from_bytes(b"XXXX\0\0\0\0"), Err(ModelError::BadMagic));
        assert_eq!(Model::from_bytes(&good[..good.len() - 1]), Err(ModelError::Truncated));
        let mut extra = good.clone();
        extra.push(0);
        assert_eq!(Model::from_bytes(&extra), Err(ModelError::TrailingBytes));
        let mut tag = good.clone();
        tag[16] = 9;
        assert_eq!(Model::from_bytes(&tag), Err(ModelError::BadActivation));
        // a layer that claims 2^20 x 2^20 weights in a file a few bytes long
        let mut huge = MODEL_MAGIC.to_vec();
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.extend_from_slice(&(MODEL_MAX_DIM as u32).to_le_bytes());
        huge.extend_from_slice(&(MODEL_MAX_DIM as u32).to_le_bytes());
        huge.extend_from_slice(&[0, 0, 0, 0, 0]);
        assert_eq!(Model::from_bytes(&huge), Err(ModelError::Truncated));
        let mut too_many = MODEL_MAGIC.to_vec();
        too_many.extend_from_slice(&1000u32.to_le_bytes());
        assert_eq!(Model::from_bytes(&too_many), Err(ModelError::BadSize));
        let mut none = MODEL_MAGIC.to_vec();
        none.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(Model::from_bytes(&none), Err(ModelError::BadSize));
    }

    #[test]
    fn layers_must_chain_and_the_input_must_fit() {
        let a = DenseLayer::new(2, 3, vec![0.0; 6], vec![0.0; 3], Activation::Identity).unwrap();
        let b = DenseLayer::new(2, 1, vec![0.0; 2], vec![0.0], Activation::Identity).unwrap();
        assert_eq!(Model::new(vec![a.clone(), b]), Err(ModelError::Chain));
        assert_eq!(Model::new(Vec::new()), Err(ModelError::BadSize));
        let m = Model::new(vec![a]).unwrap();
        assert_eq!(m.run(&[1.0]), Err(ModelError::Input));
    }
}
