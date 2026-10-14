//! Screen saver descriptor: the text a saver module ships to declare its name, frame interval, gamma fade, backing
//! store and configuration options. The descriptor is read on the host and turned into a [`ScreenSaverView`] and an
//! empty-valued [`ConfigureSheet`]. The saver's drawing code and bundle files are not read here.
//!
//! Format: one `key = value` pair per line. Blank lines and lines starting with `#` are ignored. Keys:
//!
//! - `name`: required, the module's display name.
//! - `frame_interval`: seconds between frames, a positive number. Optional.
//! - `gamma_fade`: `true` or `false`. Optional, `false` by default.
//! - `backing_store`: `buffered`, `retained` or `nonretained`. Optional, `buffered` by default.
//! - `option.<key>`: one configuration option, written as `bool <default>`, `int <min> <max> <default>` or
//!   `choice <a,b,c> <default>`. Choices are separated by commas with no spaces.

use crate::screensaver::{
    BackingStoreType, ConfigureSheet, ScreenSaverView, DEFAULT_FRAME_INTERVAL,
};
use crate::Rect;

/// Why a descriptor was refused. Line numbers start at one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// A line that is not a comment, blank, or `key = value`.
    Syntax { line: usize },
    /// A key this format does not define.
    UnknownKey { line: usize, key: String },
    /// A value that is malformed, out of range, or repeats an option key.
    BadValue { line: usize, key: String },
    /// The descriptor has no `name` line, or the name is empty.
    MissingName,
}

/// A parsed descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct SaverManifest {
    name: String,
    frame_interval: f64,
    gamma_fade: bool,
    backing_store: BackingStoreType,
    sheet: ConfigureSheet,
}

impl SaverManifest {
    /// Parses a descriptor. Later `name` lines replace earlier ones.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let mut name: Option<String> = None;
        let mut frame_interval = DEFAULT_FRAME_INTERVAL;
        let mut gamma_fade = false;
        let mut backing_store = BackingStoreType::Buffered;
        let mut sheet = ConfigureSheet::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed.split_once('=').ok_or(ManifestError::Syntax { line })?;
            let (key, value) = (key.trim(), value.trim());
            let bad = || ManifestError::BadValue { line, key: key.to_string() };
            match key {
                "name" => {
                    if value.is_empty() {
                        return Err(bad());
                    }
                    name = Some(value.to_string());
                }
                "frame_interval" => {
                    frame_interval = value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .ok_or_else(bad)?;
                }
                "gamma_fade" => gamma_fade = parse_bool(value).ok_or_else(bad)?,
                "backing_store" => backing_store = parse_backing_store(value).ok_or_else(bad)?,
                _ => {
                    let option = key
                        .strip_prefix("option.")
                        .filter(|o| !o.is_empty())
                        .ok_or_else(|| ManifestError::UnknownKey { line, key: key.to_string() })?;
                    if !add_option(&mut sheet, option, value) {
                        return Err(bad());
                    }
                }
            }
        }
        let name = name.ok_or(ManifestError::MissingName)?;
        Ok(SaverManifest { name, frame_interval, gamma_fade, backing_store, sheet })
    }

    /// The module's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The configuration options, with their defaults. Saved values are applied with `ConfigureSheet::restore`.
    pub fn sheet(&self) -> &ConfigureSheet {
        &self.sheet
    }

    /// A view for `frame` configured from the descriptor. The frame interval is clamped as `set_animation_time_interval`
    /// does, and the view reports a configuration sheet only when the descriptor declares options.
    pub fn view(&self, frame: Rect, is_preview: bool) -> ScreenSaverView {
        let mut view = ScreenSaverView::new(frame, is_preview);
        view.set_animation_time_interval(self.frame_interval);
        view.set_perform_gamma_fade(self.gamma_fade);
        view.set_backing_store_type(self.backing_store);
        view.set_has_configure_sheet(!self.sheet.is_empty());
        view
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_backing_store(value: &str) -> Option<BackingStoreType> {
    match value {
        "buffered" => Some(BackingStoreType::Buffered),
        "retained" => Some(BackingStoreType::Retained),
        "nonretained" => Some(BackingStoreType::Nonretained),
        _ => None,
    }
}

/// Adds one option from its spec. Returns false when the spec is malformed or the sheet refuses the option.
fn add_option(sheet: &mut ConfigureSheet, key: &str, spec: &str) -> bool {
    let parts: Vec<&str> = spec.split_whitespace().collect();
    match parts.as_slice() {
        ["bool", default] => parse_bool(default).is_some_and(|d| sheet.add_bool(key, d)),
        ["int", min, max, default] => match (min.parse::<i64>(), max.parse::<i64>(), default.parse::<i64>()) {
            (Ok(lo), Ok(hi), Ok(d)) => sheet.add_int(key, lo, hi, d),
            _ => false,
        },
        ["choice", list, default] => {
            let choices: Vec<&str> = list.split(',').filter(|c| !c.is_empty()).collect();
            sheet.add_choice(key, &choices, default)
        }
        _ => false,
    }
}
