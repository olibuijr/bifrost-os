//! Startup options and browser feature preferences for the pinned Servo release.
//! Public builder inputs: <https://github.com/servo/servo/blob/v0.7.0/components/servo/lib.rs>.

use servo::{Opts, Preferences};

pub(super) fn options_from_env() -> Opts {
    Opts {
        config_dir: std::env::var_os("BIFROST_WEBVIEW_PROFILE").map(std::path::PathBuf::from),
        multiprocess: std::env::var_os("BIFROST_WEBVIEW_SINGLE").is_none(),
        ..Opts::default()
    }
}

pub(super) fn preferences_from_env() -> Preferences {
    let mut preferences = Preferences::default();
    super::memory::apply(&mut preferences);
    // Use native observers instead of document-injected polling approximations.
    // IntersectionObserver is opt-in in Servo 0.7; ResizeObserver is already enabled by default.
    preferences.dom_intersection_observer_enabled = true;
    preferences.dom_resize_observer_enabled = true;
    // Preserve the browser compatibility UA override; an explicitly empty value keeps Servo's UA.
    let ua = std::env::var("BIFROST_BROWSER_UA").unwrap_or_else(|_| super::DEFAULT_UA.to_string());
    if !ua.is_empty() {
        preferences.user_agent = ua;
    }
    preferences
}
