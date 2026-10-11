//! Reader (Browser: the Reader button in the address field, Cmd+Shift+R). A page script finds the article (an
//! `<article>`/`<main>` element or the block with the most paragraph text, scored by text length and link density),
//! the app renders its title, byline and body as a clean `data:` page with a system font stack and the window's
//! Light/Dark appearance, and loads it in the same view. Esc, the button or Cmd+Shift+R loads the original URL again.
//! Answers arrive through the token-correlated dev eval (a plain eval reply is capped at 4 KB).
//! Print (Cmd+P) is not possible: Servo 0.7.0 exposes no printing API (`WebView` has no print call), so there is no item.
use std::collections::HashMap;

use bifrost_app::controls::Rect;
use bifrost_app::webview::{WebProcessPool, WebViewId};
use bifrost_app::webview_dev::DevEvent;

/// Every reader page is a URL starting with this, so a loaded URL tells reader pages from ordinary ones.
pub const DATA_PREFIX: &str = "data:text/html;charset=utf-8,";
/// Fewest scored characters of paragraph text for a page to count as an article.
const MIN_CHARS: usize = 600;

/// Declares `s(e)` (paragraph text length of `e`, paragraphs in nav/aside/footer/header/form skipped, discounted by
/// link density) and picks `best`, the highest-scoring `article`, `main`, `section` or `div` (`<article>` x1.3).
macro_rules! pick {
    () => {
        r#"function s(e){var t=0,ps=e.querySelectorAll('p');for(var i=0;i<ps.length;i++){var p=ps[i];if(p.closest&&p.closest('nav,aside,footer,header,form'))continue;var n=(p.textContent||'').trim().length;if(n>=60)t+=n;}var l=0,as=e.querySelectorAll('a');for(var j=0;j<as.length;j++)l+=(as[j].textContent||'').length;var all=(e.textContent||'').length||1;return t*(1-Math.min(0.9,l/all));}var best=null,bs=0;var c=document.querySelectorAll('article,main,[role=main],section,div');for(var k=0;k<c.length&&k<3000;k++){var e=c[k],v=s(e);if(e.tagName=='ARTICLE')v*=1.3;if(v>bs){bs=v;best=e;}}"#
    };
}

/// Returns `'bfr:' + score` of the best block.
const FIND: &str = concat!("(function(){", pick!(), "return 'bfr:'+Math.round(bs);})()");

/// Returns `'bfr:' + JSON` with title, byline and the best block's body, reduced to text-level tags.
const EXTRACT: &str = concat!(
    "(function(){",
    pick!(),
    r#"if(!best)return 'bfr:';var n=best.cloneNode(true);var bad=n.querySelectorAll('script,style,noscript,nav,aside,footer,form,iframe,button,input,select,textarea,svg,canvas,video,audio,object,embed,dialog,[hidden],[aria-hidden=true]');for(var i=0;i<bad.length;i++)bad[i].remove();var keep={P:1,H1:1,H2:1,H3:1,H4:1,H5:1,H6:1,UL:1,OL:1,LI:1,BLOCKQUOTE:1,PRE:1,CODE:1,IMG:1,FIGURE:1,FIGCAPTION:1,A:1,EM:1,STRONG:1,B:1,I:1,BR:1,SUP:1,SUB:1,TABLE:1,THEAD:1,TBODY:1,TR:1,TD:1,TH:1,HR:1,SPAN:1,DIV:1,SECTION:1,ARTICLE:1,MAIN:1};var all=n.querySelectorAll('*');for(var j=all.length-1;j>=0;j--){var e=all[j];if(!keep[e.tagName]){e.replaceWith.apply(e,Array.prototype.slice.call(e.childNodes));continue;}var href=e.tagName=='A'?e.href:null,src=e.tagName=='IMG'?e.src:null,alt=e.getAttribute('alt');var a=e.attributes;for(var q=a.length-1;q>=0;q--)e.removeAttribute(a[q].name);if(href&&href.toLowerCase().indexOf('javascript:')!=0)e.setAttribute('href',href);if(src)e.setAttribute('src',src);if(alt)e.setAttribute('alt',alt);}var h=best.querySelector('h1')||document.querySelector('h1');var by=document.querySelector('[rel=author],[itemprop=author],.byline,.author,meta[name=author]');var byt=by?(by.content||by.textContent||'').trim().replace(/\s+/g,' '):'';if(h){var hh=n.querySelector('h1');if(hh&&hh.textContent==h.textContent)hh.remove();}return 'bfr:'+JSON.stringify({title:(h?h.textContent:document.title).trim().replace(/\s+/g,' '),byline:byt.slice(0,120),html:n.innerHTML});})()"#
);

