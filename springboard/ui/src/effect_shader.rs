//! App-supplied shader effects as a pure model: the four effect kinds an app attaches to a view (`colorEffect`,
//! `distortionEffect`, `layerEffect` and `visualEffect`), each bound to one fragment function of a `ShaderLibrary` and to a
//! set of uniforms. Before a renderer builds anything, `validate` checks that the function exists and runs in the fragment
//! stage, and the uniform set holds only valid names and finite values. The model compiles no shader and holds no source.
//!
//! The kinds follow the reference effect modifiers: a colour effect maps each pixel's colour, a distortion effect returns
//! a sample offset for each pixel, a layer effect reads the layer's pixels through a sampler, and a visual effect is
//! applied to a view's composited output. All four take a fragment function here.

use crate::shader_library::{Function, LibraryError, ShaderLibrary, Stage};
use crate::shader_uniform::{UniformError, UniformSet, Value};

/// The effect modifier an app attaches a shader with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// `colorEffect`: returns a new colour for each pixel.
    Color,
    /// `distortionEffect`: returns a sample offset for each pixel, which moves the drawn content.
    Distortion,
    /// `layerEffect`: reads the layer's pixels and returns a colour.
    Layer,
    /// `visualEffect`: applies to the composited output of a view.
    Visual,
}

impl EffectKind {
    /// The reference modifier name.
    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Color => "colorEffect",
            EffectKind::Distortion => "distortionEffect",
            EffectKind::Layer => "layerEffect",
            EffectKind::Visual => "visualEffect",
        }
    }

    /// True for a distortion effect, whose result is a sample offset rather than a colour.
    pub fn returns_offset(self) -> bool {
        self == EffectKind::Distortion
    }
}

/// One shader effect: its kind, the fragment function it runs, and its uniforms.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderEffect {
    kind: EffectKind,
    function: String,
    uniforms: UniformSet,
}

impl ShaderEffect {
    /// An effect that runs `function` from a library. The uniform set starts empty.
    pub fn new(kind: EffectKind, function: &str) -> Self {
        ShaderEffect { kind, function: function.to_string(), uniforms: UniformSet::new() }
    }

    pub fn kind(&self) -> EffectKind {
        self.kind
    }

    /// The name of the fragment function this effect runs.
    pub fn function(&self) -> &str {
        &self.function
    }

    pub fn uniforms(&self) -> &UniformSet {
        &self.uniforms
    }

    /// Sets one uniform. Returns the uniform error, and changes nothing, when the name or value is refused.
    pub fn set_uniform(&mut self, name: &str, value: Value) -> Result<(), UniformError> {
        self.uniforms.set(name, value)
    }

    /// Checks the effect against a library: the function must exist and be a fragment function. A missing function is
    /// `NotFound`, and a function of another stage is `WrongStage`.
    pub fn validate(&self, lib: &ShaderLibrary) -> Result<(), LibraryError> {
        let function: &Function = lib.make_function(&self.function)?;
        if function.stage != Stage::Fragment {
            return Err(LibraryError::WrongStage { name: self.function.clone(), found: function.stage });
        }
        Ok(())
    }
}
