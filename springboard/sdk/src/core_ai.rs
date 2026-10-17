//! Core AI for apps: run a model on the device as a typed inference function.
//!
//! An [`InferenceSession`] holds the models an app loaded under a name. Loading a model compiles it for the session's
//! target once and keeps the plan in a specialization cache (`bifrost_ui::model_compile`, `model_specialization`), so
//! loading the same model again, or after a cache purge of a persistent entry, does not compile twice. [`InferenceSession::call`]
//! checks the call against the function's signature (`crate::inference::InferenceFunctionDescriptor`), converts the
//! float32 arrays to tensors, runs the CPU evaluator (`bifrost_ui::ml_model::Model::predict`) and returns the output as
//! a float32 array.
//!
//! Limits: models are chains of dense, relu and softmax layers; they run on the CPU. There is no Neural Engine on this
//! platform. Image values are accepted by the signature types but a model here takes one float32 array input, so an
//! image is turned into an array first (`ImageDescriptor::to_ndarray`).

use std::collections::BTreeMap;
use std::fmt;

use bifrost_ui::features::{DictionaryProvider, FeatureProvider, FeatureValue};
use bifrost_ui::ml_model::{Layer, Model, ModelError};
use bifrost_ui::ml_tensor::{Tensor, TensorError};
use bifrost_ui::model_compile::{self, CompileError, Graph, Op};
pub use bifrost_ui::model_compile::Target;
pub use bifrost_ui::model_specialization::Policy;
use bifrost_ui::model_specialization::SpecializationCache;

use crate::inference::{InferenceError, InferenceFunctionDescriptor, InferenceValue, ValueDescriptor};
use crate::ndarray::{NDArray, NDArrayDescriptor, NDArrayError, ScalarType};

/// Why a model could not be loaded or a call could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum CoreAiError {
    /// The call does not match the function's signature.
    Inference(InferenceError),
    /// The model could not be read or run.
    Model(ModelError),
    /// The model could not be compiled for the target.
    Compile(CompileError),
    /// A tensor could not be made from an input array.
    Tensor(TensorError),
    /// An output array could not be made from the prediction.
    Array(NDArrayError),
    /// No model is loaded under the name.
    UnknownModel(String),
    /// A model is already loaded under the name.
    AlreadyLoaded(String),
}

impl fmt::Display for CoreAiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreAiError::Inference(e) => write!(f, "{e}"),
            CoreAiError::Model(e) => write!(f, "model error: {e:?}"),
            CoreAiError::Compile(e) => write!(f, "compile error: {e:?}"),
            CoreAiError::Tensor(e) => write!(f, "tensor error: {e:?}"),
            CoreAiError::Array(e) => write!(f, "{e}"),
            CoreAiError::UnknownModel(name) => write!(f, "no model loaded as `{name}`"),
            CoreAiError::AlreadyLoaded(name) => write!(f, "a model is already loaded as `{name}`"),
        }
    }
}

impl std::error::Error for CoreAiError {}

/// A model with the inference function it exposes and the graph the compiler takes.
#[derive(Clone, Debug)]
pub struct InferenceModel {
    name: String,
    model: Model,
    function: InferenceFunctionDescriptor,
    graph: Graph,
    output: String,
}

impl InferenceModel {
    /// Wraps a model. Its function takes the model's input under the model's input name, as a float32 array of
    /// `input_len` elements, and returns the model's output as a float32 array of `output_len` elements.
    pub fn new(name: &str, model: Model) -> Result<Self, CoreAiError> {
        let output = model.output_name().ok_or(CoreAiError::Model(ModelError::Incomplete))?.to_owned();
        let mut function = InferenceFunctionDescriptor::new(name);
        let input_shape = NDArrayDescriptor::new(&[model.input_len()], ScalarType::Float32).map_err(CoreAiError::Array)?;
        let output_shape = NDArrayDescriptor::new(&[model.output_len()], ScalarType::Float32).map_err(CoreAiError::Array)?;
        function.add_input(model.input_name(), ValueDescriptor::Array(input_shape)).map_err(CoreAiError::Inference)?;
        function.add_output(&output, ValueDescriptor::Array(output_shape)).map_err(CoreAiError::Inference)?;
        let ops = model
            .layers()
            .iter()
            .map(|layer| match layer {
                Layer::Dense { outputs, .. } => Op::Dense { outputs: *outputs as u32 },
                Layer::Relu => Op::Relu,
                Layer::Softmax => Op::Softmax,
            })
            .collect();
        let graph = Graph { name: name.to_owned(), input_width: model.input_len() as u32, ops };
        Ok(InferenceModel { name: name.to_owned(), model, function, graph, output })
    }

