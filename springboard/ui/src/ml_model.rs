//! A model that runs: a chain of layers that maps one input tensor to one output tensor (a prediction).
//! A model is built with [`Model::new`], the layer methods and [`Model::with_output`], or read from its text form with
//! [`Model::parse`]. [`Model::predict`] checks the input provider against the declared input, runs each layer in order
//! and returns the output as a feature provider. The layers are dense (a matrix product and a bias), relu and softmax.
//! This is a plain CPU evaluator: it uses no compute device and keeps nothing between calls (state lives in `ml_state`).
use crate::features::{check, DictionaryProvider, FeatureError, FeatureProvider, FeatureSpec, FeatureValue, Kind};
use crate::ml_tensor::{Tensor, TensorError};

/// One step of a model.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer {
    /// A fully connected layer: `out[j] = bias[j] + sum over i of input[i] * weights[i * outputs + j]`.
    Dense { outputs: usize, weights: Vec<f32>, bias: Vec<f32> },
    /// `max(0, x)` for each element.
    Relu,
    /// `exp(x - max) / sum(exp(x - max))` over all elements, which is stable for large inputs.
    Softmax,
}

/// Why a model could not be built, read or run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// The input provider lacks the declared input or holds it with another kind.
    Feature(FeatureError),
    /// The input tensor's element count is not the model's input length.
    InputLength { expected: usize, found: usize },
    /// A dense layer's weight count is not its input width times its outputs, or its bias count is not its outputs.
    LayerSize,
    /// The model has a zero-length input or dense layer, or no output name.
    Incomplete,
    /// A text line could not be read. The number is the 1-based line.
    Parse { line: usize },
    /// A tensor could not be created from the output values.
    Tensor(TensorError),
}

/// A model with one named tensor input, a chain of layers and one named output.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    input: String,
    input_len: usize,
    output: Option<String>,
    layers: Vec<Layer>,
    width: usize,
}

impl Model {
    /// A model with no layers yet. `input_len` is the number of elements the input tensor must hold.
    pub fn new(input: &str, input_len: usize) -> Result<Self, ModelError> {
        if input_len == 0 {
            return Err(ModelError::Incomplete);
        }
        Ok(Model { input: input.to_owned(), input_len, output: None, layers: Vec::new(), width: input_len })
    }

    /// Names the output that predictions return.
    pub fn with_output(mut self, output: &str) -> Self {
        self.output = Some(output.to_owned());
        self
    }

    /// A dense layer. `weights` holds `width * outputs` values, one row per input element, and `bias` one value per
    /// output. Both counts are checked here, so a built model cannot fail on its own shapes.
    pub fn dense(mut self, outputs: usize, weights: Vec<f32>, bias: Vec<f32>) -> Result<Self, ModelError> {
        if outputs == 0 {
            return Err(ModelError::Incomplete);
        }
        let expected = self.width.checked_mul(outputs).ok_or(ModelError::LayerSize)?;
        if weights.len() != expected || bias.len() != outputs {
            return Err(ModelError::LayerSize);
        }
        self.layers.push(Layer::Dense { outputs, weights, bias });
        self.width = outputs;
        Ok(self)
    }

    /// A relu layer, which keeps the width.
    pub fn relu(mut self) -> Self {
        self.layers.push(Layer::Relu);
        self
    }

    /// A softmax layer, which keeps the width.
    pub fn softmax(mut self) -> Self {
        self.layers.push(Layer::Softmax);
        self
    }

    /// The name of the input tensor.
    pub fn input_name(&self) -> &str {
        &self.input
    }

    /// The number of elements the input tensor must hold.
    pub fn input_len(&self) -> usize {
        self.input_len
    }

    /// The name of the output, once one is set.
    pub fn output_name(&self) -> Option<&str> {
        self.output.as_deref()
    }

    /// The number of elements the output holds.
    pub fn output_len(&self) -> usize {
        self.width
    }

