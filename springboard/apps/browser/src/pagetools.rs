//! Page-side scripts of the developer tools that are not the network log: the console collector (source and line
//! of `console.*` calls, uncaught errors and unhandled promise rejections), the Elements highlight overlay, the
//! node-at-point lookup behind Inspect Element and `browser.inspect`.
//!
//! Servo 0.7.0 reports `console.*` to the embedder as level and text only (`show_console_message`), so the page
//! side derives `file:line` from `new Error().stack` and queues it; the Inspector drains the queue and matches
//! each entry to the console line the engine already sent (same level and text) or, for events the engine does not
//! report (unhandled rejections), adds a line of its own.
//!
//! Nodes are addressed by a path of element indexes from `<html>`: `0` is the root, `0.1.3` its second child's
//! fourth child (`Element.children`, so text nodes do not count). `control::js_tree` emits the same path per node.
use bifrost_core::json::{self, Json};

/// Id of the highlight overlay element the page receives (the tree skips it).
pub const OVERLAY_ID: &str = "__bifrost_hl";

/// Installs the console collector (idempotent). Runs at document start with the network collector.
pub const CONSOLE_JS: &str = "(function(){if(window.__bc)return 0;var bc=window.__bc={q:[]};\
var RX=/([^\\s@()]+):(\\d+):(\\d+)\\)?\\s*$/;\
function loc(skip){try{var st=String(new Error().stack||'').split('\\n'),n=0;for(var i=0;i<st.length;i++){var m=RX.exec(st[i]);if(!m)continue;if(n++<skip)continue;return [m[1],+m[2]]}}catch(e){}return ['',0]}\
function fmt(a){try{if(typeof a==='string')return a;if(a instanceof Error)return String(a.name+': '+a.message);return typeof a==='object'&&a!==null?JSON.stringify(a):String(a)}catch(e){return String(a)}}\
function push(e){if(bc.q.length>=500)bc.q.shift();bc.q.push(e)}\
['log','info','warn','error','debug'].forEach(function(k){var o=console[k];if(typeof o!=='function')return;\
console[k]=function(){try{var l=loc(2);push({l:k,t:Array.prototype.map.call(arguments,fmt).join(' ').slice(0,4000),s:l[0],n:l[1]})}catch(e){}return o.apply(this,arguments)}});\
window.addEventListener('unhandledrejection',function(e){var r=e.reason,t,s='',n=0;try{t=r&&r.message?String(r.name||'Error')+': '+r.message:String(r);var m=RX.exec(String(r&&r.stack||'').split('\\n').filter(function(x){return RX.test(x)})[0]||'');if(m){s=m[1];n=+m[2]}}catch(x){t=String(r)}\
push({l:'error',t:'Unhandled Promise Rejection: '+String(t).slice(0,4000),s:s,n:n,own:1})});\
window.addEventListener('error',function(e){if(e.target&&e.target!==window)return;push({l:'error',t:String(e.message||'').slice(0,4000),s:String(e.filename||''),n:e.lineno|0})});\
return 1})()";

/// Installs the collector if needed, then returns (and removes) the page's queued console entries as JSON.
pub fn drain_console_js() -> String {
    format!("{CONSOLE_JS};(function(){{var bc=window.__bc;return JSON.stringify(bc?bc.q.splice(0):[])}})()")
}

/// One console entry captured in the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Cap {
    pub level: String,
    pub text: String,
    pub source: String,
    pub line: u32,
    /// the engine does not report this one: the app adds it
    pub own: bool,
}

/// Reads the JSON string `drain_console_js` returns. Unknown levels become `log`.
pub fn parse_caps(raw: &str) -> Vec<Cap> {
    let Ok(Json::Arr(items)) = json::parse(raw) else { return Vec::new() };
    items
        .iter()
        .filter_map(|v| {
            let level = v.get("l").and_then(Json::as_str)?;
            let level = if matches!(level, "log" | "info" | "warn" | "error" | "debug") { level } else { "log" };
            Some(Cap {
                level: level.to_string(),
                text: v.get("t").and_then(Json::as_str).unwrap_or("").to_string(),
                source: v.get("s").and_then(Json::as_str).unwrap_or("").to_string(),
                line: v.get("n").and_then(Json::as_u64).unwrap_or(0) as u32,
                own: matches!(v.get("own"), Some(Json::Num(n)) if *n != 0.0),
            })
        })
        .collect()
}

