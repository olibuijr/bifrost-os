//! Icons drawn with CSS masks. Servo 0.7.0 does not parse `mask`, `mask-image` or their `-webkit-` forms (stylo
//! gates them behind `layout.unimplemented`), so a page that paints an icon as `background-color: currentColor` cut
//! out by `mask-image: url(icon.svg)` (MediaWiki's `mw-ui-icon`, the Wikipedia header and language icons) shows the
//! whole box filled with the text colour: a solid black or blue square. The unsupported declaration is dropped at
//! parse time, so the CSSOM cannot reveal it; this page script reads the stylesheet source instead, finds the rules
//! that carry a masked `url(...)` and clears the background of their selectors, so the element is blank rather than
//! a square. It does nothing once the engine supports masks (`CSS.supports` is true).
use bifrost_app::web_config::{InjectionTime, UserScript};

/// Document-end, main-frame script (the SDK wrapper gives it its own function scope).
pub const MASK_FALLBACK_JS: &str = "if(window.__bmask)return;window.__bmask=1;\
try{if(CSS.supports('mask-image','url(x)')||CSS.supports('-webkit-mask-image','url(x)'))return}catch(e){}\
var DECL=/(?:^|[;\\s])(?:-webkit-)?mask(?:-image)?\\s*:[^;}]*url\\(/i,seen={},css='';\
function scan(t){t=String(t).replace(/\\/\\*[\\s\\S]*?\\*\\//g,'');var re=/([^{}]+)\\{([^{}]*)\\}/g,m;\
while((m=re.exec(t))){if(!DECL.test(m[2]))continue;m[1].split(',').forEach(function(s){s=s.trim();\
if(!s||s.charAt(0)==='@'||seen[s])return;seen[s]=1;css+=s+'{background:none!important}\\n'})}}\
function apply(){if(!css)return;var st=document.createElement('style');st.textContent=css;(document.head||document.documentElement).appendChild(st);css=''}\
var n=0,pending=0;\
function done(){if(--pending<=0)apply()}\
Array.prototype.forEach.call(document.querySelectorAll('style'),function(s){scan(s.textContent||'')});\
Array.prototype.forEach.call(document.querySelectorAll('link[rel~=stylesheet][href]'),function(l){\
if(n++>=24||!l.href)return;pending++;\
try{fetch(l.href,{credentials:'omit'}).then(function(r){return r.ok?r.text():''}).then(function(t){if(t.length<4000000)scan(t)}).catch(function(){}).then(done)}catch(e){done()}});\
if(!pending)apply();";

/// The script as an SDK user script, for `WebViewConfiguration`.
pub fn script() -> UserScript {
    UserScript::new(MASK_FALLBACK_JS, InjectionTime::AtDocumentEnd, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_runs_at_document_end_in_the_main_frame_and_defers_to_native_masks() {
        let s = script();
        assert_eq!(s.injection_time, InjectionTime::AtDocumentEnd);
        assert!(s.for_main_frame_only);
        assert!(s.source.contains("CSS.supports('mask-image'"));
        assert!(s.source.contains("background:none!important"));
    }
}
