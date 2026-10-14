//! Shader descriptions for scene nodes: the source text, the per-vertex attributes it reads and the uniform values
//! the app sets. The model checks what the system needs before it accepts a shader: a non-empty source, identifier
//! names, one name per attribute or uniform, and finite values.
//!
//! The system presenter runs no application shader yet. A validated description is the request the presenter would
//! receive; the glass and blur passes stay with the system.

/// The type of a value a shader reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
}

impl Value {
    /// True when every component is a finite number.
    fn is_finite(&self) -> bool {
        match self {
            Value::Float(v) => v.is_finite(),
            Value::Vec2(v) => v.iter().all(|c| c.is_finite()),
            Value::Vec4(v) => v.iter().all(|c| c.is_finite()),
        }
    }
}

/// A named uniform: one value the app sets for every draw of the node.
#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    pub name: String,
    pub value: Value,
}

/// The per-vertex type of an attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeKind {
    Float,
    Vec2,
    Vec4,
}

/// A named per-vertex attribute the shader reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub kind: AttributeKind,
}

/// Why a shader description was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderError {
    /// The source is empty or only white space.
    EmptySource,
    /// A name is not an identifier (a letter or underscore, then letters, digits or underscores).
    InvalidName(String),
    /// Two uniforms or attributes share a name.
    DuplicateName(String),
    /// A uniform holds a value that is not finite.
    NonFiniteValue(String),
}

/// A shader: its source text, its attributes and its uniforms.
#[derive(Clone, Debug, PartialEq)]
pub struct Shader {
    pub source: String,
    pub attributes: Vec<Attribute>,
    pub uniforms: Vec<Uniform>,
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => chars.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

impl Shader {
    /// A shader with no attributes or uniforms yet.
    pub fn new(source: &str) -> Shader {
        Shader { source: source.to_string(), attributes: Vec::new(), uniforms: Vec::new() }
    }

    /// Adds an attribute the source reads.
    pub fn with_attribute(mut self, name: &str, kind: AttributeKind) -> Shader {
        self.attributes.push(Attribute { name: name.to_string(), kind });
        self
    }

    /// Sets a uniform, replacing the value of an existing uniform with the same name.
    pub fn set_uniform(&mut self, name: &str, value: Value) {
        match self.uniforms.iter_mut().find(|u| u.name == name) {
            Some(existing) => existing.value = value,
            None => self.uniforms.push(Uniform { name: name.to_string(), value }),
        }
    }

    /// The value of a uniform, if it is set.
    pub fn uniform(&self, name: &str) -> Option<Value> {
        self.uniforms.iter().find(|u| u.name == name).map(|u| u.value)
    }

    /// Checks the description. Names are unique across attributes and uniforms, because the source reads both from
    /// one namespace.
    pub fn validate(&self) -> Result<(), ShaderError> {
        if self.source.trim().is_empty() {
            return Err(ShaderError::EmptySource);
        }
        let mut seen: Vec<&str> = Vec::new();
        let names = self.attributes.iter().map(|a| a.name.as_str()).chain(self.uniforms.iter().map(|u| u.name.as_str()));
        for name in names {
            if !is_identifier(name) {
                return Err(ShaderError::InvalidName(name.to_string()));
            }
            if seen.contains(&name) {
                return Err(ShaderError::DuplicateName(name.to_string()));
            }
            seen.push(name);
        }
        for uniform in &self.uniforms {
            if !uniform.value.is_finite() {
                return Err(ShaderError::NonFiniteValue(uniform.name.clone()));
            }
        }
        Ok(())
    }
}
