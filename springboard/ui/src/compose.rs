//! Post composition model (the compose sheet's state, see `docs/parity/social.md`): the text, images and links a user
//! has put into a post before it is sent, and the result the sheet reports when it closes.
//!
//! Only the content is modelled here. Showing the sheet, posting to a network and the service list belong to the shell
//! and to services that do not exist in BifrOSt. [`ComposeDraft::items`] gives the parts in the order a share hand-off
//! expects (text, then images, then links), so the shell can pass them to the share menu.

use std::fmt;

/// At most this many images and this many links per post. The framework documents no limit; this matches the share menu's
/// cap of 8 items.
pub const MAX_IMAGES: usize = 8;
pub const MAX_URLS: usize = 8;

/// One part of a draft, in hand-off order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeItem {
    Text(String),
    Image(String),
    Url(String),
}

/// What the compose sheet reports when it is dismissed: `Done` when the post is handed off to be sent, `Cancelled` when
/// it is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeResult {
    Cancelled,
    Done,
}

/// Why a draft refused a change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// An image or link with no content.
    Empty,
    /// The link is not an `http://` or `https://` URL with a host.
    BadUrl(String),
    /// The post already holds the maximum number of images (`MAX_IMAGES`) or links (`MAX_URLS`).
    Full(usize),
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComposeError::Empty => write!(f, "The item is empty"),
            ComposeError::BadUrl(u) => write!(f, "Not a web address: {u}"),
            ComposeError::Full(n) => write!(f, "The post already has {n} items of this kind"),
        }
    }
}

/// The content of a post being composed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeDraft {
    text: String,
    images: Vec<String>,
    urls: Vec<String>,
}

impl ComposeDraft {
    pub fn new() -> Self {
        Self::default()
    }

    /// `setInitialText(_:)`: replaces the text of the post.
    pub fn set_initial_text(&mut self, text: &str) {
        self.text = text.to_string();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// `add(_:)` for an image. `name` is whatever identifies the image to the shell (a document grant or a file name).
    pub fn add_image(&mut self, name: &str) -> Result<(), ComposeError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ComposeError::Empty);
        }
        if self.images.len() >= MAX_IMAGES {
            return Err(ComposeError::Full(MAX_IMAGES));
        }
        self.images.push(name.to_string());
        Ok(())
    }

    /// `add(_:)` for a URL. Only `http://` and `https://` links with a host are accepted; the scheme match ignores case.
    pub fn add_url(&mut self, url: &str) -> Result<(), ComposeError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(ComposeError::Empty);
        }
        if !is_web_url(url) {
            return Err(ComposeError::BadUrl(url.to_string()));
        }
        if self.urls.len() >= MAX_URLS {
            return Err(ComposeError::Full(MAX_URLS));
        }
        self.urls.push(url.to_string());
        Ok(())
    }

    /// `removeAllImages()`.
    pub fn remove_all_images(&mut self) {
        self.images.clear();
    }

    /// `removeAllURLs()`.
    pub fn remove_all_urls(&mut self) {
        self.urls.clear();
    }

    pub fn images(&self) -> &[String] {
        &self.images
    }

    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// True when there is nothing to post: blank text and no images or links.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.images.is_empty() && self.urls.is_empty()
    }

    /// The draft as hand-off parts: the text (when not blank), then each image, then each link.
    pub fn items(&self) -> Vec<ComposeItem> {
        let mut out = Vec::new();
        if !self.text.trim().is_empty() {
            out.push(ComposeItem::Text(self.text.clone()));
        }
        out.extend(self.images.iter().cloned().map(ComposeItem::Image));
        out.extend(self.urls.iter().cloned().map(ComposeItem::Url));
        out
    }

    /// The result when the sheet closes. A post is handed off (`Done`) only when the user sends it and it has content;
    /// otherwise the sheet reports `Cancelled`.
    pub fn dismiss(&self, send: bool) -> ComposeResult {
        if send && !self.is_empty() {
            ComposeResult::Done
        } else {
            ComposeResult::Cancelled
        }
    }
}

/// `http://` or `https://` (any case) followed by a host with no spaces.
fn is_web_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !url.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_replaced_not_appended() {
        let mut d = ComposeDraft::new();
        d.set_initial_text("first");
        d.set_initial_text("second");
        assert_eq!(d.text(), "second");
    }

    #[test]
    fn only_web_links_are_accepted() {
        let mut d = ComposeDraft::new();
        assert!(d.add_url("https://example.test/a?b=1").is_ok());
        assert!(d.add_url("HTTP://example.test").is_ok());
        assert_eq!(d.add_url("ftp://example.test"), Err(ComposeError::BadUrl("ftp://example.test".into())));
        assert_eq!(d.add_url("https://"), Err(ComposeError::BadUrl("https://".into())));
        assert_eq!(d.add_url("https://exa mple.test"), Err(ComposeError::BadUrl("https://exa mple.test".into())));
        assert_eq!(d.add_url("  "), Err(ComposeError::Empty));
        assert_eq!(d.urls().len(), 2);
    }

    #[test]
    fn images_and_links_stop_at_their_limit() {
        let mut d = ComposeDraft::new();
        for i in 0..MAX_IMAGES {
            d.add_image(&format!("img{i}.png")).unwrap();
        }
        assert_eq!(d.add_image("one-more.png"), Err(ComposeError::Full(MAX_IMAGES)));
        for i in 0..MAX_URLS {
            d.add_url(&format!("https://example.test/{i}")).unwrap();
        }
        assert_eq!(d.add_url("https://example.test/extra"), Err(ComposeError::Full(MAX_URLS)));
        d.remove_all_images();
        assert!(d.images().is_empty());
        assert_eq!(d.urls().len(), MAX_URLS);
        d.remove_all_urls();
        assert!(d.urls().is_empty());
    }

    #[test]
    fn items_come_out_text_then_images_then_links() {
        let mut d = ComposeDraft::new();
        d.add_url("https://example.test").unwrap();
        d.add_image("a.png").unwrap();
        d.set_initial_text("hello");
        assert_eq!(
            d.items(),
            vec![ComposeItem::Text("hello".into()), ComposeItem::Image("a.png".into()), ComposeItem::Url("https://example.test".into())]
        );
        d.set_initial_text("   ");
        assert_eq!(d.items().len(), 2);
    }

    #[test]
    fn dismiss_is_done_only_for_a_sent_post_with_content() {
        let mut d = ComposeDraft::new();
        assert_eq!(d.dismiss(true), ComposeResult::Cancelled);
        d.set_initial_text("hi");
        assert_eq!(d.dismiss(false), ComposeResult::Cancelled);
        assert_eq!(d.dismiss(true), ComposeResult::Done);
    }

    #[test]
    fn a_draft_with_only_a_link_is_not_empty() {
        let mut d = ComposeDraft::new();
        assert!(d.is_empty());
        d.add_url("https://example.test").unwrap();
        assert!(!d.is_empty());
    }
}