    /// Reads the model from its text form (`Model::parse`) and wraps it.
    pub fn from_text(name: &str, text: &str) -> Result<Self, CoreAiError> {
        let model = Model::parse(text).map_err(CoreAiError::Model)?;
        Self::new(name, model)
    }

    /// The name the model is loaded under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The function's signature: named inputs and outputs with their shapes.
    pub fn function(&self) -> &InferenceFunctionDescriptor {
        &self.function
    }

    /// The description the compiler takes.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }
}

/// The models an app has loaded, their specialized plans and the calls made on them.
pub struct InferenceSession {
    target: Target,
    policy: Policy,
    cache: SpecializationCache,
    models: BTreeMap<String, InferenceModel>,
    calls: BTreeMap<String, u64>,
}

impl InferenceSession {
    /// A session that compiles for `target` and keeps plans under `policy`. Purgeable plans together may cost at most
    /// `cache_limit` bytes.
    pub fn new(target: Target, policy: Policy, cache_limit: usize) -> Self {
        InferenceSession {
            target,
            policy,
            cache: SpecializationCache::new(cache_limit),
            models: BTreeMap::new(),
            calls: BTreeMap::new(),
        }
    }

    /// Loads a model and makes sure its plan is in the cache. Returns whether this call compiled the plan; a plan that
    /// is already cached is reused. A name that is already loaded is refused.
    pub fn load(&mut self, model: InferenceModel) -> Result<bool, CoreAiError> {
        if self.models.contains_key(model.name()) {
            return Err(CoreAiError::AlreadyLoaded(model.name().to_owned()));
        }
        let compiled = model_compile::prepare(&mut self.cache, &model.graph, self.target, self.policy)
            .map_err(CoreAiError::Compile)?;
        self.calls.insert(model.name().to_owned(), 0);
        self.models.insert(model.name().to_owned(), model);
        Ok(compiled)
    }

    /// Unloads a model and deletes its cached plans. Returns whether the model was loaded.
    pub fn unload(&mut self, name: &str) -> bool {
        self.calls.remove(name);
        self.cache.delete_entries_for(name);
        self.models.remove(name).is_some()
    }

    /// The names of the loaded models, sorted.
    pub fn loaded(&self) -> Vec<&str> {
        self.models.keys().map(String::as_str).collect()
    }

    /// The signature of a loaded model's function.
    pub fn function(&self, name: &str) -> Option<&InferenceFunctionDescriptor> {
        self.models.get(name).map(InferenceModel::function)
    }

    /// The compiled plan of a loaded model for this session's target, if the cache still holds it.
    pub fn plan(&mut self, name: &str) -> Option<&[u8]> {
        self.cache.lookup(name, self.target.name())
    }

    /// Drops every purgeable plan (memory pressure). Persistent plans stay. Returns how many were dropped.
    pub fn purge_plans(&mut self) -> usize {
        self.cache.purge()
    }

    /// How many calls ran on a loaded model.
    pub fn call_count(&self, name: &str) -> Option<u64> {
        self.calls.get(name).copied()
    }

    /// Runs the function of a loaded model. The inputs are checked against the signature first; a mismatch runs
    /// nothing and counts no call. A purged plan is compiled again before the model runs.
    pub fn call(&mut self, name: &str, inputs: &[(&str, InferenceValue)]) -> Result<Vec<(String, InferenceValue)>, CoreAiError> {
        let entry = self.models.get(name).ok_or_else(|| CoreAiError::UnknownModel(name.to_owned()))?;
        entry.function.check_inputs(inputs).map_err(CoreAiError::Inference)?;
        let mut provider = DictionaryProvider::new();
        for (input_name, value) in inputs {
            if let InferenceValue::Array(array) = value {
                let tensor = Tensor::new(array.shape(), array.scalars().to_vec()).map_err(CoreAiError::Tensor)?;
                provider = provider.with(input_name, FeatureValue::Tensor(tensor));
            }
        }
        let prediction = entry.model.predict(&provider).map_err(CoreAiError::Model)?;
        let tensor = prediction
            .value(&entry.output)
            .and_then(FeatureValue::as_tensor)
            .ok_or(CoreAiError::Model(ModelError::Incomplete))?;
        let array = NDArray::new(tensor.values().to_vec(), tensor.shape()).map_err(CoreAiError::Array)?;
        let result = vec![(entry.output.clone(), InferenceValue::Array(array))];
        // The plan is a cache of work, not a requirement: make sure it is there for the next call.
        let graph = entry.graph.clone();
        model_compile::prepare(&mut self.cache, &graph, self.target, self.policy).map_err(CoreAiError::Compile)?;
        *self.calls.entry(name.to_owned()).or_insert(0) += 1;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{ImageDescriptor, PixelFormat};

    const DOUBLER: &str = "input x 2\ndense 2 2,0,0,0 0,0\nsoftmax\noutput y\n";

    fn array(values: Vec<f32>) -> InferenceValue {
        let len = values.len();
        InferenceValue::Array(NDArray::new(values, &[len]).unwrap())
    }

    fn session() -> InferenceSession {
        InferenceSession::new(Target::Cpu, Policy::Purgeable, 4096)
    }

    #[test]
    fn a_model_loads_compiles_once_and_runs_through_its_signature() {
        let mut s = session();
        assert_eq!(s.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()), Ok(true));
        assert_eq!(s.loaded(), vec!["doubler"]);
        assert!(s.plan("doubler").is_some());
        let function = s.function("doubler").unwrap();
        assert_eq!(function.input_names(), vec!["x"]);
        assert_eq!(function.output_names(), vec!["y"]);
        let out = s.call("doubler", &[("x", array(vec![1.0, 0.0]))]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "y");
        match &out[0].1 {
            InferenceValue::Array(a) => {
                assert_eq!(a.shape(), &[2]);
                let p = a.scalars()[0];
                assert!(p > 0.85 && p < 1.0, "softmax of [2, 0] is about 0.88, got {p}");
            }
            other => panic!("expected an array, got {other:?}"),
        }
        assert_eq!(s.call_count("doubler"), Some(1));
    }