#[derive(Default)]
pub struct Reader {
    avail: HashMap<WebViewId, bool>,
    /// Tabs showing the reader page, with the URL to return to.
    active: HashMap<WebViewId, String>,
    detect: Option<(u64, WebViewId)>,
    extract: Option<(u64, WebViewId, String, bool)>,
}

impl Reader {
    pub fn is_active(&self, v: WebViewId) -> bool {
        self.active.contains_key(&v)
    }

    /// The URL the address field shows while the reader page is up.
    pub fn original(&self, v: WebViewId) -> Option<&str> {
        self.active.get(&v).map(String::as_str)
    }

    /// The button shows on a page with an article, and on the reader page (to leave it).
    pub fn button(&self, v: WebViewId) -> bool {
        self.is_active(v) || self.avail.get(&v).copied().unwrap_or(false)
    }

    pub fn forget(&mut self, v: WebViewId) {
        self.avail.remove(&v);
        self.active.remove(&v);
    }

    /// A page finished loading: a reader page keeps its state, any other page is checked for an article.
    pub fn on_load(&mut self, pool: &mut WebProcessPool, v: WebViewId, url: &str) {
        if url.starts_with(DATA_PREFIX) && self.is_active(v) {
            return;
        }
        self.active.remove(&v);
        self.avail.insert(v, false);
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let t = bifrost_app::control::new_token();
        self.detect = Some((t, v));
        pool.dev_eval(v, t, FIND);
    }

    /// Cmd+Shift+R or the button: leave the reader page, or build it for the current page.
    pub fn toggle(&mut self, pool: &mut WebProcessPool, v: WebViewId, url: &str, dark: bool) {
        if self.exit(pool, v) {
            return;
        }
        if !self.avail.get(&v).copied().unwrap_or(false) || self.extract.is_some() {
            return;
        }
        let t = bifrost_app::control::new_token();
        self.extract = Some((t, v, url.to_string(), dark));
        pool.dev_eval(v, t, EXTRACT);
    }

    /// Esc: back to the original page; true when this tab was in reader.
    pub fn exit(&mut self, pool: &mut WebProcessPool, v: WebViewId) -> bool {
        match self.active.remove(&v) {
            Some(orig) => {
                pool.load(v, &orig);
                true
            }
            None => false,
        }
    }

    /// Takes the answers to the detect and extract scripts; true when `ev` was one of them (a redraw is needed).
    pub fn on_dev(&mut self, pool: &mut WebProcessPool, ev: &DevEvent) -> bool {
        let DevEvent::Eval { token, result } = ev else { return false };
        let text = result.as_ref().ok().and_then(|v| crate::control::inner_string(&v.ty, &v.json)).and_then(|s| s.strip_prefix("bfr:").map(String::from));
        if let Some((t, v)) = self.detect {
            if t == *token {
                self.detect = None;
                let n: usize = text.and_then(|s| s.parse().ok()).unwrap_or(0);
                self.avail.insert(v, n >= MIN_CHARS);
                return true;
            }
        }
        if let Some((t, v, url, dark)) = self.extract.clone() {
            if t == *token {
                self.extract = None;
                if let Some(a) = text.as_deref().and_then(parse_article) {
                    self.active.insert(v, url.clone());
                    pool.load(v, &data_url(&page(&a, &url, dark)));
                }
                return true;
            }
        }
        false
    }
}

