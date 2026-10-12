//! A standard in-app browser sheet as a pure model. It holds the address the sheet opened, the current address after
//! redirects, the reader and dismiss settings, the bar tints, and the events the sheet reports to its owner: the
//! initial load finished, a redirect during that load, the sheet dismissed, and the page sent to the full browser.
//! The caller draws the chrome and loads the page. Pure model: no web view and no window. Only http and https
//! addresses with a host can be shown, and the sheet reports each event once.

/// The label and meaning of the button that closes the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissStyle {
    Done,
    Close,
    Cancel,
}

impl DismissStyle {
    /// The text the button shows.
    pub fn title(self) -> &'static str {
        match self {
            DismissStyle::Done => "Done",
            DismissStyle::Close => "Close",
            DismissStyle::Cancel => "Cancel",
        }
    }
}

/// How the sheet is set up before it opens. The defaults are: no reader, collapsing bar on, a Done button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    /// Enter reader view when the page offers one.
    pub enters_reader_if_available: bool,
    /// Let the navigation bar collapse while the page scrolls.
    pub bar_collapsing_enabled: bool,
    pub dismiss_style: DismissStyle,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration { enters_reader_if_available: false, bar_collapsing_enabled: true, dismiss_style: DismissStyle::Done }
    }
}

/// An event for the owner of the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The first page finished loading. `success` is false when it failed.
    InitialLoadCompleted { success: bool },
    /// The first load was redirected to another address.
    Redirected { url: String },
    /// The person closed the sheet. Control goes back to the owner.
    Dismissed,
    /// The person chose to open the page in the full browser. The sheet stays open until it is dismissed.
    OpenedInBrowser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Only http and https addresses with a host can be shown.
    UnsupportedUrl,
}

#[derive(Debug)]
pub struct BrowserSheet {
    url: String,
    current: String,
    config: Configuration,
    loaded: bool,
    dismissed: bool,
    bar_tint: Option<String>,
    control_tint: Option<String>,
}

impl BrowserSheet {
    /// A sheet that will show `url`. Fails for any address `supports` refuses.
    pub fn new(url: &str, config: Configuration) -> Result<Self, Error> {
        if !Self::supports(url) {
            return Err(Error::UnsupportedUrl);
        }
        Ok(BrowserSheet {
            url: url.to_string(),
            current: url.to_string(),
            config,
            loaded: false,
            dismissed: false,
            bar_tint: None,
            control_tint: None,
        })
    }

    /// Whether `url` can be shown: an http or https address with a host, and no spaces or control characters.
    /// The scheme is matched without regard to case.
    pub fn supports(url: &str) -> bool {
        let rest = if url.get(..8).map_or(false, |s| s.eq_ignore_ascii_case("https://")) {
            &url[8..]
        } else if url.get(..7).map_or(false, |s| s.eq_ignore_ascii_case("http://")) {
            &url[7..]
        } else {
            return false;
        };
        let host = rest.split(|c: char| c == '/' || c == '?' || c == '#').next().unwrap_or("");
        !host.is_empty() && !url.chars().any(|c| c.is_whitespace() || c.is_control())
    }

    /// The address the sheet was opened with.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The address shown now: the start address, or the last redirect or visit.
    pub fn current_url(&self) -> &str {
        &self.current
    }

    pub fn configuration(&self) -> &Configuration {
        &self.config
    }

