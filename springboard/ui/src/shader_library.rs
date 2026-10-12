//! Shader function tables as a pure model: a named library of entry points, each with the stage it runs in (vertex,
//! fragment or compute kernel). It answers the questions an app asks before it builds a pipeline: which names exist,
//! whether a name is a valid identifier, and whether a vertex and fragment pair (or a kernel) fits the pipeline kind.
//!
//! The model compiles nothing and holds no shader source. Entry points are registered by name and stage, so the
//! compositor's fixed shaders and any future app-supplied table use the same checks. No GPU object is created.
//!
//! Rules: a name is 1 to 64 ASCII letters, digits or underscores and does not start with a digit. Names are unique
//! within a library. A render pipeline needs a vertex entry point and a fragment entry point, in that order of
//! arguments. A compute pipeline needs a kernel entry point. Lookups are exact and case-sensitive.

/// The stage an entry point runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Kernel,
}

/// One entry point in a library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub stage: Stage,
}

/// Why a library or a pipeline request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryError {
    /// The name is empty, too long, starts with a digit or has a character outside the identifier set.
    InvalidName(String),
    /// A function with this name is already in the library.
    Duplicate(String),
    /// No function with this name is in the library.
    NotFound(String),
    /// The named function exists but runs in the wrong stage for the pipeline.
    WrongStage { name: String, found: Stage },
}

/// The longest allowed entry point name.
pub const MAX_NAME_LEN: usize = 64;

/// True when `name` is a valid entry point identifier.
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A named table of entry points.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderLibrary {
    /// A label for diagnostics. It has no effect on lookups.
    pub label: Option<String>,
    functions: Vec<Function>,
}

impl ShaderLibrary {
    /// An empty library with no label.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty library with a label.
    pub fn with_label(label: &str) -> Self {
        ShaderLibrary { label: Some(label.to_string()), functions: Vec::new() }
    }

    /// Registers an entry point. Names must be valid and unique.
    pub fn add(&mut self, name: &str, stage: Stage) -> Result<(), LibraryError> {
        if !is_valid_name(name) {
            return Err(LibraryError::InvalidName(name.to_string()));
        }
        if self.functions.iter().any(|f| f.name == name) {
            return Err(LibraryError::Duplicate(name.to_string()));
        }
        self.functions.push(Function { name: name.to_string(), stage });
        Ok(())
    }

    /// The names in registration order.
    pub fn function_names(&self) -> Vec<&str> {
        self.functions.iter().map(|f| f.name.as_str()).collect()
    }

    /// Finds an entry point by exact name.
    pub fn make_function(&self, name: &str) -> Result<&Function, LibraryError> {
        self.functions
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| LibraryError::NotFound(name.to_string()))
    }

    /// Checks a render pipeline: `vertex` must be a vertex entry point and `fragment` a fragment entry point.
    pub fn check_render_pipeline(&self, vertex: &str, fragment: &str) -> Result<(), LibraryError> {
        expect_stage(self.make_function(vertex)?, Stage::Vertex)?;
        expect_stage(self.make_function(fragment)?, Stage::Fragment)?;
        Ok(())
    }

    /// Checks a compute pipeline: `kernel` must be a kernel entry point.
    pub fn check_compute_pipeline(&self, kernel: &str) -> Result<(), LibraryError> {
        expect_stage(self.make_function(kernel)?, Stage::Kernel)
    }
}

fn expect_stage(f: &Function, want: Stage) -> Result<(), LibraryError> {
    if f.stage == want {
        Ok(())
    } else {
        Err(LibraryError::WrongStage { name: f.name.clone(), found: f.stage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ShaderLibrary {
        let mut lib = ShaderLibrary::with_label("compositor");
        lib.add("quad_vertex", Stage::Vertex).unwrap();
        lib.add("quad_fragment", Stage::Fragment).unwrap();
        lib.add("blur_pass", Stage::Kernel).unwrap();
        lib
    }

    #[test]
    fn names_follow_the_identifier_rule() {
        assert!(is_valid_name("quad_vertex"));
        assert!(is_valid_name("_a1"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("1st"));
        assert!(!is_valid_name("has space"));
        assert!(!is_valid_name("dash-name"));
        assert!(!is_valid_name(&"a".repeat(MAX_NAME_LEN + 1)));
        assert!(is_valid_name(&"a".repeat(MAX_NAME_LEN)));
    }

    #[test]
    fn duplicate_and_invalid_names_are_refused() {
        let mut lib = sample();
        assert_eq!(lib.add("quad_vertex", Stage::Vertex), Err(LibraryError::Duplicate("quad_vertex".into())));
        assert_eq!(lib.add("bad name", Stage::Kernel), Err(LibraryError::InvalidName("bad name".into())));
        assert_eq!(lib.function_names(), vec!["quad_vertex", "quad_fragment", "blur_pass"]);
    }

    #[test]
    fn lookup_is_exact_and_case_sensitive() {
        let lib = sample();
        assert_eq!(lib.make_function("blur_pass").unwrap().stage, Stage::Kernel);
        assert_eq!(lib.make_function("Blur_pass"), Err(LibraryError::NotFound("Blur_pass".into())));
        assert_eq!(lib.make_function("blur"), Err(LibraryError::NotFound("blur".into())));
    }

    #[test]
    fn render_pipeline_needs_vertex_then_fragment() {
        let lib = sample();
        assert_eq!(lib.check_render_pipeline("quad_vertex", "quad_fragment"), Ok(()));
        assert_eq!(
            lib.check_render_pipeline("quad_fragment", "quad_vertex"),
            Err(LibraryError::WrongStage { name: "quad_fragment".into(), found: Stage::Fragment })
        );
        assert_eq!(
            lib.check_render_pipeline("quad_vertex", "blur_pass"),
            Err(LibraryError::WrongStage { name: "blur_pass".into(), found: Stage::Kernel })
        );
    }

    #[test]
    fn compute_pipeline_needs_a_kernel() {
        let lib = sample();
        assert_eq!(lib.check_compute_pipeline("blur_pass"), Ok(()));
        assert_eq!(
            lib.check_compute_pipeline("quad_vertex"),
            Err(LibraryError::WrongStage { name: "quad_vertex".into(), found: Stage::Vertex })
        );
        assert_eq!(lib.check_compute_pipeline("missing"), Err(LibraryError::NotFound("missing".into())));
    }

    #[test]
    fn an_empty_library_has_no_names() {
        let lib = ShaderLibrary::new();
        assert!(lib.function_names().is_empty());
        assert_eq!(lib.label, None);
    }
}
