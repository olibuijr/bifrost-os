//! Web-compatibility polyfills, injected at document start through the shared user content manager
//! (same mechanism as `devtools::collector_builder`). Each is guarded by a feature test and only
//! defines an API Servo 0.7.0 does not expose (`IntersectionObserver` and `ResizeObserver` sit
//! behind `dom_*_enabled` prefs that are off by default; `requestIdleCallback` has no binding).
//! Script: `compat.js`.

/// The polyfill script.
pub(super) const SCRIPT: &str = include_str!("compat.js");

#[cfg(test)]
mod tests {
    use super::SCRIPT;

    #[test]
    fn every_polyfill_is_guarded() {
        assert!(SCRIPT.contains("typeof w.requestIdleCallback !== 'function'"));
        assert!(SCRIPT.contains("typeof w.IntersectionObserver === 'undefined'"));
        assert!(SCRIPT.contains("typeof w.ResizeObserver === 'undefined'"));
    }

    #[test]
    fn braces_balance() {
        assert_eq!(SCRIPT.matches('{').count(), SCRIPT.matches('}').count());
    }
}
