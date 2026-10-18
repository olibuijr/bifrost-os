//! Core ML for apps: load a model and run predictions on the device.
//!
//! The model is `bifrost_ui::ml_model::Model`, a chain of dense, relu and softmax layers read from its text form
//! ([`Model::parse`]) or built in code. [`AnalysisRuntime`] holds the named models that an app loads and calls, and
//! [`DictionaryProvider`] carries the feature values in and out. The compute policy (`MLComputePolicy`) is
//! `bifrost_ui::compute::Policy`. This module re-exports that API and adds the placement of a model's work and the
//! top-label helper.
//!
//! Limits: model work runs on the CPU evaluator. The GPU draws the screen and is not used for models, and there is
//! no Neural Engine, so [`model_device`] places every policy on [`Device::Cpu`]. Models are not downloaded or
//! compiled, and there is no image, speech or language domain API on top of the runtime.

pub use bifrost_ui::compute::{place, Device, Policy};
pub use bifrost_ui::features::{DictionaryProvider, FeatureProvider, FeatureValue, Kind};
pub use bifrost_ui::ml_model::{Layer, Model, ModelError};
pub use bifrost_ui::ml_runtime::{AnalysisRuntime, RuntimeError};
pub use bifrost_ui::ml_tensor::{Tensor, TensorError};

/// The devices model work can run on. The CPU evaluator is the only one.
pub const MODEL_DEVICES: [Device; 1] = [Device::Cpu];

/// The device a model's work runs on under `policy`. Every policy resolves to the CPU here, so `None` is never
/// returned on this platform.
pub fn model_device(policy: Policy) -> Option<Device> {
    place(policy, &MODEL_DEVICES, &MODEL_DEVICES)
}

/// The index and value of the largest element of a prediction's output. The first of equal maxima wins. `None` for an
/// empty tensor or one with a NaN (a NaN cannot be ranked).
pub fn top_index(output: &Tensor) -> Option<(usize, f32)> {
    let values = output.values();
    if values.is_empty() || values.iter().any(|v| v.is_nan()) {
        return None;
    }
    values.iter().copied().enumerate().fold(None, |best: Option<(usize, f32)>, (i, v)| match best {
        Some((_, b)) if b >= v => best,
        _ => Some((i, v)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn softmax_model() -> Model {
        // Two inputs, two outputs: the first output doubles the first input, the second is always zero.
        Model::new("x", 2)
            .unwrap()
            .dense(2, vec![2.0, 0.0, 0.0, 0.0], vec![0.0, 0.0])
            .unwrap()
            .softmax()
            .with_output("y")
    }

    #[test]
    fn every_policy_places_model_work_on_the_cpu() {
        for policy in [Policy::Automatic, Policy::CpuOnly] {
            assert_eq!(model_device(policy), Some(Device::Cpu));
        }
    }

    #[test]
    fn top_index_picks_the_largest_and_the_first_of_equals() {
        let t = Tensor::new(&[4], vec![0.1, 0.7, 0.7, 0.2]).unwrap();
        assert_eq!(top_index(&t), Some((1, 0.7)));
        assert_eq!(top_index(&Tensor::new(&[0], vec![]).unwrap()), None);
        assert_eq!(top_index(&Tensor::new(&[2], vec![0.0, f32::NAN]).unwrap()), None);
    }

    #[test]
    fn a_loaded_model_runs_through_the_runtime_and_ranks_its_output() {
        let mut runtime = AnalysisRuntime::new();
        runtime.load("doubler", softmax_model()).unwrap();
        let input = DictionaryProvider::new().with("x", FeatureValue::Tensor(Tensor::new(&[2], vec![1.0, 0.0]).unwrap()));
        let out = runtime.predict("doubler", &input).unwrap();
        let tensor = out.value("y").and_then(FeatureValue::as_tensor).unwrap();
        let (index, p) = top_index(tensor).unwrap();
        assert_eq!(index, 0);
        assert!(p > 0.85 && p < 1.0, "softmax of [2, 0] is about 0.88, got {p}");
        assert_eq!(runtime.request_count("doubler"), Some(1));
    }

    #[test]
    fn an_unloaded_name_is_refused() {
        let mut runtime = AnalysisRuntime::new();
        let input = DictionaryProvider::new();
        assert_eq!(runtime.predict("missing", &input), Err(RuntimeError::UnknownModel("missing".into())));
    }
}
