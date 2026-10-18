//! Typed inference signatures: `InferenceFunctionDescriptor`, `InferenceValue` and `ImageDescriptor`.
//!
//! A model exposes named inputs and outputs, each described by an array shape or an image size. [`InferenceFunctionDescriptor::check_inputs`]
//! rejects a call whose values do not match that signature before anything runs. This module runs no model: it has
//! no compute stream, no state descriptors, and only float32 arrays as values. Pixel data reaches a float32 array
//! through [`ImageDescriptor::to_ndarray`].

use crate::ndarray::{NDArray, NDArrayDescriptor, NDArrayError};
use std::fmt;

/// The pixel layouts an image value can carry. Pixels are packed rows, top to bottom, channels interleaved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Gray8,
    Rgb8,
    Rgba8,
}

impl PixelFormat {
    /// Bytes per pixel, one per channel.
    pub fn channels(self) -> usize {
        match self {
            PixelFormat::Gray8 => 1,
            PixelFormat::Rgb8 => 3,
            PixelFormat::Rgba8 => 4,
        }
    }
}

/// `ImageDescriptor`: an image's width, height and pixel format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDescriptor {
    width: usize,
    height: usize,
    pixel_format: PixelFormat,
}

impl ImageDescriptor {
    /// Both sides must be non-zero and the packed byte count must be representable.
    pub fn new(width: usize, height: usize, pixel_format: PixelFormat) -> Result<Self, InferenceError> {
        let bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(pixel_format.channels()));
        match bytes {
            Some(_) if width > 0 && height > 0 => Ok(Self { width, height, pixel_format }),
            _ => Err(InferenceError::InvalidImage { width, height }),
        }
    }

    /// `width`, in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// `height`, in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The pixel format.
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Bytes of one packed image.
    pub fn minimum_byte_count(&self) -> usize {
        self.width * self.height * self.pixel_format.channels()
    }

    /// Converts packed 8-bit pixels to a float32 array of shape `[height, width, channels]`, mapping each byte `b`
    /// to `b * scale + bias`. For example, `scale = 1/255, bias = 0` gives the usual unit range.
    pub fn to_ndarray(&self, pixels: &[u8], scale: f32, bias: f32) -> Result<NDArray<f32>, InferenceError> {
        if pixels.len() != self.minimum_byte_count() {
            return Err(InferenceError::ImageBytes { expected: self.minimum_byte_count(), got: pixels.len() });
        }
        let scalars: Vec<f32> = pixels.iter().map(|&b| f32::from(b) * scale + bias).collect();
        let shape = [self.height, self.width, self.pixel_format.channels()];
        NDArray::new(scalars, &shape).map_err(InferenceError::Array)
    }
}

/// What an input or output carries: an array's shape or an image's size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueDescriptor {
    Array(NDArrayDescriptor),
    Image(ImageDescriptor),
}

/// `InferenceValue`: one input to or output of an inference function.
#[derive(Clone, Debug, PartialEq)]
pub enum InferenceValue {
    /// A float32 multidimensional array (`ndArray`).
    Array(NDArray<f32>),
    /// Packed pixels with their descriptor (`pixelBuffer`).
    Image { descriptor: ImageDescriptor, pixels: Vec<u8> },
}

impl InferenceValue {
    /// An image value. The pixel count must match the descriptor.
    pub fn image(descriptor: ImageDescriptor, pixels: Vec<u8>) -> Result<Self, InferenceError> {
        if pixels.len() != descriptor.minimum_byte_count() {
            return Err(InferenceError::ImageBytes { expected: descriptor.minimum_byte_count(), got: pixels.len() });
        }
        Ok(InferenceValue::Image { descriptor, pixels })
    }

    /// The shape or size this value has.
    pub fn descriptor(&self) -> ValueDescriptor {
        match self {
            InferenceValue::Array(array) => ValueDescriptor::Array(array.descriptor().clone()),
            InferenceValue::Image { descriptor, .. } => ValueDescriptor::Image(*descriptor),
        }
    }
}

/// Errors from building a signature or checking a call against it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceError {
    /// Two inputs, or two outputs, share a name.
    DuplicateName(String),
    /// The same input is given twice in one call.
    DuplicateInput(String),
    /// The call names an input the function does not declare.
    UnknownInput(String),
    /// A declared input is absent from the call.
    MissingInput(String),
    /// The value's shape, scalar type or image size differs from the declared one.
    TypeMismatch { name: String, expected: ValueDescriptor, got: ValueDescriptor },
    /// An image with a zero side or an unrepresentable size.
    InvalidImage { width: usize, height: usize },
    /// Packed pixel bytes do not match the image size.
    ImageBytes { expected: usize, got: usize },
    /// An array error.
    Array(NDArrayError),
}

impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InferenceError::DuplicateName(name) => write!(f, "duplicate name `{name}`"),
            InferenceError::DuplicateInput(name) => write!(f, "input `{name}` is given twice"),
            InferenceError::UnknownInput(name) => write!(f, "no input named `{name}`"),
            InferenceError::MissingInput(name) => write!(f, "missing input `{name}`"),
            InferenceError::TypeMismatch { name, expected, got } => {
                write!(f, "input `{name}` expects {expected:?}, got {got:?}")
            }
            InferenceError::InvalidImage { width, height } => write!(f, "invalid image size {width}x{height}"),
            InferenceError::ImageBytes { expected, got } => write!(f, "expected {expected} image bytes, got {got}"),
            InferenceError::Array(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for InferenceError {}

/// `InferenceFunctionDescriptor`: a function's name and the descriptors of its inputs and outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct InferenceFunctionDescriptor {
    name: String,
    inputs: Vec<(String, ValueDescriptor)>,
    outputs: Vec<(String, ValueDescriptor)>,
}

impl InferenceFunctionDescriptor {
    /// An empty signature with the given `name`.
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), inputs: Vec::new(), outputs: Vec::new() }
    }

    /// `name`: the function's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Declares an input. Input names are unique.
    pub fn add_input(&mut self, name: &str, descriptor: ValueDescriptor) -> Result<(), InferenceError> {
        if self.inputs.iter().any(|(n, _)| n == name) {
            return Err(InferenceError::DuplicateName(name.to_string()));
        }
        self.inputs.push((name.to_string(), descriptor));
        Ok(())
    }

    /// Declares an output. Output names are unique.
    pub fn add_output(&mut self, name: &str, descriptor: ValueDescriptor) -> Result<(), InferenceError> {
        if self.outputs.iter().any(|(n, _)| n == name) {
            return Err(InferenceError::DuplicateName(name.to_string()));
        }
        self.outputs.push((name.to_string(), descriptor));
        Ok(())
    }

    /// `inputCount`.
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// `inputNames`, in declaration order.
    pub fn input_names(&self) -> Vec<&str> {
        self.inputs.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// `inputDescriptor(of:)`.
    pub fn input_descriptor(&self, name: &str) -> Option<&ValueDescriptor> {
        self.inputs.iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }

    /// `outputCount`.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// `outputNames`, in declaration order.
    pub fn output_names(&self) -> Vec<&str> {
        self.outputs.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// `outputDescriptor(of:)`.
    pub fn output_descriptor(&self, name: &str) -> Option<&ValueDescriptor> {
        self.outputs.iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }

    /// Checks a call against the signature before any model runs. Every declared input must appear exactly once, no
    /// name may be unknown, and each value must match its descriptor exactly.
    pub fn check_inputs(&self, values: &[(&str, InferenceValue)]) -> Result<(), InferenceError> {
        for (i, (name, value)) in values.iter().enumerate() {
            if values[..i].iter().any(|(other, _)| other == name) {
                return Err(InferenceError::DuplicateInput(name.to_string()));
            }
            let expected = self
                .input_descriptor(name)
                .ok_or_else(|| InferenceError::UnknownInput(name.to_string()))?;
            let got = value.descriptor();
            if got != *expected {
                return Err(InferenceError::TypeMismatch {
                    name: name.to_string(),
                    expected: expected.clone(),
                    got,
                });
            }
        }
        for (name, _) in &self.inputs {
            if !values.iter().any(|(given, _)| *given == name.as_str()) {
                return Err(InferenceError::MissingInput(name.clone()));
            }
        }
        Ok(())
    }
}

/// A processor a model may run on. The PC has a CPU and, when the machine has one, a GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputeUnit {
    Cpu,
    Gpu,
}

/// The units a model is allowed to use (`MLComputeUnits`). The PC has no Neural Engine, so the options that name it
/// resolve to the units this machine has (see [`ComputeUnits::usable`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputeUnits {
    CpuOnly,
    CpuAndGpu,
    /// Every unit the machine has; with no Neural Engine here this is the CPU and the GPU.
    All,
}

