//! On-device model updates: a plain stochastic gradient descent step that personalizes a model from labelled samples.
//! The trainable layer is the last dense layer of a [`Model`]. Layers before it are frozen and run once per sample to
//! give the features that layer reads. The loss is softmax cross-entropy over the last dense layer's outputs, so the
//! class count is that layer's width. [`update`] returns a new model and leaves the old one unchanged, and it runs on
//! the calling thread: a caller that must not block runs it on its own worker.
//! Sample order is fixed, so the same model, samples and parameters always give the same result.
use crate::ml_model::{Layer, Model, ModelError};

/// Settings for one update. The learning rate and the epoch count are the optimizer settings a model update accepts.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    /// The step size applied to each sample's gradient. Must be finite and above zero.
    pub learning_rate: f32,
    /// How many passes over the samples. Must be at least one.
    pub epochs: usize,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters { learning_rate: 0.1, epochs: 1 }
    }
}

/// One labelled training sample: the input tensor and the index of its correct output.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub input: Vec<f32>,
    pub label: usize,
}

/// What an update did. The losses are mean cross-entropy over the samples, before and after training.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub samples: usize,
    pub epochs: usize,
    pub loss_before: f32,
    pub loss_after: f32,
}

/// Why an update could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateError {
    /// There are no samples to learn from.
    Empty,
    /// The epoch count is zero.
    Epochs,
    /// The learning rate is zero, negative or not finite.
    LearningRate,
    /// The model has no dense layer to train.
    NoDenseLayer,
    /// A sample's input does not hold the model's input length.
    InputLength { sample: usize, expected: usize, found: usize },
    /// A sample's label is not one of the trained layer's outputs.
    Label { sample: usize, label: usize, classes: usize },
    /// The updated model could not be rebuilt.
    Model(ModelError),
}

impl From<ModelError> for UpdateError {
    fn from(error: ModelError) -> Self {
        UpdateError::Model(error)
    }
}

/// Trains the last dense layer of `model` on `samples` and returns the updated model with a report.
pub fn update(model: &Model, samples: &[Sample], params: &Parameters) -> Result<(Model, Report), UpdateError> {
    if samples.is_empty() {
        return Err(UpdateError::Empty);
    }
    if params.epochs == 0 {
        return Err(UpdateError::Epochs);
    }
    if !(params.learning_rate.is_finite() && params.learning_rate > 0.0) {
        return Err(UpdateError::LearningRate);
    }
    let layers = model.layers();
    let last = layers.iter().rposition(|l| matches!(l, Layer::Dense { .. })).ok_or(UpdateError::NoDenseLayer)?;
    let (outputs, weights, bias) = match &layers[last] {
        Layer::Dense { outputs, weights, bias } => (*outputs, weights.clone(), bias.clone()),
        _ => return Err(UpdateError::NoDenseLayer),
    };
    for (sample, s) in samples.iter().enumerate() {
        if s.input.len() != model.input_len() {
            return Err(UpdateError::InputLength { sample, expected: model.input_len(), found: s.input.len() });
        }
        if s.label >= outputs {
            return Err(UpdateError::Label { sample, label: s.label, classes: outputs });
        }
    }

    // The frozen layers run once per sample; the trained layer reads their output.
    let features: Vec<Vec<f32>> = samples.iter().map(|s| run_layers(&layers[..last], &s.input)).collect();
    let loss_before = mean_loss(&features, samples, &weights, &bias, outputs);

    let mut w = weights.clone();
    let mut b = bias.clone();
    for _ in 0..params.epochs {
        for (x, s) in features.iter().zip(samples) {
            // The gradient of the loss with respect to each output is softmax(logits) minus the one-hot label.
            let mut grad = dense(x, &w, &b, outputs);
            softmax(&mut grad);
            grad[s.label] -= 1.0;
            for j in 0..outputs {
                b[j] -= params.learning_rate * grad[j];
            }
            for (i, xi) in x.iter().enumerate() {
                for j in 0..outputs {
                    w[i * outputs + j] -= params.learning_rate * xi * grad[j];
                }
            }
        }
    }
    let loss_after = mean_loss(&features, samples, &w, &b, outputs);

    let mut rebuilt = Model::new(model.input_name(), model.input_len())?;
    if let Some(name) = model.output_name() {
        rebuilt = rebuilt.with_output(name);
    }
    for (k, layer) in layers.iter().enumerate() {
        rebuilt = match layer {
            Layer::Dense { outputs, weights, bias } => {
                if k == last {
                    rebuilt.dense(*outputs, w.clone(), b.clone())?
                } else {
                    rebuilt.dense(*outputs, weights.clone(), bias.clone())?
                }
            }
            Layer::Relu => rebuilt.relu(),
            Layer::Softmax => rebuilt.softmax(),
        };
    }
    let report = Report { samples: samples.len(), epochs: params.epochs, loss_before, loss_after };
    Ok((rebuilt, report))
}

