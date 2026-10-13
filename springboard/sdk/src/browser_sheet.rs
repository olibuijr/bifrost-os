//! In-app browser sheet: a page shown inside an app, with a Done or Close control, that returns control to the app when
//! the person dismisses it. The model lives in `bifrost_ui::browser_sheet`; this module re-exports it for `bifrost_app`.
//! Nothing draws the page here: the shell presents the sheet, and the web view is the caller's to show.

pub use bifrost_ui::browser_sheet::{BrowserSheet, Configuration, DismissStyle, Error, Event};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_load_is_reported_once_through_the_sdk_path() {
        let mut sheet = BrowserSheet::new("https://example.com/", Configuration::default()).unwrap();
        assert_eq!(sheet.finish_initial_load(true), Some(Event::InitialLoadCompleted { success: true }));
        assert_eq!(sheet.finish_initial_load(true), None);
    }

    #[test]
    fn unsupported_address_is_refused() {
        assert_eq!(BrowserSheet::new("file:///etc/hosts", Configuration::default()).err(), Some(Error::UnsupportedUrl));
    }
}