    /// The layers in the order they run.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Reads a model from its text form. Blank lines and lines starting with `#` are skipped. The lines are:
    /// `input <name> <length>`, then the layers in order (`dense <outputs> <weights> <bias>`, `relu`, `softmax`),
    /// then `output <name>`. Weights and bias are comma-separated numbers; the weights are row-major, one row per
    /// input element.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let mut model: Option<Model> = None;
        for (n, raw) in text.lines().enumerate() {
            let line = n + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            match tokens.as_slice() {
                ["input", name, len] => {
                    if model.is_some() {
                        return Err(ModelError::Parse { line });
                    }
                    let len: usize = len.parse().map_err(|_| ModelError::Parse { line })?;
                    model = Some(Model::new(name, len).map_err(|_| ModelError::Parse { line })?);
                }
                ["output", name] => {
                    let m = model.take().ok_or(ModelError::Parse { line })?;
                    model = Some(m.with_output(name));
                }
                ["dense", outputs, weights, bias] => {
                    let m = model.take().ok_or(ModelError::Parse { line })?;
                    let outputs: usize = outputs.parse().map_err(|_| ModelError::Parse { line })?;
                    let weights = parse_list(weights).ok_or(ModelError::Parse { line })?;
                    let bias = parse_list(bias).ok_or(ModelError::Parse { line })?;
                    model = Some(m.dense(outputs, weights, bias).map_err(|_| ModelError::Parse { line })?);
                }
                ["relu"] => {
                    let m = model.take().ok_or(ModelError::Parse { line })?;
                    model = Some(m.relu());
                }
                ["softmax"] => {
                    let m = model.take().ok_or(ModelError::Parse { line })?;
                    model = Some(m.softmax());
                }
                _ => return Err(ModelError::Parse { line }),
            }
        }
        match model {
            Some(m) if m.output.is_some() => Ok(m),
            _ => Err(ModelError::Incomplete),
        }
    }

    /// Runs the model on the input provider and returns a provider holding the output tensor under the output name.
    /// The input may have any shape as long as it holds `input_len` elements; the output is a vector.
    pub fn predict(&self, input: &dyn FeatureProvider) -> Result<DictionaryProvider, ModelError> {
        let output = self.output.as_deref().ok_or(ModelError::Incomplete)?;
        let spec = FeatureSpec { name: self.input.clone(), kind: Kind::Tensor };
        check(input, &[spec]).map_err(ModelError::Feature)?;
        let tensor = input
            .value(&self.input)
            .and_then(FeatureValue::as_tensor)
            .ok_or_else(|| ModelError::Feature(FeatureError::Missing(self.input.clone())))?;
        let values = tensor.values();
        if values.len() != self.input_len {
            return Err(ModelError::InputLength { expected: self.input_len, found: values.len() });
        }
        let mut x = values.to_vec();
        for layer in &self.layers {
            x = run(layer, x);
        }
        let len = x.len();
        let out = Tensor::new(&[len], x).map_err(ModelError::Tensor)?;
        Ok(DictionaryProvider::new().with(output, FeatureValue::Tensor(out)))
    }
}

/// Parses a comma-separated list of numbers. `None` when an item is not a number.
fn parse_list(text: &str) -> Option<Vec<f32>> {
    text.split(',').map(|item| item.trim().parse::<f32>().ok()).collect()
}

