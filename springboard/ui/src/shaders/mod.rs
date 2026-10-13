//! The built-in shader library and the compute path over it. `SOURCE` declares the image kernels the way a library
//! source file does; `library()` scans it with `ShaderLibrary::from_source`, so names and stages come from one place.
//! A kernel is specialised with `FunctionConstantValues` (`MTLFunctionConstantValues`), then `make_compute_pipeline_state`
//! (`MTLDevice.makeComputePipelineState(function:)`) returns a state that `run`s on an image. `ci_filter` maps the
//! Core Image built-in filter names and input keys onto those kernels, and `CiKernel` is `CIKernel`, `CIColorKernel`
//! and `CIWarpKernel` created from a function name in a library.
//!
//! The pixel work is the CPU reference in `image_kernels`. No GPU adapter is used here (the workspace has no GPU
//! compute crate); a GPU back end would plug in behind `ComputePipelineState::run` and be checked against this path.
//! Constants are `f32`; a boolean is 0 or 1. A constant that is not set takes the Core Image default.
use crate::image_kernels::{Kernel, KernelKind};
use crate::shader_library::{LibraryError, ShaderLibrary, Stage};
use bifrost_render::gfx::Image;
use std::collections::BTreeMap;

/// Entry points of the built-in library, one declaration per line.
pub const SOURCE: &str = "\
// Core Image kernels
kernel void ci_gaussian_blur(texture2d<float> src)
kernel void ci_color_controls(texture2d<float> src)
kernel void ci_color_matrix(texture2d<float> src)
kernel void ci_sepia_tone(texture2d<float> src)
kernel void ci_vignette(texture2d<float> src)
kernel void ci_affine_transform(texture2d<float> src)
kernel void ci_lanczos_scale(texture2d<float> src)
kernel void ci_source_over(texture2d<float> fg, texture2d<float> bg)
// Performance shader kernels
kernel void mps_image_convolution(texture2d<float> src)
kernel void mps_image_histogram(texture2d<float> src)
// Compositor passes
vertex float4 quad_vertex(uint vid)
fragment float4 quad_fragment(float4 pos)
";

/// The built-in library.
pub fn library() -> ShaderLibrary {
    let mut lib = ShaderLibrary::from_source(SOURCE).expect("built-in source declares unique valid names");
    lib.label = Some("builtin".to_string());
    lib
}

/// `MTLFunctionConstantValues`: named values a kernel is specialised with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FunctionConstantValues(BTreeMap<String, f32>);

impl FunctionConstantValues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: &str, value: f32) -> &mut Self {
        self.0.insert(name.to_string(), value);
        self
    }

    pub fn set_bool(&mut self, name: &str, value: bool) -> &mut Self {
        self.set(name, if value { 1.0 } else { 0.0 })
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.0.get(name).copied()
    }

    fn or(&self, name: &str, default: f32) -> f32 {
        self.get(name).unwrap_or(default)
    }
}

/// Why a function could not become a pipeline state or a Core Image kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelError {
    Library(LibraryError),
    /// The function is in the library but has no built-in kernel behind it (or the filter name is unknown).
    Unsupported(String),
    /// The function needs more than constants (a second image, weights or a result buffer); call it directly.
    NeedsData(String),
    /// A `CIColorKernel` or `CIWarpKernel` was asked for a function of another class.
    KindMismatch { name: String, wanted: KernelKind, found: KernelKind },
}

impl From<LibraryError> for KernelError {
    fn from(e: LibraryError) -> Self {
        KernelError::Library(e)
    }
}

fn build(name: &str, c: &FunctionConstantValues) -> Result<Kernel, KernelError> {
    Ok(match name {
        "ci_gaussian_blur" => Kernel::GaussianBlur { radius: c.or("radius", 10.0) },
        "ci_color_controls" => Kernel::ColorControls {
            saturation: c.or("saturation", 1.0),
            brightness: c.or("brightness", 0.0),
            contrast: c.or("contrast", 1.0),
        },
        "ci_color_matrix" => {
            let m = |i: usize| c.or(&format!("m{i}"), if i < 16 && i % 5 == 0 { 1.0 } else { 0.0 });
            let mut rows = [[0.0; 4]; 4];
            for (i, row) in rows.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = m(4 * i + j);
                }
            }
            Kernel::ColorMatrix { rows, bias: [m(16), m(17), m(18), m(19)] }
        }
        "ci_sepia_tone" => Kernel::Sepia { intensity: c.or("intensity", 1.0) },
        "ci_vignette" => Kernel::Vignette { intensity: c.or("intensity", 0.0), radius: c.or("radius", 1.0) },
        "ci_affine_transform" => Kernel::AffineTransform {
            m: [c.or("a", 1.0), c.or("b", 0.0), c.or("c", 0.0), c.or("d", 1.0), c.or("tx", 0.0), c.or("ty", 0.0)],
        },
        "ci_lanczos_scale" => Kernel::LanczosScale { scale: c.or("scale", 1.0), aspect_ratio: c.or("aspect_ratio", 1.0) },
        "ci_source_over" | "mps_image_convolution" | "mps_image_histogram" => {
            return Err(KernelError::NeedsData(name.to_string()))
        }
        _ => return Err(KernelError::Unsupported(name.to_string())),
    })
}