/// One dense layer over `x`: `out[j] = bias[j] + sum over i of x[i] * weights[i * outputs + j]`.
fn dense(x: &[f32], weights: &[f32], bias: &[f32], outputs: usize) -> Vec<f32> {
    let mut y = bias.to_vec();
    for (i, xi) in x.iter().enumerate() {
        for j in 0..outputs {
            y[j] += xi * weights[i * outputs + j];
        }
    }
    y
}

/// Runs a chain of frozen layers over `input`, as the model's own evaluator does.
fn run_layers(layers: &[Layer], input: &[f32]) -> Vec<f32> {
    let mut x = input.to_vec();
    for layer in layers {
        x = match layer {
            Layer::Dense { outputs, weights, bias } => dense(&x, weights, bias, *outputs),
            Layer::Relu => x.iter().map(|v| v.max(0.0f32)).collect(),
            Layer::Softmax => {
                softmax(&mut x);
                x
            }
        };
    }
    x
}

/// `exp(x - max) / sum(exp(x - max))`, in place.
fn softmax(v: &mut [f32]) {
    let max = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0f32;
    for x in v.iter_mut() {
        *x = (*x - max).exp();
        sum += *x;
    }
    for x in v.iter_mut() {
        *x /= sum;
    }
}

/// Cross-entropy of one logit vector against its label: `ln(sum(exp(z - max))) + max - z[label]`.
fn loss(logits: &[f32], label: usize) -> f32 {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = logits.iter().map(|z| (*z - max).exp()).sum();
    sum.ln() + max - logits[label]
}

