//! Notification content: the height an app asks for its custom content view, per notification category, and the limits the
//! shell keeps it within.
//!
//! A [`ContentRegistry`] maps a category identifier to the height its content view should take in a banner or a list
//! card. Heights are clamped to [`MIN_HEIGHT`] and [`MAX_HEIGHT`] so a request cannot make a card unreadable or push
//! the rest of the list off the screen. The registry holds sizes only; drawing the view is the app's content process,
//! which the shell does not host yet.

use std::collections::BTreeMap;
use std::fmt;

/// The shortest content view, in points.
pub const MIN_HEIGHT: f64 = 44.0;
/// The tallest content view, in points.
pub const MAX_HEIGHT: f64 = 320.0;
/// The longest category identifier, in bytes.
pub const MAX_CATEGORY_LEN: usize = 128;

/// Why a registration or a removal was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentError {
    /// The category identifier is empty.
    EmptyCategory,
    /// The category identifier is too long, or holds a character other than an ASCII letter, a digit, `.`, `-` or `_`.
    BadCategory(String),
    /// The requested height is NaN or infinite.
    HeightNotFinite,
    /// The category has no registered content view to remove.
    UnknownCategory(String),
}

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentError::EmptyCategory => f.write_str("notification category is empty"),
            ContentError::BadCategory(name) => write!(f, "notification category is not a valid identifier: {name}"),
            ContentError::HeightNotFinite => f.write_str("content height is not a finite number"),
            ContentError::UnknownCategory(name) => write!(f, "no content view is registered for category {name}"),
        }
    }
}

impl std::error::Error for ContentError {}

/// The height a category's content view gets, already within the limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentLayout {
    pub height: f64,
}

/// Clamps a requested height to the limits of a content view.
pub fn clamp_height(height: f64) -> f64 {
    height.clamp(MIN_HEIGHT, MAX_HEIGHT)
}

fn validate_category(category: &str) -> Result<(), ContentError> {
    if category.is_empty() {
        return Err(ContentError::EmptyCategory);
    }
    let valid = category.len() <= MAX_CATEGORY_LEN && category.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(ContentError::BadCategory(category.into()))
    }
}

/// The content heights registered by category, in category order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentRegistry {
    layouts: BTreeMap<String, ContentLayout>,
}

impl ContentRegistry {
    /// Registers the content view height for a category, or replaces the height of an earlier registration. The height
    /// is clamped; the returned layout is the one kept.
    pub fn register(&mut self, category: &str, preferred_height: f64) -> Result<ContentLayout, ContentError> {
        validate_category(category)?;
        if !preferred_height.is_finite() {
            return Err(ContentError::HeightNotFinite);
        }
        let layout = ContentLayout { height: clamp_height(preferred_height) };
        self.layouts.insert(category.to_string(), layout);
        Ok(layout)
    }

    /// The layout kept for a category, or `None` when no content view is registered for it.
    pub fn layout_for(&self, category: &str) -> Option<ContentLayout> {
        self.layouts.get(category).copied()
    }

    /// Removes a category's content view. Fails when the category has none.
    pub fn unregister(&mut self, category: &str) -> Result<(), ContentError> {
        self.layouts.remove(category).map(|_| ()).ok_or_else(|| ContentError::UnknownCategory(category.into()))
    }
}