/// `MTLComputePipelineState`: a kernel function specialised and ready to run on images.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputePipelineState {
    function: String,
    kernel: Kernel,
}

impl ComputePipelineState {
    pub fn function_name(&self) -> &str {
        &self.function
    }

    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    /// `maxTotalThreadsPerThreadgroup` of the CPU path.
    pub fn max_total_threads_per_threadgroup(&self) -> usize {
        256
    }

    /// `threadExecutionWidth` of the CPU path.
    pub fn thread_execution_width(&self) -> usize {
        8
    }

    /// Dispatches the kernel over every pixel of `img`.
    pub fn run(&self, img: &Image) -> Image {
        self.kernel.run(img)
    }
}

/// `makeComputePipelineState`: looks `name` up in `lib` (it must be a kernel) and specialises it with `constants`.
pub fn make_compute_pipeline_state(
    lib: &ShaderLibrary,
    name: &str,
    constants: &FunctionConstantValues,
) -> Result<ComputePipelineState, KernelError> {
    lib.check_compute_pipeline(name)?;
    Ok(ComputePipelineState { function: name.to_string(), kernel: build(name, constants)? })
}

/// Core Image built-in filter names, the kernel behind each and the input keys it takes (key, constant).
const CI_FILTERS: &[(&str, &str, &[(&str, &str)])] = &[
    ("CIGaussianBlur", "ci_gaussian_blur", &[("inputRadius", "radius")]),
    (
        "CIColorControls",
        "ci_color_controls",
        &[("inputSaturation", "saturation"), ("inputBrightness", "brightness"), ("inputContrast", "contrast")],
    ),
    ("CISepiaTone", "ci_sepia_tone", &[("inputIntensity", "intensity")]),
    ("CIVignette", "ci_vignette", &[("inputIntensity", "intensity"), ("inputRadius", "radius")]),
    ("CILanczosScaleTransform", "ci_lanczos_scale", &[("inputScale", "scale"), ("inputAspectRatio", "aspect_ratio")]),
    (
        "CIAffineTransform",
        "ci_affine_transform",
        &[("inputA", "a"), ("inputB", "b"), ("inputC", "c"), ("inputD", "d"), ("inputTX", "tx"), ("inputTY", "ty")],
    ),
];

/// The Core Image built-in filter names that run on this path (scalar-input filters plus the two special cases).
pub fn ci_filter_names() -> Vec<&'static str> {
    let mut v: Vec<&str> = CI_FILTERS.iter().map(|f| f.0).collect();
    v.extend(["CIColorMatrix", "CISourceOverCompositing"]);
    v
}

/// `CIFilter(name:)` with scalar inputs set by key. Unknown keys are ignored. Routed through the built-in library.
pub fn ci_filter(name: &str, inputs: &[(&str, f32)]) -> Result<ComputePipelineState, KernelError> {
    let (_, function, keys) = CI_FILTERS
        .iter()
        .find(|f| f.0 == name)
        .ok_or_else(|| KernelError::Unsupported(name.to_string()))?;
    let mut constants = FunctionConstantValues::new();
    for (key, value) in inputs {
        if let Some((_, constant)) = keys.iter().find(|k| k.0 == *key) {
            constants.set(constant, *value);
        }
    }
    make_compute_pipeline_state(&library(), function, &constants)
}

/// `CIColorMatrix`: `inputRVector`, `inputGVector`, `inputBVector`, `inputAVector` and `inputBiasVector`.
pub fn ci_color_matrix(r: [f32; 4], g: [f32; 4], b: [f32; 4], a: [f32; 4], bias: [f32; 4]) -> ComputePipelineState {
    ComputePipelineState { function: "ci_color_matrix".to_string(), kernel: Kernel::ColorMatrix { rows: [r, g, b, a], bias } }
}

/// `CIKernel`, `CIColorKernel` or `CIWarpKernel` created with `init(functionName:fromMetalLibraryData:)`.
#[derive(Clone, Debug, PartialEq)]
pub struct CiKernel {
    pub function_name: String,
    pub kind: KernelKind,
    state: ComputePipelineState,
}

impl CiKernel {
    /// Looks `function_name` up in `lib`, builds it with its default constants and checks it is of class `kind`
    /// (`KernelKind::General` accepts any kernel, as a `CIKernel` can wrap any).
    pub fn new(function_name: &str, lib: &ShaderLibrary, kind: KernelKind) -> Result<Self, KernelError> {
        let state = make_compute_pipeline_state(lib, function_name, &FunctionConstantValues::new())?;
        let found = state.kernel().kind();
        if kind != KernelKind::General && kind != found {
            return Err(KernelError::KindMismatch { name: function_name.to_string(), wanted: kind, found });
        }
        Ok(CiKernel { function_name: function_name.to_string(), kind, state })
    }

    /// `apply(extent:arguments:)` on one input image.
    pub fn apply(&self, img: &Image) -> Image {
        self.state.run(img)
    }
}

/// True when `name` is a kernel in the built-in library.
pub fn is_builtin_kernel(name: &str) -> bool {
    library().make_function(name).map(|f| f.stage == Stage::Kernel).unwrap_or(false)
}
