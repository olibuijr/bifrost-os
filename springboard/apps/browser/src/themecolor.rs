//! The page's own colour under the toolbar (Browser tints its toolbar with the page's `theme-color`, else with the
//! background at the top of the page). Servo 0.7.0 reports no theme colour, so a script asks for it after each load:
//! the page's `<meta name="theme-color">` (the one whose `media` matches), else the first opaque background found
//! walking up from the element at the top of the page. The answer is `bifrost-theme:r,g,b`, or `bifrost-theme:` when
//! the page names none (a transparent page keeps the system toolbar).
//!
//! The tint is only ever drawn under the toolbar band. The glass groups and the address field keep their own plates,
//! so no text sits directly on the page colour and contrast does not depend on it.
use std::collections::HashMap;

use bifrost_app::webview::WebViewId;

use crate::Browser;

/// Reply prefix of [`THEME_JS`].
pub const PREFIX: &str = "bifrost-theme:";

/// Alpha of the tint over the toolbar band (0..255): the page colour with a trace of the material behind it.
pub const TINT_ALPHA: u8 = 214;

/// One line (the engine's `eval` verb carries no newlines). A colour goes through a throwaway element so any CSS
/// colour syntax (names, hex, `rgb()`) reads back as the computed `rgb(...)`; one that is not a colour, or is
/// mostly transparent, counts as none.
pub const THEME_JS: &str = concat!(
    r"(function(){try{",
    r"function p(v){var m=v&&v.indexOf('rgb')===0&&v.match(/[\d.]+/g);if(!m||m.length<3||(m.length>3&&+m[3]<0.5))return '';return Math.round(m[0])+','+Math.round(m[1])+','+Math.round(m[2])}",
    r"function n(c){var d=document.createElement('div');d.style.color=c;if(!d.style.color)return '';document.documentElement.appendChild(d);var v=getComputedStyle(d).color;d.remove();return p(v)}",
    r"var r='',ms=document.querySelectorAll('meta[name=theme-color]');",
    r"for(var i=0;i<ms.length&&!r;i++){var q=ms[i];if(!q.media||matchMedia(q.media).matches)r=n(q.getAttribute('content')||'')}",
    r"if(!r){var e=document.elementFromPoint(Math.max(1,innerWidth/2),1)||document.body;while(e&&!r){r=p(getComputedStyle(e).backgroundColor);e=e.parentElement}}",
    r"return 'bifrost-theme:'+r}catch(x){return 'bifrost-theme:'}})()",
);

/// The colour in an eval reply: `bifrost-theme:34,59,76`, bare or inside the engine's quoting. None for the empty
/// answer and for anything that is not three channels.
pub fn parse(text: &str) -> Option<(u8, u8, u8)> {
    let at = text.find(PREFIX)?;
    let rest = text[at + PREFIX.len()..].split('"').next()?;
    let mut channels = rest.split(',').map(|c| c.trim().parse::<u8>());
    let r = channels.next()?.ok()?;
    let g = channels.next()?.ok()?;
    let b = channels.next()?.ok()?;
    channels.next().is_none().then_some((r, g, b))
}

/// The theme colour last read from each tab's page, with the site it was read on. A colour only applies while the
/// tab stays on that site, so it never outlives a navigation to another one (nor an engine restart that reuses ids).
#[derive(Default)]
pub struct ThemeColors {
    by_view: HashMap<WebViewId, (String, (u8, u8, u8))>,
}

impl ThemeColors {
    /// Takes the answer to [`THEME_JS`] for `view`, whose page is at `url`. A page without a colour (or a URL
    /// without a site) clears the tab's entry.
    pub fn on_reply(&mut self, view: WebViewId, url: &str, text: &str) {
        match (crate::favicon::host_key(url), parse(text)) {
            (Some(site), Some(rgb)) => {
                self.by_view.insert(view, (site, rgb));
            }
            _ => {
                self.by_view.remove(&view);
            }
        }
    }

    /// The colour for `view` showing `url`: the one read on the same site.
    pub fn get(&self, view: WebViewId, url: &str) -> Option<(u8, u8, u8)> {
        let (site, rgb) = self.by_view.get(&view)?;
        (crate::favicon::host_key(url).as_deref() == Some(site.as_str())).then_some(*rgb)
    }

    pub fn forget(&mut self, view: WebViewId) {
        self.by_view.remove(&view);
    }

    pub fn clear(&mut self) {
        self.by_view.clear();
    }
}

impl Browser {
    /// The colour that tints the toolbar for the selected tab: its page's. A private window keeps its dark toolbar.
    pub(super) fn toolbar_tint(&self) -> Option<(u8, u8, u8)> {
        if self.private {
            return None;
        }
        let view = self.view()?;
        let state = self.pool.as_ref()?.state(view)?;
        self.themes.get(view, &state.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_is_one_line_and_answers_with_the_prefix() {
        assert!(!THEME_JS.contains('\n'));
        assert!(THEME_JS.contains(PREFIX));
    }

    #[test]
    fn a_reply_parses_bare_and_inside_quotes() {
        assert_eq!(parse("bifrost-theme:34,59,76"), Some((34, 59, 76)));
        assert_eq!(parse("\"bifrost-theme:0,0,0\""), Some((0, 0, 0)));
        assert_eq!(parse("String(\"bifrost-theme:255, 128, 1\")"), Some((255, 128, 1)));
    }

    #[test]
    fn no_colour_is_none() {
        assert_eq!(parse("bifrost-theme:"), None);
        assert_eq!(parse("\"bifrost-theme:\""), None);
        assert_eq!(parse("bifrost-theme:1,2"), None);
        assert_eq!(parse("bifrost-theme:1,2,3,4"), None);
        assert_eq!(parse("bifrost-theme:300,2,3"), None);
        assert_eq!(parse("bifrost-form:1"), None);
    }

    #[test]
    fn a_colour_applies_only_on_the_site_it_was_read_on() {
        let mut t = ThemeColors::default();
        t.on_reply(7, "https://example.com/a", "bifrost-theme:10,20,30");
        assert_eq!(t.get(7, "https://example.com/b"), Some((10, 20, 30)));
        assert_eq!(t.get(7, "https://other.org/"), None);
        assert_eq!(t.get(8, "https://example.com/a"), None);
    }

    #[test]
    fn an_empty_answer_or_a_page_without_a_site_clears_the_colour() {
        let mut t = ThemeColors::default();
        t.on_reply(1, "https://example.com/", "bifrost-theme:1,2,3");
        t.on_reply(1, "https://example.com/", "bifrost-theme:");
        assert_eq!(t.get(1, "https://example.com/"), None);
        t.on_reply(2, "https://example.com/", "bifrost-theme:1,2,3");
        t.on_reply(2, "about:blank", "bifrost-theme:1,2,3");
        assert_eq!(t.get(2, "https://example.com/"), None);
        t.on_reply(3, "https://example.com/", "bifrost-theme:1,2,3");
        t.forget(3);
        assert_eq!(t.get(3, "https://example.com/"), None);
        t.on_reply(4, "https://example.com/", "bifrost-theme:1,2,3");
        t.clear();
        assert_eq!(t.get(4, "https://example.com/"), None);
    }
}
