//! Errors of the media analysis requests (`MediaIntelligenceError`): one typed value for a request that cannot run.
//!
//! The face grouping model takes its inputs without complaint and treats a bad one as "never links". The checks here
//! refuse such an input first, with the reason: an empty or non-finite embedding, an embedding whose length differs
//! from the faces already stored, and a link threshold outside the cosine range from -1 to 1. A failure of the
//! on-device analysis runtime is carried unchanged. Pure data, no I/O.

use crate::ml_runtime::RuntimeError;
use std::fmt;

/// Why a media analysis request could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaIntelligenceError {
    /// The embedding has no values.
    EmptyEmbedding,
    /// The value at `index` is NaN or infinite.
    NonFiniteValue { index: usize },
    /// The embedding has `found` values, but the faces already stored have `expected`.
    DimensionMismatch { expected: usize, found: usize },
    /// The link threshold is NaN or outside the cosine range from -1 to 1.
    InvalidThreshold(f64),
    /// The on-device analysis runtime could not route or run the request.
    Runtime(RuntimeError),
}

impl fmt::Display for MediaIntelligenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaIntelligenceError::EmptyEmbedding => write!(f, "the face embedding has no values"),
            MediaIntelligenceError::NonFiniteValue { index } => {
                write!(f, "the face embedding value {index} is not a finite number")
            }
            MediaIntelligenceError::DimensionMismatch { expected, found } => {
                write!(f, "the face embedding has {found} values but the stored faces have {expected}")
            }
            MediaIntelligenceError::InvalidThreshold(t) => {
                write!(f, "the link threshold {t} is outside the cosine range from -1 to 1")
            }
            MediaIntelligenceError::Runtime(err) => write!(f, "the analysis runtime refused the request: {err}"),
        }
    }
}

impl std::error::Error for MediaIntelligenceError {}

impl From<RuntimeError> for MediaIntelligenceError {
    fn from(err: RuntimeError) -> Self {
        MediaIntelligenceError::Runtime(err)
    }
}

/// Checks one face embedding. `expected_len` is the length of the faces already stored, if there are any.
pub fn check_embedding(embedding: &[f64], expected_len: Option<usize>) -> Result<(), MediaIntelligenceError> {
    if embedding.is_empty() {
        return Err(MediaIntelligenceError::EmptyEmbedding);
    }
    if let Some(expected) = expected_len {
        if embedding.len() != expected {
            return Err(MediaIntelligenceError::DimensionMismatch { expected, found: embedding.len() });
        }
    }
    match embedding.iter().position(|v| !v.is_finite()) {
        Some(index) => Err(MediaIntelligenceError::NonFiniteValue { index }),
        None => Ok(()),
    }
}

/// Checks a link threshold: it must be finite and within the cosine range, -1 to 1 inclusive.
pub fn check_threshold(threshold: f64) -> Result<(), MediaIntelligenceError> {
    if threshold.is_finite() && (-1.0..=1.0).contains(&threshold) {
        Ok(())
    } else {
        Err(MediaIntelligenceError::InvalidThreshold(threshold))
    }
}