/// True when the engine's text and the page's text describe the same message (the engine may format arguments
/// differently, so a shared prefix counts).
pub fn same_text(engine: &str, page: &str) -> bool {
    let (a, b) = (engine.trim(), page.trim());
    !a.is_empty() && !b.is_empty() && (a == b || a.starts_with(b) || b.starts_with(a))
}

// ---- Elements: highlight and node info ----

/// Draws (or, with an empty `path`, removes) the highlight overlay for the node at `path` and returns its info as a
/// JSON string `{tag,id,class,x,y,w,h,attrs}` (x, y in document CSS px), `{error}` when the node is gone, or `null`
/// when removing. The overlay is a fixed, pointer-transparent `div` that follows scroll and resize.
pub fn js_highlight(path: &str) -> String {
    format!(
        "(function(p){{var h=document.getElementById('{OVERLAY_ID}');if(h)h.remove();\
         if(window.__bhl){{window.removeEventListener('scroll',window.__bhl,true);window.removeEventListener('resize',window.__bhl);window.__bhl=null}}\
         if(!p)return 'null';var e=document.documentElement,a=String(p).split('.').slice(1);\
         for(var i=0;i<a.length&&e;i++)e=e.children[+a[i]];if(!e)return JSON.stringify({{error:'the node is gone'}});\
         var d=document.createElement('div');d.id='{OVERLAY_ID}';\
         d.style.cssText='position:fixed;pointer-events:none;z-index:2147483647;box-sizing:border-box;margin:0;padding:0;border:2px solid rgb(0,122,255);background:rgba(0,122,255,0.2)';\
         function pos(){{var r=e.getBoundingClientRect();d.style.left=r.left+'px';d.style.top=r.top+'px';d.style.width=r.width+'px';d.style.height=r.height+'px'}}\
         pos();(document.body||document.documentElement).appendChild(d);window.__bhl=pos;\
         window.addEventListener('scroll',pos,true);window.addEventListener('resize',pos);\
         var r=e.getBoundingClientRect(),at={{}};for(var j=0;j<e.attributes.length;j++)at[e.attributes[j].name]=String(e.attributes[j].value).slice(0,200);\
         return JSON.stringify({{tag:e.nodeName.toLowerCase(),id:e.id||'',class:typeof e.className==='string'?e.className:'',\
         x:r.left+window.scrollX,y:r.top+window.scrollY,w:r.width,h:r.height,attrs:at}})}})({})",
        json::quote(path)
    )
}

/// The overlay removal script.
pub fn js_clear_highlight() -> String {
    js_highlight("")
}

/// The element under the page point `x, y` (CSS px of the viewport) as `{path,tag,id,class,selector,rect{x,y,w,h}}`;
/// `null` when nothing is there (outside the viewport, say). The overlay is pointer-transparent, so hit testing skips it.
pub fn js_inspect_point(x: f32, y: f32) -> String {
    format!(
        "(function(x,y){{var e=document.elementFromPoint(x,y);if(!e)return null;var path=[],n=e,rt=document.documentElement;\
         while(n&&n!==rt){{var par=n.parentElement;if(!par)return null;path.unshift(Array.prototype.indexOf.call(par.children,n));n=par}}\
         if(n!==rt)return null;var sel=e.nodeName.toLowerCase()+(e.id?'#'+e.id:'')+(typeof e.className==='string'&&e.className.trim()?'.'+e.className.trim().split(/\\s+/).join('.'):'');\
         var r=e.getBoundingClientRect();\
         return JSON.stringify({{path:['0'].concat(path).join('.'),tag:e.nodeName.toLowerCase(),id:e.id||'',class:typeof e.className==='string'?e.className:'',selector:sel,rect:{{x:r.left,y:r.top,w:r.width,h:r.height}}}})}})({x},{y})"
    )
}

