//! Weather data shared with the Today widget and the lock screen; the app
//! itself is the `weather` package (apps/weather-desktop).

pub mod data;
pub mod glyph;
pub use glyph::icon;

pub fn data_path() -> String {
    crate::shell::apphost::root().join("appdata/is.olibuijr.weather").to_string_lossy().into_owned()
}