    #[test]
    fn a_call_that_breaks_the_signature_runs_nothing() {
        let mut s = session();
        s.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()).unwrap();
        // Wrong length, unknown name, missing input.
        let short = s.call("doubler", &[("x", array(vec![1.0]))]);
        assert!(matches!(short, Err(CoreAiError::Inference(InferenceError::TypeMismatch { .. }))));
        let unknown = s.call("doubler", &[("z", array(vec![1.0, 0.0]))]);
        assert_eq!(unknown, Err(CoreAiError::Inference(InferenceError::UnknownInput("z".into()))));
        let missing = s.call("doubler", &[]);
        assert_eq!(missing, Err(CoreAiError::Inference(InferenceError::MissingInput("x".into()))));
        assert_eq!(s.call_count("doubler"), Some(0));
    }

    #[test]
    fn an_image_value_is_refused_where_an_array_is_declared() {
        let mut s = session();
        s.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()).unwrap();
        let descriptor = ImageDescriptor::new(1, 2, PixelFormat::Gray8).unwrap();
        let image = InferenceValue::image(descriptor, vec![0, 255]).unwrap();
        assert!(matches!(
            s.call("doubler", &[("x", image)]),
            Err(CoreAiError::Inference(InferenceError::TypeMismatch { .. }))
        ));
        // The same pixels as a float array run.
        let as_array = descriptor.to_ndarray(&[0, 255], 1.0 / 255.0, 0.0).unwrap().reshaped(&[2]).unwrap();
        assert!(s.call("doubler", &[("x", InferenceValue::Array(as_array))]).is_ok());
    }

    #[test]
    fn a_second_load_under_the_name_is_refused_and_unload_clears_the_plan() {
        let mut s = session();
        s.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()).unwrap();
        assert_eq!(
            s.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()),
            Err(CoreAiError::AlreadyLoaded("doubler".into()))
        );
        assert!(s.unload("doubler"));
        assert!(!s.unload("doubler"));
        assert!(s.plan("doubler").is_none());
        assert_eq!(s.call("doubler", &[]), Err(CoreAiError::UnknownModel("doubler".into())));
    }

    #[test]
    fn a_persistent_plan_survives_a_purge_and_a_purgeable_one_is_compiled_again() {
        let mut persistent = InferenceSession::new(Target::Cpu, Policy::Persistent, 4096);
        persistent.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()).unwrap();
        assert_eq!(persistent.purge_plans(), 0);
        assert!(persistent.plan("doubler").is_some());

        let mut purgeable = session();
        purgeable.load(InferenceModel::from_text("doubler", DOUBLER).unwrap()).unwrap();
        assert_eq!(purgeable.purge_plans(), 1);
        assert!(purgeable.plan("doubler").is_none());
        // The next call compiles the plan again and runs.
        purgeable.call("doubler", &[("x", array(vec![0.0, 1.0]))]).unwrap();
        assert!(purgeable.plan("doubler").is_some());
    }

    #[test]
    fn a_model_without_layers_or_output_cannot_be_loaded() {
        assert_eq!(
            InferenceModel::new("bare", Model::new("x", 2).unwrap()).map(|_| ()),
            Err(CoreAiError::Model(ModelError::Incomplete))
        );
        let no_layers = Model::new("x", 2).unwrap().with_output("y");
        let model = InferenceModel::new("empty", no_layers).unwrap();
        assert_eq!(session().load(model), Err(CoreAiError::Compile(CompileError::NoOperators)));
    }
}
