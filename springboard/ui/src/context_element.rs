//! What a right click landed on, and which menu to show for it (the reference SDK's context menu element info).
//! Pure model: the web engine reports the chain of elements under the pointer, innermost first, and this module reads
//! the link, the image and the editable state out of that chain. Nothing here opens a menu.

/// One element under the pointer, as the engine reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitElement {
    /// Tag name, for example `a`, `img` or `input`. Matched without regard to case.
    pub tag: String,
    /// The `href` attribute, if the element has one.
    pub href: Option<String>,
    /// The `src` attribute, if the element has one.
    pub src: Option<String>,
    /// True for text fields and content-editable elements.
    pub editable: bool,
}

/// The menu a right click should show. Earlier variants win when several apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    /// The pointer is over editable content: show the text menu.
    Editable,
    /// The pointer is over a link: show the link menu.
    Link,
    /// The pointer is over an image: show the image menu.
    Image,
    /// Neither: show the page menu.
    Page,
}

/// The facts a context menu needs about the element under the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextElement {
    link_url: Option<String>,
    image_url: Option<String>,
    editable: bool,
}

impl ContextElement {
    /// Reads the chain (innermost element first). The link is the nearest anchor with a usable address, the image is
    /// the nearest image with a usable address, and `editable` is the innermost element's state.
    pub fn from_chain(chain: &[HitElement]) -> ContextElement {
        let link_url = chain
            .iter()
            .filter(|e| e.tag.eq_ignore_ascii_case("a"))
            .find_map(|e| e.href.as_deref().and_then(web_address));
        let image_url = chain
            .iter()
            .filter(|e| e.tag.eq_ignore_ascii_case("img"))
            .find_map(|e| e.src.as_deref().and_then(web_address));
        let editable = chain.first().map(|e| e.editable).unwrap_or(false);
        ContextElement { link_url: link_url.map(str::to_string), image_url: image_url.map(str::to_string), editable }
    }

    /// The link address, if the pointer is over a link.
    pub fn link_url(&self) -> Option<&str> {
        self.link_url.as_deref()
    }

    /// The image address, if the pointer is over an image.
    pub fn image_url(&self) -> Option<&str> {
        self.image_url.as_deref()
    }

    /// True when the innermost element accepts typed text.
    pub fn is_editable(&self) -> bool {
        self.editable
    }

    /// The menu to show: editable content first, then a link, then an image, otherwise the page menu.
    pub fn menu(&self) -> MenuTarget {
        if self.editable {
            MenuTarget::Editable
        } else if self.link_url.is_some() {
            MenuTarget::Link
        } else if self.image_url.is_some() {
            MenuTarget::Image
        } else {
            MenuTarget::Page
        }
    }
}

/// Returns the trimmed address when it is one of the schemes a menu may open (`http`, `https`, `mailto`), and `None`
/// for anything else, such as a script address or a data address. The scheme is matched without regard to case.
fn web_address(raw: &str) -> Option<&str> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    let ok = lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:");
    if ok { Some(s) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(tag: &str, href: Option<&str>, src: Option<&str>, editable: bool) -> HitElement {
        HitElement {
            tag: tag.to_string(),
            href: href.map(str::to_string),
            src: src.map(str::to_string),
            editable,
        }
    }

    #[test]
    fn link_is_the_nearest_anchor_with_a_usable_address() {
        let chain = [el("span", None, None, false), el("a", Some("https://a.example/x"), None, false), el("div", None, None, false)];
        let c = ContextElement::from_chain(&chain);
        assert_eq!(c.link_url(), Some("https://a.example/x"));
        assert_eq!(c.menu(), MenuTarget::Link);
    }

    #[test]
    fn an_anchor_with_a_script_address_is_skipped_for_an_outer_one() {
        let chain = [el("a", Some("javascript:void(0)"), None, false), el("a", Some("  http://b.example  "), None, false)];
        let c = ContextElement::from_chain(&chain);
        assert_eq!(c.link_url(), Some("http://b.example"));
    }

    #[test]
    fn mailto_is_accepted_and_the_scheme_ignores_case() {
        let chain = [el("A", Some("MAILTO:me@example.com"), None, false)];
        assert_eq!(ContextElement::from_chain(&chain).link_url(), Some("MAILTO:me@example.com"));
    }

    #[test]
    fn image_is_read_and_data_addresses_are_refused() {
        let chain = [el("IMG", None, Some("https://i.example/p.png"), false)];
        let c = ContextElement::from_chain(&chain);
        assert_eq!(c.image_url(), Some("https://i.example/p.png"));
        assert_eq!(c.menu(), MenuTarget::Image);
        let data = [el("img", None, Some("data:image/png;base64,AAAA"), false)];
        let d = ContextElement::from_chain(&data);
        assert_eq!(d.image_url(), None);
        assert_eq!(d.menu(), MenuTarget::Page);
    }

    #[test]
    fn editable_innermost_element_wins_the_menu_but_keeps_the_link() {
        let chain = [el("input", None, None, true), el("a", Some("https://c.example"), None, false)];
        let c = ContextElement::from_chain(&chain);
        assert!(c.is_editable());
        assert_eq!(c.menu(), MenuTarget::Editable);
        assert_eq!(c.link_url(), Some("https://c.example"));
    }

    #[test]
    fn empty_chain_gives_the_page_menu() {
        let c = ContextElement::from_chain(&[]);
        assert_eq!(c.link_url(), None);
        assert_eq!(c.image_url(), None);
        assert!(!c.is_editable());
        assert_eq!(c.menu(), MenuTarget::Page);
    }
}
