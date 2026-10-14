//! The on-device analysis runtime: one table of named models that every analysis request shares.
//!
//! A model is loaded once under a name, then any number of requests run it through [`AnalysisRuntime::predict`].
//! The runtime routes each request to the model it names, counts the requests per model and refuses a name that is
//! not loaded. It keeps no request state between calls, so a request that fails leaves the other models untouched.
//!
//! Not mirrored: model compilation for a compute device, model downloads and the system-wide model cache. Every
//! model runs on the CPU evaluator in [`crate::ml_model`].

use crate::features::{DictionaryProvider, FeatureProvider};
use crate::ml_model::{Model, ModelError};
use std::collections::BTreeMap;
use std::fmt;

/// Why a request could not be routed or run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    /// No model is loaded under the requested name.
    UnknownModel(String),
    /// A model is already loaded under the name. Unload it first.
    AlreadyLoaded(String),
    /// The model refused the input or failed to run.
    Model(ModelError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::UnknownModel(name) => write!(f, "no model is loaded as `{name}`"),
            RuntimeError::AlreadyLoaded(name) => write!(f, "a model is already loaded as `{name}`"),
            RuntimeError::Model(err) => write!(f, "the model failed: {err:?}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// One loaded model and the number of requests routed to it.
#[derive(Debug)]
struct Loaded {
    model: Model,
    requests: u64,
}

/// The table of loaded models. One runtime serves every request of the process.
#[derive(Debug, Default)]
pub struct AnalysisRuntime {
    models: BTreeMap<String, Loaded>,
}

impl AnalysisRuntime {
    /// A runtime with no models loaded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `model` under `name`. A name that is already loaded is refused, so a request never changes model
    /// underneath another request.
    pub fn load(&mut self, name: &str, model: Model) -> Result<(), RuntimeError> {
        if self.models.contains_key(name) {
            return Err(RuntimeError::AlreadyLoaded(name.to_owned()));
        }
        self.models.insert(name.to_owned(), Loaded { model, requests: 0 });
        Ok(())
    }

    /// Removes the model loaded under `name`. Returns false when no model has that name.
    pub fn unload(&mut self, name: &str) -> bool {
        self.models.remove(name).is_some()
    }

    /// Whether a model is loaded under `name`.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.models.contains_key(name)
    }

    /// The names of the loaded models, in alphabetical order.
    pub fn loaded_names(&self) -> Vec<&str> {
        self.models.keys().map(String::as_str).collect()
    }

    /// The number of requests routed to the model under `name`, or `None` when no model has that name.
    pub fn request_count(&self, name: &str) -> Option<u64> {
        self.models.get(name).map(|loaded| loaded.requests)
    }

    /// Runs the model under `name` on `input`. Every request that names a loaded model is counted, whether or not the
    /// model accepts the input.
    pub fn predict(&mut self, name: &str, input: &dyn FeatureProvider) -> Result<DictionaryProvider, RuntimeError> {
        let loaded = self.models.get_mut(name).ok_or_else(|| RuntimeError::UnknownModel(name.to_owned()))?;
        loaded.requests += 1;
        loaded.model.predict(input).map_err(RuntimeError::Model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::FeatureValue;
    use crate::ml_tensor::Tensor;

    fn doubler() -> Model {
        // Two inputs, two outputs: out = input * 2 on the first element, a bias of 0 on the second.
        Model::new("x", 2)
            .unwrap()
            .dense(2, vec![2.0, 0.0, 0.0, 0.0], vec![0.0, 0.0])
            .unwrap()
            .with_output("y")
    }

    fn input(values: &[f32]) -> DictionaryProvider {
        let tensor = Tensor::new(&[values.len()], values.to_vec()).unwrap();
        DictionaryProvider::new().with("x", FeatureValue::Tensor(tensor))
    }

    fn output(provider: &DictionaryProvider) -> Vec<f32> {
        provider.value("y").and_then(FeatureValue::as_tensor).unwrap().values().to_vec()
    }

    #[test]
    fn a_loaded_model_answers_a_request_by_name() {
        let mut runtime = AnalysisRuntime::new();
        runtime.load("doubler", doubler()).unwrap();
        let out = runtime.predict("doubler", &input(&[3.0, 5.0])).unwrap();
        assert_eq!(output(&out), vec![6.0, 0.0]);
    }

    #[test]
    fn a_request_for_a_model_that_is_not_loaded_is_refused() {
        let mut runtime = AnalysisRuntime::new();
        assert_eq!(
            runtime.predict("missing", &input(&[1.0, 1.0])).unwrap_err(),
            RuntimeError::UnknownModel("missing".into())
        );
        assert_eq!(runtime.request_count("missing"), None);
    }

    #[test]
    fn a_name_can_hold_only_one_model_until_it_is_unloaded() {
        let mut runtime = AnalysisRuntime::new();
        runtime.load("doubler", doubler()).unwrap();
        assert_eq!(runtime.load("doubler", doubler()), Err(RuntimeError::AlreadyLoaded("doubler".into())));
        assert!(runtime.unload("doubler"));
        assert!(!runtime.unload("doubler"));
        assert!(runtime.load("doubler", doubler()).is_ok());
    }

    #[test]
    fn requests_are_counted_per_model_including_refused_input() {
        let mut runtime = AnalysisRuntime::new();
        runtime.load("doubler", doubler()).unwrap();
        runtime.load("other", doubler()).unwrap();
        runtime.predict("doubler", &input(&[1.0, 1.0])).unwrap();
        // A wrong input length is refused by the model but still routed to it.
        assert!(runtime.predict("doubler", &input(&[1.0])).is_err());
        assert_eq!(runtime.request_count("doubler"), Some(2));
        assert_eq!(runtime.request_count("other"), Some(0));
    }

    #[test]
    fn loaded_names_are_listed_in_order_and_follow_unloading() {
        let mut runtime = AnalysisRuntime::new();
        runtime.load("sentiment", doubler()).unwrap();
        runtime.load("classifier", doubler()).unwrap();
        assert_eq!(runtime.loaded_names(), vec!["classifier", "sentiment"]);
        assert!(runtime.is_loaded("sentiment"));
        runtime.unload("sentiment");
        assert!(!runtime.is_loaded("sentiment"));
        assert_eq!(runtime.loaded_names(), vec!["classifier"]);
    }
}
