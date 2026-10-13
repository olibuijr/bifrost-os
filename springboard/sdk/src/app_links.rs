//! Web site to app associations for apps: a site names the app that handles its paths, and a link opens in the enabled
//! app with the longest matching path prefix. The model lives in `bifrost_core::app_links`; this module re-exports it.

pub use bifrost_core::app_links::AppLinks;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claimed_path_resolves_to_its_app_and_an_unclaimed_site_does_not() {
        let mut links = AppLinks::new();
        links.claim("example.com", "Notes", &["/notes"]);
        assert_eq!(links.resolve("https://example.com/notes/1"), Some("Notes"));
        assert_eq!(links.resolve("https://example.com/shop"), None);
    }
}