/// What the Elements panel shows for the selected node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeInfo {
    pub tag: String,
    pub id: String,
    pub class: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub attrs: Vec<(String, String)>,
}

/// Reads the reply of `js_highlight`. `Err` carries the page's message; `Ok(None)` is the reply to a removal.
pub fn parse_info(raw: &str) -> Result<Option<NodeInfo>, String> {
    if raw == "null" {
        return Ok(None);
    }
    let v = json::parse(raw)?;
    if let Some(e) = v.get("error").and_then(Json::as_str) {
        return Err(e.to_string());
    }
    let s = |k: &str| v.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let n = |k: &str| v.get(k).and_then(Json::number).unwrap_or(0.0);
    let attrs = match v.get("attrs") {
        Some(Json::Obj(fields)) => fields.iter().map(|(k, val)| (k.clone(), val.as_str().unwrap_or("").to_string())).collect(),
        _ => Vec::new(),
    };
    Ok(Some(NodeInfo { tag: s("tag"), id: s("id"), class: s("class"), x: n("x"), y: n("y"), w: n("w"), h: n("h"), attrs }))
}

/// `div#id.a.b` for a node.
pub fn selector_of(i: &NodeInfo) -> String {
    let mut s = i.tag.clone();
    if !i.id.is_empty() {
        s.push('#');
        s.push_str(&i.id);
    }
    for c in i.class.split_whitespace() {
        s.push('.');
        s.push_str(c);
    }
    s
}

/// `120 x 40 at 10, 20` with whole numbers when the value is whole.
pub fn box_text(i: &NodeInfo) -> String {
    let f = |v: f64| if (v - v.round()).abs() < 0.05 { format!("{}", v.round() as i64) } else { format!("{v:.1}") };
    format!("{} x {} at {}, {}", f(i.w), f(i.h), f(i.x), f(i.y))
}

/// The tree keys (`0.1.3`) a node's ancestors have, from the root down, excluding the node itself.
pub fn ancestor_keys(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('.').collect();
    (1..parts.len()).map(|n| parts[..n].join(".")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_parse_and_match() {
        let caps = parse_caps(r#"[{"l":"warn","t":"hi","s":"https://x/a.js","n":12},{"l":"weird","t":"u","own":1}]"#);
        assert_eq!(caps.len(), 2);
        assert_eq!((caps[0].level.as_str(), caps[0].line, caps[0].own), ("warn", 12, false));
        assert_eq!((caps[1].level.as_str(), caps[1].own), ("log", true));
        assert!(parse_caps("not json").is_empty());
        assert!(same_text("a b", "a b") && same_text("a b c", "a b") && !same_text("x", "y") && !same_text("", ""));
    }

    #[test]
    fn info_and_paths() {
        let i = parse_info(r#"{"tag":"div","id":"a","class":"b c","x":10,"y":20.5,"w":120,"h":40,"attrs":{"id":"a","data-x":"1"}}"#).unwrap().unwrap();
        assert_eq!(selector_of(&i), "div#a.b.c");
        assert_eq!(box_text(&i), "120 x 40 at 10, 20.5");
        assert_eq!(parse_info("null"), Ok(None));
        assert!(parse_info(r#"{"error":"gone"}"#).is_err());
        assert_eq!(ancestor_keys("0.2.1"), vec!["0".to_string(), "0.2".to_string()]);
        assert!(ancestor_keys("0").is_empty());
    }

    #[test]
    fn scripts_are_one_line_per_statement_safe() {
        assert!(js_highlight("0.1").contains("children[+a[i]]"));
        assert!(js_inspect_point(1.0, 2.5).ends_with("(1,2.5)"));
        assert!(drain_console_js().contains("splice(0)"));
    }
}
