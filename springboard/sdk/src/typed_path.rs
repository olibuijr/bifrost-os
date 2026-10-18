//! Typed file paths, one component at a time: a path is a root (absolute or relative) and a list of parts. A part is a
//! name, `.` or `..`. Building a path checks its text: no empty text, no NUL byte, and no separator hidden inside a name.
//! Lexical normalisation works on the parts and never touches the file system, so it can run on a path that does not
//! exist yet.
//!
//! The language's path types (`std::path::Path` and `PathBuf`) remain what the rest of the SDK takes. This module adds
//! the checks that those types do not make: a name cannot carry a separator, and a path cannot carry a NUL byte into a
//! system call. [`TypedPath::to_path_buf`] is the bridge back to the language type.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// One component of a path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// `.`: the current directory.
    CurrentDirectory,
    /// `..`: the parent directory.
    ParentDirectory,
    /// A name: non-empty, with no separator and no NUL byte.
    Name(String),
}

impl Part {
    /// Checks one component. `.` and `..` keep their own meaning; any other text becomes a name.
    pub fn from_name(text: &str) -> Result<Part, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if let Some(at) = text.find('\0') {
            return Err(PathError::NulByte(at));
        }
        match text {
            "." => Ok(Part::CurrentDirectory),
            ".." => Ok(Part::ParentDirectory),
            _ if text.contains('/') => Err(PathError::SeparatorInName),
            _ => Ok(Part::Name(text.to_string())),
        }
    }

    fn text(&self) -> &str {
        match self {
            Part::CurrentDirectory => ".",
            Part::ParentDirectory => "..",
            Part::Name(name) => name,
        }
    }
}

/// Why a path or a component cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    /// The text is empty.
    Empty,
    /// The text holds a NUL byte at this byte offset. A system call would end the path there.
    NulByte(usize),
    /// A component contains the `/` separator.
    SeparatorInName,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => f.write_str("the path or name is empty"),
            PathError::NulByte(at) => write!(f, "the path has a NUL byte at offset {at}"),
            PathError::SeparatorInName => f.write_str("a name cannot contain a separator"),
        }
    }
}

impl std::error::Error for PathError {}

/// A path split into its root and its parts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TypedPath {
    absolute: bool,
    parts: Vec<Part>,
}

impl TypedPath {
    /// Parses slash-separated text. A leading `/` makes the path absolute. Repeated separators and a trailing separator
    /// add nothing; `.` and `..` stay as written until [`TypedPath::lexically_normalized`] is called.
    pub fn parse(text: &str) -> Result<TypedPath, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if let Some(at) = text.find('\0') {
            return Err(PathError::NulByte(at));
        }
        let parts = text
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(|segment| match segment {
                "." => Part::CurrentDirectory,
                ".." => Part::ParentDirectory,
                name => Part::Name(name.to_string()),
            })
            .collect();
        Ok(TypedPath { absolute: text.starts_with('/'), parts })
    }

    /// True when the path starts at the root directory.
    pub fn is_absolute(&self) -> bool {
        self.absolute
    }

    /// The components after the root, in order.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The last component when it is a name, such as `report.pdf` for `/x/report.pdf`. `None` for the root, for an
    /// empty relative path, and when the last component is `.` or `..`.
    pub fn file_name(&self) -> Option<&str> {
        match self.parts.last() {
            Some(Part::Name(name)) => Some(name),
            _ => None,
        }
    }

    /// Appends one component. The text is checked as a component: a separator inside it is an error, and the path is
    /// left unchanged when the check fails.
    pub fn push(&mut self, name: &str) -> Result<(), PathError> {
        let part = Part::from_name(name)?;
        self.parts.push(part);
        Ok(())
    }

    /// Removes the last component. Returns false when there was none to remove, as at the root.
    pub fn remove_last(&mut self) -> bool {
        self.parts.pop().is_some()
    }

    /// Resolves `.` and `..` on the text alone. A `..` after a name removes that name. At the root a `..` has no effect;
    /// on a relative path a `..` that cannot cancel a name is kept.
    pub fn lexically_normalized(&self) -> TypedPath {
        let mut out: Vec<Part> = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            match part {
                Part::CurrentDirectory => {}
                Part::ParentDirectory => match out.last() {
                    Some(Part::Name(_)) => {
                        out.pop();
                    }
                    _ if self.absolute => {}
                    _ => out.push(Part::ParentDirectory),
                },
                name => out.push(name.clone()),
            }
        }
        TypedPath { absolute: self.absolute, parts: out }
    }

    /// The language path type with the same text. An empty relative path becomes `.`.
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.to_string())
    }
}

/// Whether a path starts at the root directory or at the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Root {
    /// The path starts with `/`.
    Absolute,
    /// The path starts with a name, `.` or `..`.
    Relative,
}

/// A read-only view of a path's components: the root, then each part in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentView<'a> {
    root: Root,
    parts: &'a [Part],
}

impl<'a> ComponentView<'a> {
    /// The root the components hang from.
    pub fn root(&self) -> Root {
        self.root
    }

    /// The components after the root, in order.
    pub fn parts(&self) -> &'a [Part] {
        self.parts
    }

    /// The number of components after the root.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// True when there are no components after the root (`/`, or an empty relative path).
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Each component in order.
    pub fn iter(&self) -> std::slice::Iter<'a, Part> {
        self.parts.iter()
    }
}

impl TypedPath {
    /// The root of the path.
    pub fn root(&self) -> Root {
        if self.absolute { Root::Absolute } else { Root::Relative }
    }

    /// A view of the root and the components, for code that walks a path without taking it apart.
    pub fn components(&self) -> ComponentView<'_> {
        ComponentView { root: self.root(), parts: &self.parts }
    }

    /// The path as a NUL-terminated platform string, for a system call. Parsing already refuses a NUL byte, so this
    /// fails only for a path built some other way; the error names the position of the byte.
    pub fn to_c_string(&self) -> Result<std::ffi::CString, PathError> {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::CString::new(self.to_path_buf().as_os_str().as_bytes()).map_err(|e| PathError::NulByte(e.nul_position()))
    }
}

impl fmt::Display for TypedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.absolute {
            f.write_str("/")?;
        }
        if self.parts.is_empty() {
            return if self.absolute { Ok(()) } else { f.write_str(".") };
        }
        for (index, part) in self.parts.iter().enumerate() {
            if index > 0 {
                f.write_str("/")?;
            }
            f.write_str(part.text())?;
        }
        Ok(())
    }
}

impl FromStr for TypedPath {
    type Err = PathError;

    fn from_str(text: &str) -> Result<TypedPath, PathError> {
        TypedPath::parse(text)
    }
}
