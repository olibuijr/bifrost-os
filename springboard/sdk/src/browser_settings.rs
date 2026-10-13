//! System-wide browser settings (homepage, default search engine, restore last session). The model lives in
//! `bifrost_ui::browser_settings`; this module re-exports it for `bifrost_app`. Values are validated here and stored by
//! the caller.

pub use bifrost_ui::browser_settings::{
    BrowserSettings, Error, SearchEngine, KEYS, KEY_HOMEPAGE, KEY_RESTORE_SESSION, KEY_SEARCH_ENGINE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_homepage_is_stored_and_read_back_by_key() {
        let mut settings = BrowserSettings::new();
        settings.set(KEY_HOMEPAGE, "https://example.org").unwrap();
        assert_eq!(settings.get(KEY_HOMEPAGE).unwrap(), "https://example.org");
        assert!(KEYS.contains(&KEY_SEARCH_ENGINE));
        assert!(KEYS.contains(&KEY_RESTORE_SESSION));
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let mut settings = BrowserSettings::new();
        assert!(settings.set("browser.unknown", "x").is_err());
    }
}