pub struct Article {
    pub title: String,
    pub byline: String,
    pub html: String,
}

fn parse_article(raw: &str) -> Option<Article> {
    use bifrost_core::json::Json;
    let Ok(Json::Obj(o)) = bifrost_core::json::parse(raw) else { return None };
    let get = |k: &str| o.iter().find(|(n, _)| n == k).and_then(|(_, v)| if let Json::Str(s) = v { Some(s.clone()) } else { None });
    let html = get("html")?;
    (!html.trim().is_empty()).then(|| Article { title: get("title").unwrap_or_default(), byline: get("byline").unwrap_or_default(), html })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The reader page: system font stack, 680 px measure, Light and Dark colours chosen by the window's appearance.
pub fn page(a: &Article, url: &str, dark: bool) -> String {
    let (bg, fg, sub, link, rule, code) = if dark {
        ("#1e1e1e", "#e6e6e6", "#98989d", "#0a84ff", "#3a3a3c", "#2c2c2e")
    } else {
        ("#ffffff", "#1d1d1f", "#6e6e73", "#007aff", "#d2d2d7", "#f2f2f7")
    };
    let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("");
    let by = if a.byline.is_empty() { host.to_string() } else { format!("{} \u{b7} {}", a.byline, host) };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{t}</title><style>\
html{{background:{bg};color:{fg};color-scheme:{cs}}}body{{margin:0;padding:48px 24px 96px;font:19px/1.6 -apple-system,\"SF Pro Text\",system-ui,\"Helvetica Neue\",Helvetica,Arial,sans-serif}}\
main{{max-width:680px;margin:0 auto}}h1{{font-size:34px;line-height:1.2;margin:0 0 8px;font-weight:700}}.by{{color:{sub};font-size:14px;margin:0 0 32px}}\
h2,h3,h4{{line-height:1.3;margin:1.6em 0 .5em}}p{{margin:0 0 1em}}a{{color:{link}}}img{{max-width:100%;height:auto;border-radius:6px}}\
blockquote{{margin:1em 0;padding-left:16px;border-left:3px solid {rule};color:{sub}}}pre,code{{background:{code};border-radius:6px;font:15px ui-monospace,\"SF Mono\",Menlo,monospace}}\
pre{{padding:12px;overflow:auto}}code{{padding:1px 4px}}pre code{{padding:0}}hr{{border:0;border-top:1px solid {rule}}}table{{border-collapse:collapse}}td,th{{border:1px solid {rule};padding:4px 8px}}figcaption{{color:{sub};font-size:14px}}\
</style></head><body><main><h1>{t}</h1><p class=\"by\">{by}</p>{body}</main></body></html>",
        t = esc(&a.title),
        by = esc(&by),
        body = a.html,
        cs = if dark { "dark" } else { "light" },
    )
}

/// `data:` URL for `html` (every byte outside the unreserved set percent-encoded).
pub fn data_url(html: &str) -> String {
    let mut out = String::with_capacity(DATA_PREFIX.len() + html.len() * 2);
    out.push_str(DATA_PREFIX);
    for b in html.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The button's rectangle at the field's leading edge (points).
pub fn button_rect(field: Rect) -> Rect {
    Rect { x: field.x, y: field.y, w: field.h, h: field.h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_is_encoded_and_page_follows_appearance() {
        let u = data_url("<p>é 1</p>");
        assert!(u.starts_with(DATA_PREFIX) && !u.contains('<') && !u.contains(' '));
        let p = page(&Article { title: "A&B".into(), byline: String::new(), html: "<p>x</p>".into() }, "https://e.com/a", true);
        assert!(p.contains("A&amp;B") && p.contains("color-scheme:dark"));
    }

    #[test]
    fn article_parses() {
        let a = parse_article(r#"{"title":"T","byline":"","html":"<p>x</p>"}"#).unwrap();
        assert_eq!(a.title, "T");
        assert!(parse_article(r#"{"html":" "}"#).is_none());
    }
}