    /// The text of the close button.
    pub fn dismiss_title(&self) -> &'static str {
        self.config.dismiss_style.title()
    }

    /// Whether the first load has finished.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Whether the sheet is still open.
    pub fn is_open(&self) -> bool {
        !self.dismissed
    }

    /// Whether the sheet shows reader view for a page that offers one: the setting is on and the page has a reader.
    pub fn enters_reader(&self, page_has_reader: bool) -> bool {
        self.config.enters_reader_if_available && page_has_reader
    }

    /// Sets the bar tint and the control tint. Each is a colour token name that the caller resolves; `None` keeps the
    /// system colour.
    pub fn set_tints(&mut self, bar: Option<&str>, control: Option<&str>) {
        self.bar_tint = bar.map(str::to_string);
        self.control_tint = control.map(str::to_string);
    }

    pub fn bar_tint(&self) -> Option<&str> {
        self.bar_tint.as_deref()
    }

    pub fn control_tint(&self) -> Option<&str> {
        self.control_tint.as_deref()
    }

    /// Reports that the first load ended. Returns the event once; later calls return `None`.
    pub fn finish_initial_load(&mut self, success: bool) -> Option<Event> {
        if self.loaded || self.dismissed {
            return None;
        }
        self.loaded = true;
        Some(Event::InitialLoadCompleted { success })
    }

    /// Reports a redirect. During the first load it returns the event. After that the new address only becomes the
    /// current one. An address that cannot be shown is refused and leaves the sheet unchanged.
    pub fn redirect(&mut self, url: &str) -> Result<Option<Event>, Error> {
        if !Self::supports(url) {
            return Err(Error::UnsupportedUrl);
        }
        self.current = url.to_string();
        if self.loaded || self.dismissed {
            return Ok(None);
        }
        Ok(Some(Event::Redirected { url: url.to_string() }))
    }

    /// Records a visit to a new page inside the sheet. The same rule as `redirect` applies to the address.
    pub fn visit(&mut self, url: &str) -> Result<(), Error> {
        if !Self::supports(url) {
            return Err(Error::UnsupportedUrl);
        }
        self.current = url.to_string();
        Ok(())
    }

    /// Closes the sheet. Returns the event once; later calls return `None`.
    pub fn dismiss(&mut self) -> Option<Event> {
        if self.dismissed {
            return None;
        }
        self.dismissed = true;
        Some(Event::Dismissed)
    }

    /// Sends the current page to the full browser. Returns `None` once the sheet is closed.
    pub fn open_in_browser(&self) -> Option<Event> {
        if self.dismissed {
            None
        } else {
            Some(Event::OpenedInBrowser)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_addresses_with_a_host_open() {
        assert!(BrowserSheet::supports("https://olibuijr.com/a?b=1"));
        assert!(BrowserSheet::supports("HTTP://example.test"));
        assert!(!BrowserSheet::supports("file:///home/user/a.html"));
        assert!(!BrowserSheet::supports("https:///path-without-host"));
        assert!(!BrowserSheet::supports("https://has space.test/"));
        assert!(!BrowserSheet::supports("ftp://example.test/"));
        assert_eq!(BrowserSheet::new("ftp://x.test", Configuration::default()).unwrap_err(), Error::UnsupportedUrl);
    }

    #[test]
    fn the_initial_load_is_reported_once() {
        let mut sheet = BrowserSheet::new("https://a.test/", Configuration::default()).unwrap();
        assert!(!sheet.is_loaded());
        assert_eq!(sheet.finish_initial_load(false), Some(Event::InitialLoadCompleted { success: false }));
        assert!(sheet.is_loaded());
        assert_eq!(sheet.finish_initial_load(true), None);
    }

    #[test]
    fn a_redirect_during_the_first_load_is_an_event_and_later_only_moves_the_address() {
        let mut sheet = BrowserSheet::new("https://a.test/", Configuration::default()).unwrap();
        assert_eq!(
            sheet.redirect("https://b.test/landing"),
            Ok(Some(Event::Redirected { url: "https://b.test/landing".to_string() }))
        );
        assert_eq!(sheet.current_url(), "https://b.test/landing");
        assert_eq!(sheet.url(), "https://a.test/");
        sheet.finish_initial_load(true);
        assert_eq!(sheet.redirect("https://c.test/"), Ok(None));
        assert_eq!(sheet.current_url(), "https://c.test/");
        assert_eq!(sheet.redirect("file:///x"), Err(Error::UnsupportedUrl));
        assert_eq!(sheet.current_url(), "https://c.test/");
    }

    #[test]
    fn dismissing_reports_once_and_closes_the_full_browser_hand_off() {
        let mut sheet = BrowserSheet::new("https://a.test/", Configuration::default()).unwrap();
        assert_eq!(sheet.open_in_browser(), Some(Event::OpenedInBrowser));
        assert!(sheet.is_open());
        assert_eq!(sheet.dismiss(), Some(Event::Dismissed));
        assert!(!sheet.is_open());
        assert_eq!(sheet.dismiss(), None);
        assert_eq!(sheet.open_in_browser(), None);
        assert_eq!(sheet.finish_initial_load(true), None);
    }

    #[test]
    fn the_close_button_title_follows_the_dismiss_style() {
        let mut config = Configuration::default();
        assert_eq!(config.dismiss_style, DismissStyle::Done);
        assert_eq!(BrowserSheet::new("https://a.test/", config.clone()).unwrap().dismiss_title(), "Done");
        config.dismiss_style = DismissStyle::Close;
        assert_eq!(BrowserSheet::new("https://a.test/", config.clone()).unwrap().dismiss_title(), "Close");
        config.dismiss_style = DismissStyle::Cancel;
        assert_eq!(BrowserSheet::new("https://a.test/", config).unwrap().dismiss_title(), "Cancel");
    }

    #[test]
    fn reader_needs_both_the_setting_and_a_page_that_offers_it() {
        let on = Configuration { enters_reader_if_available: true, ..Configuration::default() };
        let sheet_on = BrowserSheet::new("https://a.test/", on).unwrap();
        assert!(sheet_on.enters_reader(true));
        assert!(!sheet_on.enters_reader(false));
        let sheet_off = BrowserSheet::new("https://a.test/", Configuration::default()).unwrap();
        assert!(!sheet_off.enters_reader(true));
        assert!(sheet_off.configuration().bar_collapsing_enabled);
    }

    #[test]
    fn tints_are_optional_token_names() {
        let mut sheet = BrowserSheet::new("https://a.test/", Configuration::default()).unwrap();
        assert_eq!(sheet.bar_tint(), None);
        sheet.set_tints(Some("LINK"), None);
        assert_eq!(sheet.bar_tint(), Some("LINK"));
        assert_eq!(sheet.control_tint(), None);
        sheet.set_tints(None, Some("DESTRUCTIVE"));
        assert_eq!(sheet.bar_tint(), None);
        assert_eq!(sheet.control_tint(), Some("DESTRUCTIVE"));
    }
}
