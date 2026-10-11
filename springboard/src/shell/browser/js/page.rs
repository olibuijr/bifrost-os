//! `JsPage`: one Boa context per tab, bound to the tab's `Dom`.

use super::net::{header_lines, NetRequest, NetResponse};
use super::timers::Timers;
use super::{host, js_string, prelude, register_all, Host};
use crate::shell::browser::dom::{Dom, NodeId};
use boa_engine::{Context, JsError, JsString, JsValue, Source};
use std::cell::RefCell;
use std::rc::Rc;

/// What the shell knows about an input event it hands to the page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventInit {
    pub bubbles: bool,
    pub cancelable: bool,
    pub client_x: f64,
    pub client_y: f64,
    /// Click count or other numeric detail; non-zero turns a plain event into a `CustomEvent`.
    pub detail: f64,
    /// Key value for keyboard events.
    pub key: String,
    pub button: i32,
}

impl EventInit {
    /// A bubbling, cancelable event, as clicks and key presses are.
    pub fn bubbling() -> EventInit {
        EventInit { bubbles: true, cancelable: true, ..EventInit::default() }
    }
}

/// A thrown value as text: `Error: message` for error objects.
fn describe(ctx: &mut Context, e: JsError) -> String {
    let opaque = e.clone().into_opaque(ctx).ok();
    match opaque.and_then(|v| v.to_string(ctx).ok()) {
        Some(s) => s.to_std_string_escaped(),
        None => e.to_string(),
    }
}

/// A JavaScript page: the context, the host state shared with its natives
/// and the DOM generation the shell last saw.
pub struct JsPage {
    ctx: Context,
    host: Rc<Host>,
    seen_gen: u64,
}

impl JsPage {
    pub fn new(dom: Rc<RefCell<Dom>>, base_url: &str) -> JsPage {
        let seen_gen = dom.borrow().generation();
        let host = Rc::new(Host {
            dom,
            base_url: base_url.to_owned(),
            log: RefCell::new(Vec::new()),
            timers: RefCell::new(Timers::default()),
            net: RefCell::new(Default::default()),
        });
        let mut ctx = Context::default();
        ctx.insert_data(host.clone());
        register_all(&mut ctx);
        if let Err(e) = ctx.eval(Source::from_bytes(&prelude())) {
            panic!("JS prelude failed: {e}");
        }
        JsPage { ctx, host, seen_gen }
    }

    /// Runs a classic script. `name` labels errors. Pending promise jobs run afterwards.
    pub fn run_script(&mut self, src: &str, name: &str) -> Result<(), String> {
        let r = self.ctx.eval(Source::from_bytes(src));
        self.drain_microtasks();
        r.map(|_| ()).map_err(|e| format!("{name}: {}", describe(&mut self.ctx, e)))
    }

    /// Dispatches a host event at `target`; false when a listener called `preventDefault`.
    pub fn dispatch_event(&mut self, target: NodeId, kind: &str, init: EventInit) -> bool {
        let args = [
            JsValue::from(f64::from(target.0)),
            js_string(kind),
            JsValue::from(init.bubbles),
            JsValue::from(init.cancelable),
            JsValue::from(init.client_x),
            JsValue::from(init.client_y),
            JsValue::from(init.detail),
            js_string(&init.key),
            JsValue::from(f64::from(init.button)),
        ];
        let result = self.call_global("__dispatch", &args);
        self.drain_microtasks();
        result.map_or(true, |v| v.to_boolean())
    }

    /// Fires every timer due at `now_ms` (each at most once per call, in due
    /// order). True when the DOM changed while running them.
    pub fn run_timers(&mut self, now_ms: f64) -> bool {
        let before = self.generation();
        let start_seq = {
            let mut t = self.host.timers.borrow_mut();
            t.now = now_ms;
            t.seq
        };
        loop {
            let next = self.host.timers.borrow_mut().pop_due(now_ms, start_seq);
            let Some(id) = next else { break };
            self.call_global("__fire", &[JsValue::from(f64::from(id)), JsValue::from(now_ms)]);
            self.drain_microtasks();
        }
        self.generation() != before
    }

    /// Runs queued promise jobs and microtasks until none are left.
    pub fn drain_microtasks(&mut self) {
        // A job that throws is already reported through its promise.
        let _ = self.ctx.run_jobs();
    }

    /// Requests scripts made since the last call, for the loader.
    pub fn take_requests(&mut self) -> Vec<NetRequest> {
        self.host.net.borrow_mut().take()
    }

    /// Settles request `id` (a fetch promise or an XHR) with the loader's answer.
    pub fn complete_request(&mut self, id: u32, resp: NetResponse) {
        let args = [
            JsValue::from(f64::from(id)),
            JsValue::from(f64::from(resp.status)),
            js_string(&resp.status_text),
            js_string(&header_lines(&resp.headers)),
            js_string(&String::from_utf8_lossy(&resp.body)),
            js_string(&resp.url),
            js_string(resp.error.as_deref().unwrap_or("")),
        ];
        self.call_global("__netDone", &args);
        self.drain_microtasks();
    }

    /// True once per DOM mutation burst since the last call.
    pub fn dirty(&mut self) -> bool {
        let g = self.generation();
        let changed = g != self.seen_gen;
        self.seen_gen = g;
        changed
    }

    /// Everything the page logged through `console`, oldest first.
    pub fn console_log(&self) -> Vec<String> {
        self.host.log.borrow().clone()
    }