impl ComputeUnits {
    /// The units that may run the model, CPU first, given whether a GPU is present. The CPU is always there, so a
    /// machine without a GPU runs the model on the CPU whatever was asked for.
    pub fn usable(self, gpu_available: bool) -> Vec<ComputeUnit> {
        match self {
            ComputeUnits::CpuOnly => vec![ComputeUnit::Cpu],
            ComputeUnits::CpuAndGpu | ComputeUnits::All if gpu_available => vec![ComputeUnit::Cpu, ComputeUnit::Gpu],
            ComputeUnits::CpuAndGpu | ComputeUnits::All => vec![ComputeUnit::Cpu],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ndarray::ScalarType;

    #[test]
    fn the_cpu_is_always_a_compute_unit_and_the_gpu_only_when_present() {
        assert_eq!(ComputeUnits::CpuOnly.usable(true), vec![ComputeUnit::Cpu]);
        assert_eq!(ComputeUnits::CpuAndGpu.usable(true), vec![ComputeUnit::Cpu, ComputeUnit::Gpu]);
        assert_eq!(ComputeUnits::All.usable(true), vec![ComputeUnit::Cpu, ComputeUnit::Gpu]);
        assert_eq!(ComputeUnits::CpuAndGpu.usable(false), vec![ComputeUnit::Cpu]);
        assert_eq!(ComputeUnits::All.usable(false), vec![ComputeUnit::Cpu]);
    }

    fn classifier() -> InferenceFunctionDescriptor {
        let mut f = InferenceFunctionDescriptor::new("classify");
        let image = ImageDescriptor::new(2, 2, PixelFormat::Rgb8).unwrap();
        f.add_input("pixels", ValueDescriptor::Image(image)).unwrap();
        let mask = NDArrayDescriptor::new(&[1, 3], ScalarType::Float32).unwrap();
        f.add_input("prior", ValueDescriptor::Array(mask)).unwrap();
        let scores = NDArrayDescriptor::new(&[1, 4], ScalarType::Float32).unwrap();
        f.add_output("scores", ValueDescriptor::Array(scores)).unwrap();
        f
    }

    fn prior() -> InferenceValue {
        InferenceValue::Array(NDArray::new(vec![0.1, 0.2, 0.7], &[1, 3]).unwrap())
    }

    fn pixels() -> InferenceValue {
        let image = ImageDescriptor::new(2, 2, PixelFormat::Rgb8).unwrap();
        InferenceValue::image(image, vec![0; 12]).unwrap()
    }

    #[test]
    fn check_inputs_accepts_a_matching_call() {
        let f = classifier();
        assert_eq!(f.input_names(), vec!["pixels", "prior"]);
        assert_eq!(f.output_names(), vec!["scores"]);
        assert!(f.check_inputs(&[("prior", prior()), ("pixels", pixels())]).is_ok());
    }

    #[test]
    fn check_inputs_rejects_a_shape_mismatch() {
        let f = classifier();
        let wide = InferenceValue::Array(NDArray::new(vec![0.0; 4], &[1, 4]).unwrap());
        let err = f.check_inputs(&[("prior", wide), ("pixels", pixels())]).unwrap_err();
        assert!(matches!(err, InferenceError::TypeMismatch { ref name, .. } if name == "prior"));
    }

    #[test]
    fn check_inputs_reports_missing_and_unknown_inputs() {
        let f = classifier();
        assert_eq!(
            f.check_inputs(&[("prior", prior())]),
            Err(InferenceError::MissingInput("pixels".to_string()))
        );
        assert_eq!(
            f.check_inputs(&[("prior", prior()), ("pixels", pixels()), ("extra", prior())]),
            Err(InferenceError::UnknownInput("extra".to_string()))
        );
    }

    #[test]
    fn check_inputs_rejects_a_repeated_input() {
        let f = classifier();
        assert_eq!(
            f.check_inputs(&[("prior", prior()), ("prior", prior()), ("pixels", pixels())]),
            Err(InferenceError::DuplicateInput("prior".to_string()))
        );
    }

    #[test]
    fn signature_names_are_unique_per_list() {
        let mut f = classifier();
        let d = ValueDescriptor::Array(NDArrayDescriptor::new(&[1], ScalarType::Float32).unwrap());
        assert_eq!(f.add_input("prior", d.clone()), Err(InferenceError::DuplicateName("prior".to_string())));
        assert_eq!(f.add_output("scores", d.clone()), Err(InferenceError::DuplicateName("scores".to_string())));
        assert!(f.add_output("labels", d).is_ok());
        assert_eq!(f.output_count(), 2);
    }

    #[test]
    fn image_to_ndarray_maps_bytes_into_hwc_order() {
        let image = ImageDescriptor::new(2, 1, PixelFormat::Gray8).unwrap();
        let array = image.to_ndarray(&[0, 4], 0.5, 1.0).unwrap();
        assert_eq!(array.shape(), &[1, 2, 1]);
        assert_eq!(array.scalars().to_vec(), vec![1.0, 3.0]);
    }

    #[test]
    fn image_values_and_descriptors_reject_bad_sizes() {
        assert_eq!(
            ImageDescriptor::new(0, 4, PixelFormat::Rgba8),
            Err(InferenceError::InvalidImage { width: 0, height: 4 })
        );
        let image = ImageDescriptor::new(2, 2, PixelFormat::Rgba8).unwrap();
        assert_eq!(image.minimum_byte_count(), 16);
        assert_eq!(
            InferenceValue::image(image, vec![0; 15]),
            Err(InferenceError::ImageBytes { expected: 16, got: 15 })
        );
    }
}
