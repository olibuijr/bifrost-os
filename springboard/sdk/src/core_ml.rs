//! Core ML for apps: load a model and run predictions on the device.
//!
//! The model is `bifrost_ui::ml_model::Model`, a chain of dense, relu and softmax layers read from its text form
//! ([`Model::parse`]) or built in code. [`AnalysisRuntime`] holds the named models that an app loads and calls, and
//! [`DictionaryProvider`] carries the feature values in and out. The compute policy (`MLComputePolicy`) is
//! `bifrost_ui::compute::Policy`. This module re-exports that API and adds the placement of a model's work and the
//! top-label helper.
//!
//! Models are compiled once and loaded from the compiled form: [`install_model`] compiles a model's text form
//! (`bifrost_ui::ml_compile`, a checked binary) into a models directory and [`load_installed`] loads it into a runtime,
//! refusing a damaged file before any prediction runs.
//!
//! Limits: model work runs on the CPU evaluator. The GPU draws the screen and is not used for models, and there is
//! no Neural Engine, so [`model_device`] places every policy on [`Device::Cpu`]. Models are not downloaded (the
//! caller hands over the text), and there is no image, speech or language domain API on top of the runtime.

pub use bifrost_ui::compute::{place, Device, Policy};
pub use bifrost_ui::features::{DictionaryProvider, FeatureProvider, FeatureValue, Kind};
pub use bifrost_ui::ml_compile::{compile, compile_text, decompile, CompileError, EXTENSION as COMPILED_EXTENSION};
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

/// Why a model could not be installed or loaded from the models directory.
#[derive(Debug)]
pub enum InstallError {
    /// The name is empty or uses anything beyond letters, digits, `.`, `-` and `_`, so it cannot be a file name.
    BadName,
    Compile(CompileError),
    Runtime(RuntimeError),
    Io(std::io::Error),
}

fn compiled_path(dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf, InstallError> {
    let safe = !name.is_empty()
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !safe {
        return Err(InstallError::BadName);
    }
    Ok(dir.join(format!("{name}.{COMPILED_EXTENSION}")))
}

/// Compiles the text form of a model and stores it as `<dir>/<name>.bmlc`, replacing an older version in one rename.
/// A text that does not compile leaves the directory untouched.
pub fn install_model(dir: &std::path::Path, name: &str, text: &str) -> Result<std::path::PathBuf, InstallError> {
    let path = compiled_path(dir, name)?;
    let bytes = compile_text(text).map_err(InstallError::Compile)?;
    std::fs::create_dir_all(dir).map_err(InstallError::Io)?;
    let tmp = path.with_extension(format!("{COMPILED_EXTENSION}.tmp"));
    std::fs::write(&tmp, bytes).map_err(InstallError::Io)?;
    std::fs::rename(&tmp, &path).map_err(InstallError::Io)?;
    Ok(path)
}

/// Loads the compiled model `<dir>/<name>.bmlc` into `runtime` under `name`.
pub fn load_installed(runtime: &mut AnalysisRuntime, dir: &std::path::Path, name: &str) -> Result<(), InstallError> {
    let bytes = std::fs::read(compiled_path(dir, name)?).map_err(InstallError::Io)?;
    let model = decompile(&bytes).map_err(InstallError::Compile)?;
    runtime.load(name, model).map_err(InstallError::Runtime)
}

/// The names of the compiled models in `dir`, in alphabetical order. A missing directory has none.
pub fn installed_models(dir: &std::path::Path) -> Vec<String> {
    let suffix = format!(".{COMPILED_EXTENSION}");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    names.retain(|n| n.ends_with(&suffix) && n.len() > suffix.len());
    let mut names: Vec<String> = names.into_iter().map(|n| n[..n.len() - suffix.len()].to_string()).collect();
    names.sort();
    names
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

    fn models_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-core-ml-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    const TEXT: &str = "input x 2\ndense 2 2,0,0,0 0,0\nsoftmax\noutput y\n";

    #[test]
    fn an_installed_model_loads_and_predicts_like_the_built_one() {
        let dir = models_dir("install");
        let path = install_model(&dir, "doubler", TEXT).unwrap();
        assert!(path.ends_with("doubler.bmlc"));
        let mut runtime = AnalysisRuntime::new();
        load_installed(&mut runtime, &dir, "doubler").unwrap();
        let input = DictionaryProvider::new().with("x", FeatureValue::Tensor(Tensor::new(&[2], vec![1.0, 0.0]).unwrap()));
        let out = runtime.predict("doubler", &input).unwrap();
        let (index, _) = top_index(out.value("y").and_then(FeatureValue::as_tensor).unwrap()).unwrap();
        assert_eq!(index, 0);
        assert_eq!(installed_models(&dir), vec!["doubler".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_text_that_does_not_compile_installs_nothing() {
        let dir = models_dir("bad-text");
        assert!(matches!(install_model(&dir, "broken", "input x\n"), Err(InstallError::Compile(_))));
        assert!(installed_models(&dir).is_empty());
    }

    #[test]
    fn names_that_are_not_file_names_are_refused() {
        let dir = models_dir("names");
        for bad in ["", "../escape", "a/b", ".hidden", "sp ace"] {
            assert!(matches!(install_model(&dir, bad, TEXT), Err(InstallError::BadName)), "{bad:?}");
        }
    }

    #[test]
    fn a_damaged_compiled_file_is_refused_at_load() {
        let dir = models_dir("damaged");
        let path = install_model(&dir, "m", TEXT).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();
        let mut runtime = AnalysisRuntime::new();
        assert!(matches!(load_installed(&mut runtime, &dir, "m"), Err(InstallError::Compile(CompileError::Checksum))));
        assert!(!runtime.is_loaded("m"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_twice_is_refused_by_the_runtime_and_a_missing_file_is_io() {
        let dir = models_dir("twice");
        install_model(&dir, "m", TEXT).unwrap();
        let mut runtime = AnalysisRuntime::new();
        load_installed(&mut runtime, &dir, "m").unwrap();
        assert!(matches!(load_installed(&mut runtime, &dir, "m"), Err(InstallError::Runtime(RuntimeError::AlreadyLoaded(_)))));
        assert!(matches!(load_installed(&mut runtime, &dir, "absent"), Err(InstallError::Io(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
