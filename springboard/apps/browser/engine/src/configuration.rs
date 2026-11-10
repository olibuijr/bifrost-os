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

/// User-origin style sheet given to every document (`devtools::manager` adds it to the shared user content manager).
///
/// Servo 0.7.0 resolves system colours such as `CanvasText` (the root's default `color`, layout/stylesheets/servo.css)
/// from the element's `color-scheme`; when that is `normal` it follows the system appearance (stylo 0.22
/// device/servo.rs `is_dark_color_scheme`). The page canvas does not: a root with no background shows the fixed white
/// `shell_background_color_rgba`. In the dark appearance a page that sets no colours therefore drew pale text on
/// white. This makes `light` the default scheme, so default text and default canvas agree. It is a normal user-origin
/// declaration: a page's own `color-scheme` (author origin) still wins, and `prefers-color-scheme` is untouched.
pub(super) const DEFAULT_PAGE_STYLE: &str = ":root { color-scheme: light; }";

#[cfg(test)]
mod tests {
    use super::DEFAULT_PAGE_STYLE;

    /// The declarations of the `:root` rule, lower-cased, as (property, value) pairs.
    fn root_declarations() -> Vec<(String, String)> {
        let css = DEFAULT_PAGE_STYLE.to_ascii_lowercase();
        let (selector, rest) = css.split_once('{').expect("one rule");
        assert_eq!(selector.trim(), ":root");
        let (body, tail) = rest.split_once('}').expect("closed rule");
        assert!(tail.trim().is_empty(), "only the root rule");
        body.split(';')
            .filter_map(|declaration| declaration.split_once(':'))
            .map(|(property, value)| (property.trim().to_string(), value.trim().to_string()))
            .collect()
    }

    #[test]
    fn pages_default_to_the_light_scheme_and_may_override_it() {
        let declarations = root_declarations();
        // Default text (CanvasText) must come from the same scheme as the white default canvas.
        assert_eq!(declarations, [("color-scheme".to_string(), "light".to_string())]);
        // `!important` in the user origin would beat a page that declares `color-scheme: dark`.
        assert!(!DEFAULT_PAGE_STYLE.contains('!'));
    }
}
