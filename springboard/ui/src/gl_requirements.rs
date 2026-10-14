//! OpenGL and OpenGL ES capability checks. An app reads the `GL_VERSION` string of its rendering context, parses it
//! with [`GlVersion::parse`] and asks whether the context meets a [`GlRequirement`] before it chooses a renderer.
//!
//! The version strings follow the usual driver format: `"4.6.0 NVIDIA 550.54"`, `"3.2 (Core Profile) Mesa 23.2.1"` for a
//! desktop context and `"OpenGL ES 3.2 Mesa 23.2.1"` for an embedded one. The check is pure: it reads a string and
//! compares numbers, so it runs without a GPU. The contexts themselves are created by the system layer
//! (`springboard/src/gpu/egl.rs`), which is not yet reachable from apps.

use std::fmt;

/// The API family a context reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlApi {
    Desktop,
    Es,
}

/// A parsed `GL_VERSION` string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlVersion {
    pub api: GlApi,
    pub major: u32,
    pub minor: u32,
    /// True when the string names a core profile context (desktop only).
    pub core_profile: bool,
}

/// Why a version string could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlVersionError {
    /// The string is empty or only spaces.
    Empty,
    /// The string does not start with a `major.minor` number. Holds the trimmed input.
    Malformed(String),
}

impl fmt::Display for GlVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlVersionError::Empty => f.write_str("the rendering context reported no GL version"),
            GlVersionError::Malformed(text) => write!(f, "unrecognised GL version string: {text}"),
        }
    }
}

impl GlVersion {
    /// Parses a `GL_VERSION` string. Only the leading `major.minor` number is read; vendor and build text after it is
    /// ignored.
    pub fn parse(text: &str) -> Result<GlVersion, GlVersionError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(GlVersionError::Empty);
        }
        let (api, rest) = match text.strip_prefix("OpenGL ES ") {
            Some(rest) => (GlApi::Es, rest),
            None => (GlApi::Desktop, text),
        };
        let malformed = || GlVersionError::Malformed(text.to_string());
        let number = rest.split_whitespace().next().unwrap_or("");
        let mut parts = number.split('.');
        let major = parts.next().and_then(|p| p.parse::<u32>().ok()).ok_or_else(malformed)?;
        let minor = parts.next().and_then(|p| p.parse::<u32>().ok()).ok_or_else(malformed)?;
        Ok(GlVersion { api, major, minor, core_profile: api == GlApi::Desktop && rest.contains("Core Profile") })
    }

    /// True when this context is at least `major.minor` of the same API family.
    pub fn is_at_least(&self, api: GlApi, major: u32, minor: u32) -> bool {
        self.api == api && (self.major, self.minor) >= (major, minor)
    }

    /// True when this context meets `requirement`.
    pub fn satisfies(&self, requirement: GlRequirement) -> bool {
        match requirement {
            GlRequirement::DesktopCore32 => self.core_profile && self.is_at_least(GlApi::Desktop, 3, 2),
            GlRequirement::EmbeddedEs20 => self.is_at_least(GlApi::Es, 2, 0),
            GlRequirement::EmbeddedEs30 => self.is_at_least(GlApi::Es, 3, 0),
        }
    }
}

/// A rendering floor an app can require. The desktop floor is the core profile the reference system needs on its
/// desktop; the embedded floors are the OpenGL ES versions of the reference system's touch devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlRequirement {
    /// OpenGL 3.2 core profile.
    DesktopCore32,
    /// OpenGL ES 2.0.
    EmbeddedEs20,
    /// OpenGL ES 3.0.
    EmbeddedEs30,
}

impl GlRequirement {
    /// The requirement as a short phrase for messages.
    pub fn describe(&self) -> &'static str {
        match self {
            GlRequirement::DesktopCore32 => "OpenGL 3.2 core profile",
            GlRequirement::EmbeddedEs20 => "OpenGL ES 2.0",
            GlRequirement::EmbeddedEs30 => "OpenGL ES 3.0",
        }
    }

    /// Checks a parsed version against this requirement. The error names the requirement the context did not meet.
    pub fn check(&self, version: &GlVersion) -> Result<(), String> {
        if version.satisfies(*self) {
            Ok(())
        } else {
            Err(format!("the context does not meet {}", self.describe()))
        }
    }
}