    /// Like `console_log`, emptying the log.
    pub fn take_console(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.host.log.borrow_mut())
    }

    /// Evaluates an expression and returns its string form (for the shell and tests).
    pub fn eval_string(&mut self, src: &str) -> Result<String, String> {
        let v = match self.ctx.eval(Source::from_bytes(src)) {
            Ok(v) => v,
            Err(e) => return Err(describe(&mut self.ctx, e)),
        };
        self.drain_microtasks();
        v.to_string(&mut self.ctx).map(|s| s.to_std_string_escaped()).map_err(|e| e.to_string())
    }

    fn generation(&self) -> u64 {
        host(&self.ctx).dom.borrow().generation()
    }

    /// Calls a global function; errors are logged like uncaught exceptions.
    fn call_global(&mut self, name: &str, args: &[JsValue]) -> Option<JsValue> {
        let f = self.ctx.global_object().get(JsString::from(name), &mut self.ctx).ok()?;
        let f = f.as_callable()?;
        match f.call(&JsValue::undefined(), args, &mut self.ctx) {
            Ok(v) => Some(v),
            Err(e) => {
                let text = describe(&mut self.ctx, e);
                self.host.log.borrow_mut().push(format!("error: Uncaught {text}"));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::browser::dom::parse_html;

    const DOC: &str = r#"<!DOCTYPE html><html><head><title>T</title></head>
        <body><div id="app" class="box big" data-user-id="7"><p class="a">one</p><p class="b">two</p><span>s</span></div></body></html>"#;

    fn page_with(html: &str) -> (JsPage, Rc<RefCell<Dom>>) {
        let dom = Rc::new(RefCell::new(parse_html(html)));
        (JsPage::new(dom.clone(), "https://example.com/dir/page.html?q=1#frag"), dom)
    }

    fn page() -> (JsPage, Rc<RefCell<Dom>>) {
        page_with(DOC)
    }

    /// Runs `src`, expecting success, and returns the console log.
    fn run(p: &mut JsPage, src: &str) -> Vec<String> {
        p.run_script(src, "test.js").unwrap_or_else(|e| panic!("script failed: {e}"));
        p.console_log()
    }

    fn eval(p: &mut JsPage, src: &str) -> String {
        p.eval_string(src).unwrap_or_else(|e| panic!("eval failed: {e}"))
    }

    // ---- document and element lookup ----

    #[test]
    fn document_lookup_and_text() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "document.getElementById('app').className"), "box big");
        assert_eq!(eval(&mut p, "document.querySelector('p.b').textContent"), "two");
        assert_eq!(eval(&mut p, "document.querySelectorAll('p').length"), "2");
        assert_eq!(eval(&mut p, "document.getElementById('nope')"), "null");
        assert_eq!(eval(&mut p, "document.body.tagName"), "BODY");
        assert_eq!(eval(&mut p, "document.head.tagName"), "HEAD");
        assert_eq!(eval(&mut p, "document.documentElement.tagName"), "HTML");
        assert_eq!(eval(&mut p, "document.title"), "T");
    }

    #[test]
    fn same_node_yields_the_same_object() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "document.getElementById('app') === document.querySelector('div')"), "true");
        assert_eq!(eval(&mut p, "document.body.firstElementChild === document.getElementById('app')"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app').parentNode === document.body"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app').classList === document.getElementById('app').classList"), "true");
    }

    #[test]
    fn invalid_selector_throws_syntax_error() {
        let (mut p, _) = page();
        let err = p.run_script("document.querySelector('p >')", "t.js").unwrap_err();
        assert!(err.contains("selector"), "{err}");
    }

    #[test]
    fn title_setter_and_missing_title() {
        let (mut p, dom) = page();
        run(&mut p, "document.title = 'New'");
        assert_eq!(dom.borrow().text_content(dom.borrow().head().unwrap()), "New");
        let (mut p, _) = page_with("<html><head></head><body></body></html>");
        run(&mut p, "document.title = 'Made'");
        assert_eq!(eval(&mut p, "document.querySelector('head > title').textContent"), "Made");
    }

    // ---- element API ----

    #[test]
    fn attributes() {
        let (mut p, dom) = page();
        run(
            &mut p,
            "const d = document.getElementById('app');\
             d.setAttribute('Title', 'hello'); d.setAttribute('data-x', 5); d.removeAttribute('class');",
        );
        let d = dom.borrow();
        let app = d.get_element_by_id("app").unwrap();
        assert_eq!(d.attr(app, "title"), Some("hello"));
        assert_eq!(d.attr(app, "data-x"), Some("5"));
        assert_eq!(d.attr(app, "class"), None);
        drop(d);
        assert_eq!(eval(&mut p, "document.getElementById('app').hasAttribute('title')"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app').getAttribute('nope')"), "null");
        assert_eq!(eval(&mut p, "document.getElementById('app').getAttributeNames().join()"), "id,data-user-id,title,data-x");
        assert_eq!(eval(&mut p, "document.getElementById('app').toggleAttribute('hidden')"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app').hidden"), "true");
    }

    #[test]
    fn class_list() {
        let (mut p, dom) = page();
        run(
            &mut p,
            "const c = document.getElementById('app').classList;\
             c.add('x', 'y'); c.remove('box'); c.toggle('z'); c.toggle('y');",
        );
        {
            let d = dom.borrow();
            let app = d.get_element_by_id("app").unwrap();
            assert_eq!(d.attr(app, "class"), Some("big x z"));
        }
        assert_eq!(eval(&mut p, "const c2 = document.getElementById('app').classList; c2.contains('z') + ',' + c2.length"), "true,3");
        assert_eq!(eval(&mut p, "document.getElementById('app').classList.toggle('z', true)"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app').classList.replace('x', 'w'); document.getElementById('app').className"), "big w z");
        assert_eq!(eval(&mut p, "[...document.getElementById('app').classList].join('|')"), "big|w|z");
    }

    #[test]
    fn dataset() {
        let (mut p, dom) = page();
        assert_eq!(eval(&mut p, "document.getElementById('app').dataset.userId"), "7");
        run(&mut p, "const ds = document.getElementById('app').dataset; ds.fooBar = 'baz'; delete ds.userId;");
        {
            let d = dom.borrow();
            let app = d.get_element_by_id("app").unwrap();
            assert_eq!(d.attr(app, "data-foo-bar"), Some("baz"));
            assert_eq!(d.attr(app, "data-user-id"), None);
        }
        assert_eq!(eval(&mut p, "Object.keys(document.getElementById('app').dataset).join()"), "fooBar");
        assert_eq!(eval(&mut p, "'fooBar' in document.getElementById('app').dataset"), "true");
    }

    #[test]
    fn style_writes_the_attribute() {
        let (mut p, dom) = page();
        run(
            &mut p,
            "const s = document.getElementById('app').style;\
             s.color = 'red'; s.backgroundColor = 'blue'; s.setProperty('--x', '1px');",
        );
        let attr = {
            let d = dom.borrow();
            let app = d.get_element_by_id("app").unwrap();
            d.attr(app, "style").unwrap().to_owned()
        };
        assert_eq!(attr, "color: red; background-color: blue; --x: 1px;");
        assert_eq!(eval(&mut p, "document.getElementById('app').style.backgroundColor"), "blue");
        assert_eq!(eval(&mut p, "document.getElementById('app').style.removeProperty('color')"), "red");
        assert_eq!(eval(&mut p, "document.getElementById('app').style.cssText"), "background-color: blue; --x: 1px;");
        run(&mut p, "document.getElementById('app').style.cssText = 'width: 10px'");
        assert_eq!(eval(&mut p, "document.getElementById('app').style.width"), "10px");
        assert_eq!(eval(&mut p, "getComputedStyle(document.getElementById('app')).width"), "10px");
    }

    #[test]
    fn style_parsing_keeps_urls_with_semicolons() {
        let (mut p, _) = page();
        run(&mut p, "document.body.setAttribute('style', \"background: url('data:image/png;base64,AAA'); color: red\")");
        assert_eq!(eval(&mut p, "document.body.style.color"), "red");
        assert_eq!(eval(&mut p, "document.body.style.background"), "url('data:image/png;base64,AAA')");
    }

    #[test]
    fn inner_html_text_content_outer_html() {
        let (mut p, dom) = page();
        run(&mut p, "document.getElementById('app').innerHTML = '<b id=\"n\">bold</b> tail'");
        assert_eq!(eval(&mut p, "document.getElementById('n').textContent"), "bold");
        assert_eq!(eval(&mut p, "document.getElementById('app').innerHTML"), "<b id=\"n\">bold</b> tail");
        assert_eq!(eval(&mut p, "document.getElementById('n').outerHTML"), "<b id=\"n\">bold</b>");
        run(&mut p, "document.getElementById('app').textContent = 'plain <x>'");
        assert_eq!(dom.borrow().inner_html(dom.borrow().get_element_by_id("app").unwrap()), "plain &lt;x&gt;");
        assert_eq!(eval(&mut p, "document.getElementById('app').childNodes.length"), "1");
        run(&mut p, "document.getElementById('app').outerHTML = '<section id=\"s\">x</section>'");
        assert_eq!(eval(&mut p, "document.getElementById('s').parentNode === document.body"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app')"), "null");
    }

    #[test]
    fn tree_mutation() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const app = document.getElementById('app');\
             const a = document.createElement('i'); a.id = 'a';\
             const b = document.createElement('u'); b.id = 'b';\
             app.appendChild(a); app.insertBefore(b, app.firstChild);\
             app.prepend('start', document.createElement('hr'));\
             app.append('end');\
             console.log(app.childNodes.length, app.firstChild.nodeValue, app.lastChild.nodeValue);\
             console.log(app.children.map(c => c.tagName).join());\
             app.removeChild(a); b.remove();\
             console.log(app.children.map(c => c.tagName).join());",
        );
        let log = p.console_log();
        assert_eq!(log[0], "8 start end");
        assert_eq!(log[1], "HR,U,P,P,SPAN,I");
        assert_eq!(log[2], "HR,P,P,SPAN");
    }

    #[test]
    fn append_moves_nodes_and_rejects_cycles() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const p1 = document.querySelector('p.a'); document.body.appendChild(p1);\
             console.log(p1.parentNode === document.body, document.getElementById('app').children.length);",
        );
        assert_eq!(p.console_log()[0], "true 2");
        let err = p.run_script("const app = document.getElementById('app'); app.firstElementChild.appendChild(app)", "t.js");
        assert!(err.is_err());
        let err = p.run_script("document.body.removeChild(document.querySelector('span'))", "t.js").unwrap_err();
        assert!(err.contains("NotFoundError"), "{err}");
    }

    #[test]
    fn sibling_and_parent_navigation() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "document.querySelector('p.a').nextElementSibling.className"), "b");
        assert_eq!(eval(&mut p, "document.querySelector('p.b').previousSibling.className"), "a");
        assert_eq!(eval(&mut p, "document.querySelector('span').nextSibling"), "null");
        assert_eq!(eval(&mut p, "document.getElementById('app').lastElementChild.tagName"), "SPAN");
        assert_eq!(eval(&mut p, "document.querySelector('p').parentElement.id"), "app");
        assert_eq!(eval(&mut p, "document.documentElement.parentNode === document"), "true");
        assert_eq!(eval(&mut p, "document.body.contains(document.querySelector('p'))"), "true");
        assert_eq!(eval(&mut p, "document.createElement('x').isConnected"), "false");
        assert_eq!(eval(&mut p, "document.querySelector('p').isConnected"), "true");
        assert_eq!(eval(&mut p, "document.getElementById('app').childElementCount"), "3");
    }

    #[test]
    fn matches_closest_and_scoped_queries() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "document.querySelector('p').matches('#app > p.a')"), "true");
        assert_eq!(eval(&mut p, "document.querySelector('p').matches('span')"), "false");
        assert_eq!(eval(&mut p, "document.querySelector('p').closest('.box').id"), "app");
        assert_eq!(eval(&mut p, "document.querySelector('p').closest('table')"), "null");
        assert_eq!(eval(&mut p, "document.getElementById('app').querySelectorAll('p:not(.a)').length"), "1");
        assert_eq!(eval(&mut p, "document.getElementById('app').querySelector('div')"), "null");
        assert_eq!(eval(&mut p, "document.getElementsByTagName('p').length"), "2");
        assert_eq!(eval(&mut p, "document.getElementsByClassName('box big').length"), "1");
    }

    #[test]
    fn create_clone_and_fragments() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const app = document.getElementById('app');\
             const copy = app.cloneNode(true); copy.id = 'copy'; document.body.appendChild(copy);\
             const frag = document.createDocumentFragment();\
             frag.appendChild(document.createElement('em')); frag.appendChild(document.createTextNode('t'));\
             console.log(frag.nodeType, frag.childNodes.length);\
             document.body.appendChild(frag);\
             console.log(frag.childNodes.length, document.body.lastChild.nodeValue);\
             console.log(copy.children.length, app.cloneNode(false).children.length);\
             const c = document.createComment('hi'); console.log(c.nodeType, c.data);",
        );
        let log = p.console_log();
        assert_eq!(log[0], "11 2");
        assert_eq!(log[1], "0 t");
        assert_eq!(log[2], "3 0");
        assert_eq!(log[3], "8 hi");
    }

    #[test]
    fn insert_adjacent_and_replace_with() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const a = document.querySelector('p.a');\
             a.insertAdjacentHTML('beforebegin', '<h1>x</h1>');\
             a.insertAdjacentHTML('afterend', '<h2>y</h2>');\
             a.replaceWith(document.createElement('q'));\
             console.log(document.getElementById('app').children.map(c => c.tagName).join());",
        );
        assert_eq!(p.console_log()[0], "H1,Q,H2,P,SPAN");
    }

    #[test]
    fn layout_stubs_are_zero() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "JSON.stringify(document.body.getBoundingClientRect())"), r#"{"x":0,"y":0,"top":0,"left":0,"right":0,"bottom":0,"width":0,"height":0}"#);
        assert_eq!(eval(&mut p, "document.body.offsetWidth + document.body.clientHeight + document.body.scrollTop"), "0");
    }

    #[test]
    fn form_properties_reflect_attributes() {
        let (mut p, _) = page_with("<html><body><input id=i type=checkbox value=v disabled><a id=l href='/x?y=1'></a></body></html>");
        assert_eq!(eval(&mut p, "const i = document.getElementById('i'); i.value + i.type + i.disabled + i.checked"), "vcheckboxtruefalse");
        run(&mut p, "const i2 = document.getElementById('i'); i2.checked = true; i2.disabled = false; i2.value = 'w'");
        assert_eq!(eval(&mut p, "document.getElementById('i').outerHTML"), "<input id=\"i\" type=\"checkbox\" value=\"w\" checked=\"\">");
        assert_eq!(eval(&mut p, "document.getElementById('l').href"), "https://example.com/x?y=1");
        assert_eq!(eval(&mut p, "document.getElementById('l').getAttribute('href')"), "/x?y=1");
        assert_eq!(eval(&mut p, "document.querySelectorAll('input:checked').length"), "1");
    }

    // ---- events ----

    const NESTED: &str = "<html><body><div id=outer><div id=inner><button id=btn>b</button></div></div></body></html>";

    #[test]
    fn capture_target_bubble_order() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "const log = [];\
             for (const id of ['outer', 'inner', 'btn']) {\
               const e = document.getElementById(id);\
               e.addEventListener('click', () => log.push(id + ':cap'), true);\
               e.addEventListener('click', () => log.push(id + ':bub'));\
             }\
             document.addEventListener('click', () => log.push('doc'));\
             window.addEventListener('click', () => log.push('win'));\
             window.addEventListener('click', () => log.push('win:cap'), true);\
             globalThis.log = log;",
        );
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        assert!(p.dispatch_event(btn, "click", EventInit::bubbling()));
        assert_eq!(
            eval(&mut p, "log.join()"),
            "win:cap,outer:cap,inner:cap,btn:cap,btn:bub,inner:bub,outer:bub,doc,win"
        );
    }

    #[test]
    fn non_bubbling_events_stay_on_the_path_down() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "const log = []; globalThis.log = log;\
             const o = document.getElementById('outer');\
             o.addEventListener('focus', () => log.push('o-bub'));\
             o.addEventListener('focus', () => log.push('o-cap'), true);\
             document.getElementById('btn').addEventListener('focus', () => log.push('btn'));",
        );
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        p.dispatch_event(btn, "focus", EventInit::default());
        assert_eq!(eval(&mut p, "log.join()"), "o-cap,btn");
    }

    #[test]
    fn stop_propagation_and_immediate() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "const log = []; globalThis.log = log;\
             const inner = document.getElementById('inner');\
             inner.addEventListener('click', (e) => { log.push('i1'); e.stopPropagation(); });\
             inner.addEventListener('click', () => log.push('i2'));\
             document.getElementById('outer').addEventListener('click', () => log.push('outer'));\
             const btn = document.getElementById('btn');\
             btn.addEventListener('click', (e) => { log.push('b1'); e.stopImmediatePropagation(); });\
             btn.addEventListener('click', () => log.push('b2'));",
        );
        let (btn, inner) = {
            let d = dom.borrow();
            (d.get_element_by_id("btn").unwrap(), d.get_element_by_id("inner").unwrap())
        };
        p.dispatch_event(btn, "click", EventInit::bubbling());
        assert_eq!(eval(&mut p, "log.join()"), "b1");
        run(&mut p, "log.length = 0");
        p.dispatch_event(inner, "click", EventInit::bubbling());
        assert_eq!(eval(&mut p, "log.join()"), "i1,i2");
    }

    #[test]
    fn prevent_default_is_reported() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "document.getElementById('btn').addEventListener('click', (e) => e.preventDefault());\
             document.getElementById('inner').addEventListener('submit', (e) => e.preventDefault());",
        );
        let (btn, inner) = {
            let d = dom.borrow();
            (d.get_element_by_id("btn").unwrap(), d.get_element_by_id("inner").unwrap())
        };
        assert!(!p.dispatch_event(btn, "click", EventInit::bubbling()));
        // Not cancelable: preventDefault is ignored, so the result is true.
        assert!(p.dispatch_event(btn, "click", EventInit { bubbles: true, cancelable: false, ..EventInit::default() }));
        assert!(p.dispatch_event(inner, "submit", EventInit { bubbles: true, cancelable: false, ..EventInit::default() }));
        assert!(!p.dispatch_event(inner, "submit", EventInit::bubbling()));
    }

    #[test]
    fn once_and_remove_listener() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "let n = 0, m = 0; globalThis.counts = () => n + ',' + m;\
             const b = document.getElementById('btn');\
             b.addEventListener('click', () => n++, { once: true });\
             const h = () => m++;\
             b.addEventListener('click', h);\
             b.addEventListener('click', h);\
             globalThis.remove = () => b.removeEventListener('click', h);",
        );
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        p.dispatch_event(btn, "click", EventInit::bubbling());
        p.dispatch_event(btn, "click", EventInit::bubbling());
        assert_eq!(eval(&mut p, "counts()"), "1,2");
        run(&mut p, "remove()");
        p.dispatch_event(btn, "click", EventInit::bubbling());
        assert_eq!(eval(&mut p, "counts()"), "1,2");
    }

    #[test]
    fn listener_removed_during_dispatch_does_not_run() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "const log = []; globalThis.log = log;\
             const b = document.getElementById('btn');\
             const second = () => log.push('second');\
             b.addEventListener('click', () => { log.push('first'); b.removeEventListener('click', second); });\
             b.addEventListener('click', second);",
        );
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        p.dispatch_event(btn, "click", EventInit::bubbling());
        assert_eq!(eval(&mut p, "log.join()"), "first");
    }

    #[test]
    fn event_object_fields_and_handler_properties() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "const b = document.getElementById('btn');\
             b.onclick = (e) => console.log(e.type, e.target.id, e.currentTarget.id, e.eventPhase, e.clientX, e.clientY, e.button, e.isTrusted, e instanceof MouseEvent);\
             document.getElementById('outer').addEventListener('click', (e) => console.log(e.target.id, e.currentTarget.id, e.eventPhase));\
             b.addEventListener('keydown', (e) => console.log(e.key, e instanceof KeyboardEvent));\
             b.addEventListener('custom', (e) => console.log(e.detail, e instanceof CustomEvent));\
             b.addEventListener('plain', (e) => console.log(e instanceof CustomEvent, e.constructor === Event));",
        );
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        p.dispatch_event(btn, "click", EventInit { bubbles: true, cancelable: true, client_x: 10.0, client_y: 20.0, ..EventInit::default() });
        p.dispatch_event(btn, "keydown", EventInit { key: "Enter".into(), ..EventInit::default() });
        p.dispatch_event(btn, "custom", EventInit { detail: 3.0, ..EventInit::default() });
        p.dispatch_event(btn, "plain", EventInit::default());
        let log = p.console_log();
        assert_eq!(log[0], "click btn btn 2 10 20 0 true true");
        assert_eq!(log[1], "btn outer 3");
        assert_eq!(log[2], "Enter true");
        assert_eq!(log[3], "3 true");
        assert_eq!(log[4], "false true");
    }

    #[test]
    fn script_dispatch_event_and_click() {
        let (mut p, _) = page_with(NESTED);
        run(
            &mut p,
            "const b = document.getElementById('btn');\
             document.body.addEventListener('ping', (e) => console.log('ping', e.detail.n, e.target.id));\
             b.addEventListener('ping', (e) => e.preventDefault());\
             const ev = new CustomEvent('ping', { bubbles: true, cancelable: true, detail: { n: 4 } });\
             console.log(b.dispatchEvent(ev), ev.defaultPrevented);\
             b.addEventListener('click', () => console.log('clicked'));\
             b.click();\
             const plain = new Event('x'); console.log(b.dispatchEvent(plain), plain.bubbles);",
        );
        let log = p.console_log();
        assert_eq!(log[0], "ping 4 btn");
        assert_eq!(log[1], "false true");
        assert_eq!(log[2], "clicked");
        assert_eq!(log[3], "true false");
    }

    #[test]
    fn handleevent_objects_and_listener_errors() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "const b = document.getElementById('btn');\
             b.addEventListener('click', { handleEvent(e) { console.log('obj', e.type); } });\
             b.addEventListener('click', () => { throw new Error('bad'); });\
             b.addEventListener('click', () => console.log('after'));",
        );
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        p.dispatch_event(btn, "click", EventInit::bubbling());
        let log = p.console_log();
        assert_eq!(log[0], "obj click");
        assert!(log[1].starts_with("error: Uncaught Error: bad"), "{}", log[1]);
        assert_eq!(log[2], "after");
    }

    #[test]
    fn window_load_style_listeners() {
        let (mut p, _) = page_with(NESTED);
        run(
            &mut p,
            "addEventListener('load', () => console.log('load'));\
             window.onresize = () => console.log('resize');\
             dispatchEvent(new Event('load')); window.dispatchEvent(new Event('resize'));",
        );
        assert_eq!(p.console_log(), vec!["load", "resize"]);
    }

    // ---- window ----

    #[test]
    fn console_levels_are_logged() {
        let (mut p, _) = page();
        let log = run(
            &mut p,
            "console.log('a', 1, {x: 2}, [1, 2], null, undefined); console.warn('w'); console.error('e', new Error('m'));\
             console.info('i'); console.assert(false, 'nope'); console.assert(true, 'fine');",
        );
        assert_eq!(
            log,
            vec!["a 1 {\"x\":2} [1,2] null undefined", "warn: w", "error: e Error: m", "i", "error: Assertion failed: nope"]
        );
        assert_eq!(p.take_console().len(), 5);
        assert!(p.console_log().is_empty());
    }

    #[test]
    fn script_errors_are_returned() {
        let (mut p, _) = page();
        let e = p.run_script("throw new Error('boom')", "inline").unwrap_err();
        assert!(e.starts_with("inline: "), "{e}");
        assert!(e.contains("boom"), "{e}");
        let e = p.run_script("var = ;", "syntax").unwrap_err();
        assert!(e.starts_with("syntax: "), "{e}");
        run(&mut p, "console.log('still alive')");
    }

    #[test]
    fn location_comes_from_the_base_url() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "location.href"), "https://example.com/dir/page.html?q=1#frag");
        assert_eq!(eval(&mut p, "[location.protocol, location.host, location.hostname, location.port, location.pathname, location.search, location.hash, location.origin].join(' ')"),
            "https: example.com example.com  /dir/page.html ?q=1 #frag https://example.com");
        assert_eq!(eval(&mut p, "window.location === location && document.location === location && String(location)"), "https://example.com/dir/page.html?q=1#frag");
        assert_eq!(eval(&mut p, "window === self && self === globalThis && window.window === window"), "true");
    }

    #[test]
    fn navigator_and_misc_globals() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "navigator.userAgent.includes('Mozilla')"), "true");
        assert_eq!(eval(&mut p, "typeof matchMedia('(min-width: 1px)').matches"), "boolean");
        assert_eq!(eval(&mut p, "typeof performance.now()"), "number");
        assert_eq!(eval(&mut p, "typeof MutationObserver + typeof ResizeObserver + typeof IntersectionObserver"), "functionfunctionfunction");
        assert_eq!(eval(&mut p, "typeof innerWidth + typeof devicePixelRatio + typeof history.pushState"), "numbernumberfunction");
        assert_eq!(eval(&mut p, "structuredClone({a: [1, 2]}).a.length"), "2");
    }

    #[test]
    fn base64_and_text_coding() {
        let (mut p, _) = page();
        assert_eq!(eval(&mut p, "btoa('hello')"), "aGVsbG8=");
        assert_eq!(eval(&mut p, "btoa('hi')"), "aGk=");
        assert_eq!(eval(&mut p, "btoa('')"), "");
        assert_eq!(eval(&mut p, "atob('aGVsbG8=')"), "hello");
        assert_eq!(eval(&mut p, "atob('aGk')"), "hi");
        assert_eq!(eval(&mut p, "atob(btoa('\\u00e9\\u00ff'))"), "\u{e9}\u{ff}");
        assert!(p.run_script("btoa('\\u20ac')", "t").is_err());
        assert!(p.run_script("atob('***')", "t").is_err());
        assert_eq!(eval(&mut p, "Array.from(new TextEncoder().encode('a\\u00e9\\u20ac\\ud83d\\ude00')).join()"), "97,195,169,226,130,172,240,159,152,128");
        assert_eq!(eval(&mut p, "new TextDecoder().decode(new TextEncoder().encode('a\\u00e9\\u20ac\\ud83d\\ude00'))"), "a\u{e9}\u{20ac}\u{1f600}");
    }

    #[test]
    fn local_and_session_storage() {
        let (mut p, _) = page();
        run(
            &mut p,
            "localStorage.setItem('a', 1); localStorage.b = 'two'; sessionStorage.setItem('a', 's');\
             console.log(localStorage.getItem('a'), localStorage.b, localStorage.length, localStorage.key(1), sessionStorage.getItem('a'));\
             console.log(localStorage.getItem('zzz'), localStorage.zzz, 'b' in localStorage);\
             localStorage.removeItem('a'); delete localStorage.b;\
             console.log(localStorage.length); localStorage.setItem('k', 'v'); localStorage.clear(); console.log(localStorage.length, sessionStorage.length);",
        );
        let log = p.console_log();
        assert_eq!(log[0], "1 two 2 b s");
        assert_eq!(log[1], "null undefined true");
        assert_eq!(log[2], "0");
        assert_eq!(log[3], "0 1");
    }

    #[test]
    fn cookie_stub() {
        let (mut p, _) = page();
        run(&mut p, "document.cookie = 'a=1; path=/'; document.cookie = 'b=2'; console.log(document.cookie);\
                     document.cookie = 'a=; max-age=0'; console.log(document.cookie);");
        assert_eq!(p.console_log(), vec!["a=1; b=2", "b=2"]);
    }

    #[test]
    fn url_and_search_params() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const u = new URL('../x/y.html?a=1&b=two%20words#h', 'https://h.test/p/q/r');\
             console.log(u.href, u.pathname, u.search, u.hash, u.host, u.origin);\
             console.log(u.searchParams.get('b'), u.searchParams.has('a'), u.searchParams.get('nope'));\
             u.searchParams.set('a', '9'); u.searchParams.append('c', 'x y');\
             console.log(u.search, u.href);\
             u.pathname = 'z'; u.hash = ''; console.log(u.href);\
             const s = new URLSearchParams({k: 'v', 'a b': 'c&d'}); s.append('k', '2');\
             console.log(s.toString(), s.getAll('k').join(), [...s].length);\
             console.log(new URLSearchParams('?x=1&x=2&y').getAll('x').join(), new URLSearchParams('y=%41+b').get('y'));\
             let bad = 'ok'; try { new URL('nope'); } catch (e) { bad = e instanceof TypeError; }\
             console.log(bad, URL.canParse('http://a/'), URL.canParse('a'));",
        );
        let log = p.console_log();
        assert_eq!(log[0], "https://h.test/p/x/y.html?a=1&b=two%20words#h /p/x/y.html ?a=1&b=two%20words #h h.test https://h.test");
        assert_eq!(log[1], "two words true null");
        assert_eq!(log[2], "?a=9&b=two+words&c=x+y https://h.test/p/x/y.html?a=9&b=two+words&c=x+y#h");
        assert_eq!(log[3], "https://h.test/z?a=9&b=two+words&c=x+y");
        assert_eq!(log[4], "k=v&a+b=c%26d&k=2 v,2 3");
        assert_eq!(log[5], "1,2 A b");
        assert_eq!(log[6], "true true false");
    }

    // ---- timers and microtasks ----

    #[test]
    fn timeouts_fire_in_order_when_due() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const log = []; globalThis.log = log;\
             setTimeout(() => log.push('b'), 20);\
             setTimeout((x, y) => log.push('a' + x + y), 10, 1, 2);\
             setTimeout(() => log.push('c'), 20);\
             const gone = setTimeout(() => log.push('never'), 5); clearTimeout(gone);",
        );
        p.run_timers(9.0);
        assert_eq!(eval(&mut p, "log.join()"), "");
        p.run_timers(10.0);
        assert_eq!(eval(&mut p, "log.join()"), "a12");
        p.run_timers(500.0);
        assert_eq!(eval(&mut p, "log.join()"), "a12,b,c");
        p.run_timers(1000.0);
        assert_eq!(eval(&mut p, "log.length"), "3");
    }

    #[test]
    fn intervals_repeat_and_can_be_cleared() {
        let (mut p, _) = page();
        run(
            &mut p,
            "let n = 0; globalThis.count = () => n;\
             globalThis.iv = setInterval(() => { n++; if (n === 3) clearInterval(iv); }, 10);",
        );
        p.run_timers(10.0);
        p.run_timers(20.0);
        assert_eq!(eval(&mut p, "count()"), "2");
        p.run_timers(30.0);
        p.run_timers(40.0);
        p.run_timers(50.0);
        assert_eq!(eval(&mut p, "count()"), "3");
    }

    #[test]
    fn timer_scheduled_in_a_timer_waits_for_its_delay() {
        let (mut p, _) = page();
        run(&mut p, "const log = []; globalThis.log = log; setTimeout(() => { log.push('outer'); setTimeout(() => log.push('inner'), 50); }, 10);");
        p.run_timers(10.0);
        assert_eq!(eval(&mut p, "log.join()"), "outer");
        p.run_timers(59.0);
        assert_eq!(eval(&mut p, "log.join()"), "outer");
        p.run_timers(60.0);
        assert_eq!(eval(&mut p, "log.join()"), "outer,inner");
    }

    #[test]
    fn run_timers_reports_dom_changes() {
        let (mut p, _) = page();
        run(
            &mut p,
            "setTimeout(() => {}, 10); setTimeout(() => { document.body.setAttribute('data-t', '1'); }, 20);",
        );
        assert!(!p.run_timers(10.0));
        assert!(p.run_timers(20.0));
        assert!(!p.run_timers(30.0));
    }

    #[test]
    fn request_animation_frame_gets_the_timestamp() {
        let (mut p, _) = page();
        run(
            &mut p,
            "requestAnimationFrame((t) => console.log('raf', t));\
             const id = requestAnimationFrame(() => console.log('cancelled')); cancelAnimationFrame(id);",
        );
        p.run_timers(8.0);
        assert!(p.console_log().is_empty());
        p.run_timers(16.0);
        assert_eq!(p.console_log(), vec!["raf 16"]);
    }

    #[test]
    fn timer_errors_are_logged_and_do_not_stop_others() {
        let (mut p, _) = page();
        run(&mut p, "setTimeout(() => { throw new Error('oops'); }, 1); setTimeout(() => console.log('next'), 2);");
        p.run_timers(5.0);
        let log = p.console_log();
        assert!(log[0].starts_with("error: Uncaught Error: oops"), "{}", log[0]);
        assert_eq!(log[1], "next");
    }

    #[test]
    fn microtasks_and_promises_drain_after_scripts() {
        let (mut p, _) = page();
        let log = run(
            &mut p,
            "console.log('sync');\
             Promise.resolve().then(() => console.log('then'));\
             queueMicrotask(() => console.log('micro'));\
             (async () => { await null; console.log('async'); })();\
             console.log('end');",
        );
        assert_eq!(log, vec!["sync", "end", "then", "micro", "async"]);
    }

    #[test]
    fn microtasks_drain_after_timers_and_events() {
        let (mut p, dom) = page_with(NESTED);
        run(
            &mut p,
            "setTimeout(() => Promise.resolve().then(() => console.log('t-then')), 1);\
             document.getElementById('btn').addEventListener('click', () => Promise.resolve().then(() => console.log('e-then')));",
        );
        p.run_timers(1.0);
        assert_eq!(p.console_log(), vec!["t-then"]);
        let btn = dom.borrow().get_element_by_id("btn").unwrap();
        p.dispatch_event(btn, "click", EventInit::bubbling());
        assert_eq!(p.console_log(), vec!["t-then", "e-then"]);
    }

    // ---- dirty ----

    #[test]
    fn dirty_tracks_dom_mutation() {
        let (mut p, dom) = page();
        assert!(!p.dirty());
        run(&mut p, "document.getElementById('app').setAttribute('a', 'b')");
        assert!(p.dirty());
        assert!(!p.dirty());
        run(&mut p, "document.getElementById('app').getAttribute('a')");
        assert!(!p.dirty());
        let app = dom.borrow().get_element_by_id("app").unwrap();
        dom.borrow_mut().set_attr(app, "x", "y");
        assert!(p.dirty());
    }

    // ---- fetch and XHR ----

    #[test]
    fn fetch_queues_a_request_and_resolves_with_the_response() {
        let (mut p, _) = page();
        run(
            &mut p,
            "fetch('/api/data?x=1', { method: 'post', headers: { 'X-Test': 'yes' }, body: 'payload' })\
               .then((r) => r.json().then((j) => console.log(r.status, r.ok, r.statusText, r.headers.get('Content-Type'), r.url, j.v)));",
        );
        let reqs = p.take_requests();
        assert_eq!(reqs.len(), 1);
        let r = &reqs[0];
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://example.com/api/data?x=1");
        assert_eq!(r.body.as_deref(), Some(&b"payload"[..]));
        assert!(r.headers.iter().any(|(k, v)| k == "x-test" && v == "yes"));
        assert!(p.take_requests().is_empty());
        assert!(p.console_log().is_empty());
        p.complete_request(
            r.id,
            NetResponse::ok(200, "{\"v\": 42}").header("Content-Type", "application/json"),
        );
        assert_eq!(p.console_log(), vec!["200 true OK application/json https://example.com/api/data?x=1 42"]);
    }

    #[test]
    fn fetch_text_error_status_and_network_failure() {
        let (mut p, _) = page();
        run(
            &mut p,
            "fetch('a.txt').then((r) => r.text()).then((t) => console.log('text', t));\
             fetch('https://other.test/missing').then((r) => console.log('status', r.status, r.ok));\
             fetch('down').catch((e) => console.log('failed', e instanceof TypeError, e.message));",
        );
        let reqs = p.take_requests();
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[0].url, "https://example.com/dir/a.txt");
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(reqs[0].body, None);
        // Complete out of order.
        p.complete_request(reqs[2].id, NetResponse::failed("connection refused"));
        p.complete_request(reqs[1].id, NetResponse { status: 404, ..NetResponse::default() });
        p.complete_request(reqs[0].id, NetResponse::ok(200, "hello"));
        assert_eq!(
            p.console_log(),
            vec!["failed true Failed to fetch: connection refused", "status 404 false", "text hello"]
        );
    }

    #[test]
    fn fetch_abort_rejects_and_ignores_the_late_response() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const ac = new AbortController();\
             fetch('slow', { signal: ac.signal }).then(() => console.log('resolved'), (e) => console.log('rejected', e.name));\
             ac.abort();",
        );
        let reqs = p.take_requests();
        p.complete_request(reqs[0].id, NetResponse::ok(200, "late"));
        assert_eq!(p.console_log(), vec!["rejected AbortError"]);
    }

    #[test]
    fn fetch_response_helpers() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const r = new Response('{\"a\":1}', { status: 201, headers: { 'X-A': 'b' } });\
             console.log(r.status, r.ok, r.headers.get('x-a'), r.bodyUsed);\
             r.clone().json().then((j) => { console.log('json', j.a); return r.text(); })\
               .then((t) => { console.log('text', t, r.bodyUsed); return r.text(); })\
               .catch((e) => console.log('twice', e instanceof TypeError));\
             const h = new Headers([['A', '1']]); h.append('a', '2'); console.log(h.get('a'), h.has('b'));",
        );
        assert_eq!(p.console_log(), vec!["201 true b false", "1, 2 false", "json 1", "text {\"a\":1} true", "twice true"]);
    }

    #[test]
    fn xhr_lifecycle_and_events() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const x = new XMLHttpRequest();\
             x.onreadystatechange = () => console.log('rs', x.readyState);\
             x.onload = () => console.log('load', x.status, x.responseText, x.getResponseHeader('X-Y'), x.responseURL);\
             x.addEventListener('loadend', () => console.log('loadend'));\
             x.open('POST', '/submit'); x.setRequestHeader('Content-Type', 'text/plain'); x.send('body!');",
        );
        let reqs = p.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].url, "https://example.com/submit");
        assert_eq!(reqs[0].body.as_deref(), Some(&b"body!"[..]));
        assert!(reqs[0].headers.iter().any(|(k, v)| k == "content-type" && v == "text/plain"));
        assert_eq!(p.console_log(), vec!["rs 1"]);
        p.complete_request(reqs[0].id, NetResponse::ok(200, "done").header("X-Y", "z"));
        assert_eq!(
            p.console_log(),
            vec!["rs 1", "rs 2", "rs 3", "rs 4", "load 200 done z https://example.com/submit", "loadend"]
        );
    }

    #[test]
    fn xhr_error_json_and_abort() {
        let (mut p, _) = page();
        run(
            &mut p,
            "const bad = new XMLHttpRequest(); bad.onerror = () => console.log('error', bad.status);\
             bad.open('GET', 'http://down.test/'); bad.send();\
             const js = new XMLHttpRequest(); js.responseType = 'json'; js.onload = () => console.log('json', js.response.k);\
             js.open('GET', '/j'); js.send();\
             const ab = new XMLHttpRequest(); ab.onabort = () => console.log('abort'); ab.onload = () => console.log('load?');\
             ab.open('GET', '/a'); ab.send(); ab.abort();",
        );
        let reqs = p.take_requests();
        assert_eq!(reqs.len(), 3);
        p.complete_request(reqs[0].id, NetResponse::failed("dns"));
        p.complete_request(reqs[1].id, NetResponse::ok(200, "{\"k\": \"v\"}"));
        p.complete_request(reqs[2].id, NetResponse::ok(200, "late"));
        assert_eq!(p.console_log(), vec!["abort", "error 0", "json v"]);
    }

    #[test]
    fn completing_an_unknown_request_is_harmless() {
        let (mut p, _) = page();
        p.complete_request(99, NetResponse::ok(200, "x"));
        assert!(p.console_log().is_empty());
        assert!(p.take_requests().is_empty());
    }
}
