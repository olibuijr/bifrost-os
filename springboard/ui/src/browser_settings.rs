//! System-wide browser settings: the homepage, the default search engine, and whether a new window restores the last
//! session. Apps read and write them by key through one object. The browser keeps its own values in its defaults under
//! these same keys, so an app that asks for a setting gets the value the browser uses. Pure model: it validates values
//! and names keys; nothing is stored here.

use crate::reading_list::ReadingList;
use std::fmt;

pub const KEY_HOMEPAGE: &str = "browser.homepage";
pub const KEY_SEARCH_ENGINE: &str = "browser.searchEngine";
pub const KEY_RESTORE_SESSION: &str = "browser.restoreSession";
/// Every key this object knows.
pub const KEYS: [&str; 3] = [KEY_HOMEPAGE, KEY_SEARCH_ENGINE, KEY_RESTORE_SESSION];
/// The longest homepage address kept.
pub const MAX_HOMEPAGE_LEN: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchEngine {
    DuckDuckGo,
    Google,
    Bing,
    Startpage,
}

impl SearchEngine {
    pub const ALL: [SearchEngine; 4] =
        [SearchEngine::DuckDuckGo, SearchEngine::Google, SearchEngine::Bing, SearchEngine::Startpage];

    pub fn name(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "DuckDuckGo",
            SearchEngine::Google => "Google",
            SearchEngine::Bing => "Bing",
            SearchEngine::Startpage => "Startpage",
        }
    }

    /// The engine with this name, ignoring case and surrounding spaces.
    pub fn parse(name: &str) -> Option<SearchEngine> {
        SearchEngine::ALL.iter().copied().find(|e| e.name().eq_ignore_ascii_case(name.trim()))
    }

    /// The search address up to and including `q=`.
    pub fn search_prefix(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "https://duckduckgo.com/?q=",
            SearchEngine::Google => "https://www.google.com/search?q=",
            SearchEngine::Bing => "https://www.bing.com/search?q=",
            SearchEngine::Startpage => "https://www.startpage.com/do/search?q=",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The key is not one of `KEYS`.
    UnknownKey(String),
    /// The homepage is neither empty nor a plain http or https address within the length limit.
    InvalidHomepage,
    UnknownSearchEngine(String),
    /// A restore setting is neither `true` nor `false`.
    InvalidBoolean(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownKey(key) => write!(f, "no browser setting named {key}"),
            Error::InvalidHomepage => write!(f, "the homepage must be an http or https address"),
            Error::UnknownSearchEngine(name) => write!(f, "no search engine named {name}"),
            Error::InvalidBoolean(value) => write!(f, "{value} is not true or false"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserSettings {
    homepage: String,
    engine: SearchEngine,
    restore_session: bool,
}

impl Default for BrowserSettings {
    /// A blank start page, DuckDuckGo, and restore on launch.
    fn default() -> Self {
        BrowserSettings { homepage: String::new(), engine: SearchEngine::DuckDuckGo, restore_session: true }
    }
}

impl BrowserSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// The homepage address, or an empty string for a blank start page.
    pub fn homepage(&self) -> &str {
        &self.homepage
    }

    pub fn search_engine(&self) -> SearchEngine {
        self.engine
    }

    pub fn restore_session(&self) -> bool {
        self.restore_session
    }

    /// Sets the homepage. Empty (after trimming) means a blank start page. Anything else must pass the reading-list
    /// address check and stay within `MAX_HOMEPAGE_LEN`. A refused value leaves the setting as it was.
    pub fn set_homepage(&mut self, url: &str) -> Result<(), Error> {
        let url = url.trim();
        if url.is_empty() {
            self.homepage = String::new();
            return Ok(());
        }
        if url.len() > MAX_HOMEPAGE_LEN || !ReadingList::supports(url) {
            return Err(Error::InvalidHomepage);
        }
        self.homepage = url.to_string();
        Ok(())
    }

    pub fn set_search_engine(&mut self, name: &str) -> Result<(), Error> {
        self.engine = SearchEngine::parse(name).ok_or_else(|| Error::UnknownSearchEngine(name.trim().to_string()))?;
        Ok(())
    }

    pub fn set_restore_session(&mut self, on: bool) {
        self.restore_session = on;
    }

    /// Reads a setting by key, as the text the browser stores it in.
    pub fn get(&self, key: &str) -> Result<String, Error> {
        match key {
            KEY_HOMEPAGE => Ok(self.homepage.clone()),
            KEY_SEARCH_ENGINE => Ok(self.engine.name().to_string()),
            KEY_RESTORE_SESSION => Ok(self.restore_session.to_string()),
            _ => Err(Error::UnknownKey(key.to_string())),
        }
    }

    /// Writes a setting by key from its text form. A refused value leaves every setting as it was.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), Error> {
        match key {
            KEY_HOMEPAGE => self.set_homepage(value),
            KEY_SEARCH_ENGINE => self.set_search_engine(value),
            KEY_RESTORE_SESSION => {
                self.restore_session = parse_bool(value)?;
                Ok(())
            }
            _ => Err(Error::UnknownKey(key.to_string())),
        }
    }
}

/// Reads `true` or `false`, ignoring case and surrounding spaces.
fn parse_bool(value: &str) -> Result<bool, Error> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(Error::InvalidBoolean(value.trim().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_match_a_fresh_browser() {
        let settings = BrowserSettings::new();
        assert_eq!(settings.homepage(), "");
        assert_eq!(settings.search_engine(), SearchEngine::DuckDuckGo);
        assert!(settings.restore_session());
        assert_eq!(settings.get(KEY_RESTORE_SESSION), Ok("true".to_string()));
        assert_eq!(settings.get(KEY_SEARCH_ENGINE), Ok("DuckDuckGo".to_string()));
    }

    #[test]
    fn a_homepage_must_be_a_plain_http_or_https_address() {
        let mut settings = BrowserSettings::new();
        settings.set_homepage("https://example.test/home").unwrap();
        assert_eq!(settings.homepage(), "https://example.test/home");
        assert_eq!(settings.set_homepage("ftp://x.test"), Err(Error::InvalidHomepage));
        assert_eq!(settings.set_homepage("https://a b.test"), Err(Error::InvalidHomepage));
        assert_eq!(settings.homepage(), "https://example.test/home");
        settings.set_homepage("   ").unwrap();
        assert_eq!(settings.homepage(), "");
    }

    #[test]
    fn a_homepage_longer_than_the_limit_is_refused() {
        let mut settings = BrowserSettings::new();
        // "https://" is 8 bytes; 8 + 2040 = 2048 is allowed, one more byte is not.
        let at_limit = format!("https://{}", "a".repeat(MAX_HOMEPAGE_LEN - 8));
        assert_eq!(at_limit.len(), MAX_HOMEPAGE_LEN);
        settings.set_homepage(&at_limit).unwrap();
        let over = format!("https://{}", "a".repeat(MAX_HOMEPAGE_LEN - 7));
        assert_eq!(settings.set_homepage(&over), Err(Error::InvalidHomepage));
    }

    #[test]
    fn search_engines_are_matched_by_name_and_a_refused_name_keeps_the_old_one() {
        let mut settings = BrowserSettings::new();
        settings.set(KEY_SEARCH_ENGINE, " google ").unwrap();
        assert_eq!(settings.search_engine(), SearchEngine::Google);
        assert_eq!(settings.get(KEY_SEARCH_ENGINE), Ok("Google".to_string()));
        assert_eq!(
            settings.set(KEY_SEARCH_ENGINE, "Ecosia"),
            Err(Error::UnknownSearchEngine("Ecosia".to_string()))
        );
        assert_eq!(settings.search_engine(), SearchEngine::Google);
        assert_eq!(SearchEngine::Google.search_prefix(), "https://www.google.com/search?q=");
    }

    #[test]
    fn the_restore_setting_takes_only_true_or_false() {
        let mut settings = BrowserSettings::new();
        settings.set(KEY_RESTORE_SESSION, "FALSE").unwrap();
        assert!(!settings.restore_session());
        assert_eq!(
            settings.set(KEY_RESTORE_SESSION, "yes"),
            Err(Error::InvalidBoolean("yes".to_string()))
        );
        assert!(!settings.restore_session());
        assert_eq!(settings.get(KEY_RESTORE_SESSION), Ok("false".to_string()));
    }

    #[test]
    fn an_unknown_key_is_refused_for_reads_and_writes() {
        let mut settings = BrowserSettings::new();
        assert_eq!(settings.get("browser.zoom"), Err(Error::UnknownKey("browser.zoom".to_string())));
        assert_eq!(settings.set("browser.zoom", "1"), Err(Error::UnknownKey("browser.zoom".to_string())));
        assert_eq!(settings.homepage(), "");
        assert_eq!(KEYS.len(), 3);
    }
}