fn mean_loss(features: &[Vec<f32>], samples: &[Sample], weights: &[f32], bias: &[f32], outputs: usize) -> f32 {
    let total: f32 = features
        .iter()
        .zip(samples)
        .map(|(x, s)| loss(&dense(x, weights, bias, outputs), s.label))
        .sum();
    total / samples.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_dense(weights: Vec<f32>, bias: Vec<f32>) -> Model {
        Model::new("x", 2).unwrap().with_output("y").dense(2, weights, bias).unwrap()
    }

    fn sample(input: Vec<f32>, label: usize) -> Sample {
        Sample { input, label }
    }

    #[test]
    fn one_step_moves_weights_and_bias_by_the_softmax_gradient() {
        // Zero weights give logits [0, 0], so p = [0.5, 0.5] and the gradient is [-0.5, 0.5] for label 0.
        // With a rate of 1 and input [1, 0]: bias becomes [0.5, -0.5]; row 0 becomes [0.5, -0.5]; row 1 stays 0.
        let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
        let params = Parameters { learning_rate: 1.0, epochs: 1 };
        let (out, report) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
        match &out.layers()[0] {
            Layer::Dense { weights, bias, .. } => {
                assert_eq!(weights, &vec![0.5, -0.5, 0.0, 0.0]);
                assert_eq!(bias, &vec![0.5, -0.5]);
            }
            other => panic!("expected a dense layer, found {other:?}"),
        }
        assert_eq!(report.samples, 1);
        assert_eq!(report.epochs, 1);
    }

    #[test]
    fn loss_starts_at_ln_two_and_drops_after_training() {
        // Uniform two-class loss is ln 2 = 0.6931. After one step the logits are [1, -1], so the loss is
        // ln(e + 1/e) - 1 = 0.1269.
        let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
        let params = Parameters { learning_rate: 1.0, epochs: 1 };
        let (_, report) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
        assert!((report.loss_before - 0.6931).abs() < 1e-3, "before {}", report.loss_before);
        assert!((report.loss_after - 0.1269).abs() < 1e-3, "after {}", report.loss_after);
    }

    #[test]
    fn a_second_epoch_lowers_the_loss_further() {
        // Second step from logits [1, -1]: p0 = 0.8808, gradient [-0.1192, 0.1192]. Both weights and bias move
        // by 0.1192 more, so the logits are [1.2384, -1.2384] and the loss is about 0.0807.
        let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
        let one = Parameters { learning_rate: 1.0, epochs: 1 };
        let two = Parameters { learning_rate: 1.0, epochs: 2 };
        let samples = [sample(vec![1.0, 0.0], 0)];
        let (_, r1) = update(&model, &samples, &one).unwrap();
        let (_, r2) = update(&model, &samples, &two).unwrap();
        assert!(r2.loss_after < r1.loss_after);
        assert!((r2.loss_after - 0.0807).abs() < 2e-3, "after {}", r2.loss_after);
    }

    #[test]
    fn frozen_layers_before_the_trained_layer_do_not_change() {
        // Identity dense layer and relu run first; the trained layer sees [1, 0] exactly as in the one-step test.
        let model = Model::new("x", 2)
            .unwrap()
            .with_output("y")
            .dense(2, vec![1.0, 0.0, 0.0, 1.0], vec![0.0, 0.0])
            .unwrap()
            .relu()
            .dense(2, vec![0.0; 4], vec![0.0; 2])
            .unwrap();
        let params = Parameters { learning_rate: 1.0, epochs: 1 };
        let (out, _) = update(&model, &[sample(vec![1.0, 0.0], 0)], &params).unwrap();
        assert_eq!(out.layers()[0], model.layers()[0]);
        assert_eq!(out.layers()[1], Layer::Relu);
        match &out.layers()[2] {
            Layer::Dense { weights, bias, .. } => {
                assert_eq!(weights, &vec![0.5, -0.5, 0.0, 0.0]);
                assert_eq!(bias, &vec![0.5, -0.5]);
            }
            other => panic!("expected a dense layer, found {other:?}"),
        }
        assert_eq!(out.input_name(), "x");
        assert_eq!(out.output_name(), Some("y"));
    }

    #[test]
    fn bad_samples_and_parameters_are_rejected() {
        let model = one_dense(vec![0.0; 4], vec![0.0; 2]);
        let good = Parameters::default();
        assert_eq!(update(&model, &[], &good).unwrap_err(), UpdateError::Empty);
        let zero_epochs = Parameters { learning_rate: 0.1, epochs: 0 };
        assert_eq!(update(&model, &[sample(vec![1.0, 0.0], 0)], &zero_epochs).unwrap_err(), UpdateError::Epochs);
        let zero_rate = Parameters { learning_rate: 0.0, epochs: 1 };
        assert_eq!(update(&model, &[sample(vec![1.0, 0.0], 0)], &zero_rate).unwrap_err(), UpdateError::LearningRate);
        let short = [sample(vec![1.0, 0.0], 0), sample(vec![1.0], 1)];
        assert_eq!(
            update(&model, &short, &good).unwrap_err(),
            UpdateError::InputLength { sample: 1, expected: 2, found: 1 }
        );
        let bad_label = [sample(vec![1.0, 0.0], 2)];
        assert_eq!(
            update(&model, &bad_label, &good).unwrap_err(),
            UpdateError::Label { sample: 0, label: 2, classes: 2 }
        );
    }

    #[test]
    fn a_model_without_a_dense_layer_cannot_be_updated() {
        let model = Model::new("x", 2).unwrap().relu();
        assert_eq!(
            update(&model, &[sample(vec![1.0, 0.0], 0)], &Parameters::default()).unwrap_err(),
            UpdateError::NoDenseLayer
        );
    }
}