/// Applies one layer. The width is already known to match, because the builder checked every dense layer.
fn run(layer: &Layer, x: Vec<f32>) -> Vec<f32> {
    match layer {
        Layer::Dense { outputs, weights, bias } => {
            let mut out = bias.clone();
            for (i, xi) in x.iter().enumerate() {
                let row = &weights[i * outputs..(i + 1) * outputs];
                for (o, w) in out.iter_mut().zip(row) {
                    *o += xi * w;
                }
            }
            out
        }
        Layer::Relu => x.into_iter().map(|v| v.max(0.0_f32)).collect(),
        Layer::Softmax => {
            let max = x.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let e: Vec<f32> = x.iter().map(|v| (v - max).exp()).collect();
            let sum: f32 = e.iter().sum();
            e.iter().map(|v| v / sum).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, values: &[f32]) -> DictionaryProvider {
        let t = Tensor::new(&[values.len()], values.to_vec()).unwrap();
        DictionaryProvider::new().with(name, FeatureValue::Tensor(t))
    }

    fn output_values(out: &DictionaryProvider, name: &str) -> Option<Vec<f32>> {
        out.value(name).and_then(FeatureValue::as_tensor).map(|t| t.values().to_vec())
    }

    #[test]
    fn a_dense_layer_computes_the_bias_plus_the_matrix_product() {
        // Rows are input elements: input 0 feeds [1, 2], input 1 feeds [3, 4].
        // out[0] = 0.5 + 1*1 + 2*3 = 7.5 and out[1] = -1 + 1*2 + 2*4 = 9.
        let m = Model::new("x", 2).unwrap().dense(2, vec![1.0, 2.0, 3.0, 4.0], vec![0.5, -1.0]).unwrap().with_output("y");
        let out = m.predict(&input("x", &[1.0, 2.0])).unwrap();
        assert_eq!(output_values(&out, "y"), Some(vec![7.5, 9.0]));
    }

    #[test]
    fn relu_clamps_negative_values_to_zero() {
        let m = Model::new("x", 3).unwrap().relu().with_output("y");
        let out = m.predict(&input("x", &[-1.0, 0.0, 2.0])).unwrap();
        assert_eq!(output_values(&out, "y"), Some(vec![0.0, 0.0, 2.0]));
    }

    #[test]
    fn softmax_gives_equal_inputs_equal_shares_and_sums_to_one() {
        let m = Model::new("x", 3).unwrap().softmax().with_output("p");
        let out = m.predict(&input("x", &[0.0, 0.0, 0.0])).unwrap();
        let p = output_values(&out, "p").unwrap();
        assert!(p.iter().all(|v| (v - 1.0_f32 / 3.0).abs() < 1e-6));
        let two = Model::new("x", 2).unwrap().softmax().with_output("p");
        let half = output_values(&two.predict(&input("x", &[1.0, 1.0])).unwrap(), "p");
        assert_eq!(half, Some(vec![0.5, 0.5]));
    }

    #[test]
    fn predict_refuses_a_missing_or_wrongly_typed_input_and_a_wrong_length() {
        let m = Model::new("x", 2).unwrap().relu().with_output("y");
        assert_eq!(
            m.predict(&DictionaryProvider::new()),
            Err(ModelError::Feature(FeatureError::Missing("x".into())))
        );
        let text = DictionaryProvider::new().with("x", FeatureValue::Text("no".into()));
        assert_eq!(
            m.predict(&text),
            Err(ModelError::Feature(FeatureError::WrongKind {
                name: "x".into(),
                expected: Kind::Tensor,
                found: Kind::Text,
            }))
        );
        assert_eq!(
            m.predict(&input("x", &[1.0, 2.0, 3.0])),
            Err(ModelError::InputLength { expected: 2, found: 3 })
        );
    }

    #[test]
    fn the_builder_refuses_layers_whose_counts_do_not_fit() {
        let short_weights = Model::new("x", 2).unwrap().dense(2, vec![1.0; 3], vec![0.0; 2]);
        assert_eq!(short_weights, Err(ModelError::LayerSize));
        let short_bias = Model::new("x", 2).unwrap().dense(2, vec![1.0; 4], vec![0.0; 1]);
        assert_eq!(short_bias, Err(ModelError::LayerSize));
        assert_eq!(Model::new("x", 0), Err(ModelError::Incomplete));
    }

    #[test]
    fn parse_reads_the_text_form_and_gives_the_same_result_as_the_builder() {
        let text = "# two inputs, two outputs\n\ninput x 2\ndense 2 1,2,3,4 0.5,-1\nrelu\noutput y\n";
        let m = Model::parse(text).unwrap();
        assert_eq!((m.input_name(), m.input_len(), m.output_name(), m.output_len()), ("x", 2, Some("y"), 2));
        let out = m.predict(&input("x", &[1.0, 2.0])).unwrap();
        assert_eq!(output_values(&out, "y"), Some(vec![7.5, 9.0]));
    }

    #[test]
    fn parse_names_the_line_it_cannot_read() {
        let bad_weights = "input x 2\ndense 2 1,2,3 0,0\noutput y";
        assert_eq!(Model::parse(bad_weights), Err(ModelError::Parse { line: 2 }));
        let unknown_op = "input x 2\ntanh\noutput y";
        assert_eq!(Model::parse(unknown_op), Err(ModelError::Parse { line: 2 }));
        let no_output = "input x 2\nrelu";
        assert_eq!(Model::parse(no_output), Err(ModelError::Incomplete));
    }
}
