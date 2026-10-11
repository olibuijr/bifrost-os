//! The `document` object: lookup natives (id, selectors) over `Dom` and the
//! JavaScript `Document` class with element factories, `title` and a cookie
//! jar stub.

use super::element::with_dom;
use super::selector;
use super::{arg_bool, arg_node, arg_str, js_string, node_list, node_value, register as register_table, NativeFn};
use boa_engine::{Context, JsNativeError, JsResult, JsValue};

fn parse(sel: &str) -> JsResult<selector::SelectorList> {
    selector::parse(sel).map_err(|e| {
        JsNativeError::syntax()
            .with_message(format!("'{sel}' is not a valid selector: {e}"))
            .into()
    })
}

fn get_by_id(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let id = arg_str(a, 0, c)?;
    Ok(node_value(with_dom(c, |d| d.get_element_by_id(&id))))
}

fn doc_element(_: &JsValue, _: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    Ok(node_value(with_dom(c, |d| d.document_element())))
}

fn head(_: &JsValue, _: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    Ok(node_value(with_dom(c, |d| d.head())))
}

fn body(_: &JsValue, _: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    Ok(node_value(with_dom(c, |d| d.body())))
}

/// `query(root, selector, firstOnly)`: matching descendants as a node list.
fn query(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let root = arg_node(a, 0, c)?;
    let sel = arg_str(a, 1, c)?;
    let first = arg_bool(a, 2);
    let list = parse(&sel)?;
    let ids = with_dom(c, |d| selector::query_all(d, root, &list, first));
    Ok(node_list(ids.into_iter()))
}

fn matches(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let sel = arg_str(a, 1, c)?;
    let list = parse(&sel)?;
    Ok(JsValue::from(with_dom(c, |d| d.tag(n).is_some() && list.matches(d, n))))
}

fn closest(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let sel = arg_str(a, 1, c)?;
    let list = parse(&sel)?;
    Ok(node_value(with_dom(c, |d| selector::closest(d, n, &list))))
}

/// The base URL the page was created with.
fn base_url(_: &JsValue, _: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    Ok(js_string(&super::host(c).base_url))
}

const TABLE: &[(&str, NativeFn)] = &[
    ("get_by_id", get_by_id),
    ("doc_element", doc_element),
    ("head", head),
    ("body", body),
    ("query", query),
    ("matches", matches),
    ("closest", closest),
    ("base_url", base_url),
];

pub(crate) fn register(ctx: &mut Context) {
    register_table(ctx, TABLE);
}

pub(crate) const JS: &str = r####"
const cookieJar = new Map();
class Document extends Node {
  get documentElement() { return wrap(H.doc_element()); }
  get head() { return wrap(H.head()); }
  get body() { return wrap(H.body()); }
  get defaultView() { return globalThis; }
  get location() { return globalThis.location; }
  get URL() { return globalThis.location.href; }
  get documentURI() { return globalThis.location.href; }
  get domain() { return globalThis.location.hostname; }
  get referrer() { return ''; }
  get readyState() { return 'complete'; }
  get compatMode() { return 'CSS1Compat'; }
  get characterSet() { return 'UTF-8'; }
  get contentType() { return 'text/html'; }
  get hidden() { return false; }
  get visibilityState() { return 'visible'; }
  get activeElement() { return this._active || this.body; }
  get scrollingElement() { return this.documentElement; }
  get fonts() { return { ready: Promise.resolve(), check() { return true; }, load() { return Promise.resolve([]); } }; }
  get styleSheets() { return []; }
  get forms() { return query(this, 'form', false); }
  get images() { return query(this, 'img', false); }
  get links() { return query(this, 'a[href], area[href]', false); }
  get scripts() { return query(this, 'script', false); }
  hasFocus() { return true; }
  getElementById(id) { const n = wrap(H.get_by_id(String(id))); return n && n.isConnected ? n : null; }
  getElementsByName(n) { return query(this, '[name="' + String(n).replace(/["\\]/g, '\\$&') + '"]', false); }
  createElement(tag) {
    tag = String(tag).toLowerCase();
    if (!/^[a-z][^\s\/>]*$/.test(tag)) throw new Error('InvalidCharacterError: invalid element name');
    return wrap(H.create_element(tag));
  }
  createElementNS(ns, tag) { return this.createElement(tag); }
  createTextNode(t) { return wrap(H.create_text(String(t))); }
  createComment(t) { return wrap(H.create_comment(String(t))); }
  createDocumentFragment() { return wrap(H.create_element('#fragment')); }
  createEvent(type) { return new Event('', {}); }
  importNode(n, deep) { return n.cloneNode(deep); }
  adoptNode(n) { return n; }
  elementFromPoint() { return null; }
  getSelection() { return null; }
  execCommand() { return false; }
  open() {}
  close() {}
  write() {}
  get title() {
    const t = this.querySelector('title');
    return t ? t.textContent.replace(/\s+/g, ' ').trim() : '';
  }
  set title(v) {
    let t = this.querySelector('title');
    if (!t) {
      const h = this.head;
      if (!h) return;
      t = this.createElement('title');
      h.appendChild(t);
    }
    t.textContent = String(v);
  }
  get cookie() { return Array.from(cookieJar, ([k, v]) => k + '=' + v).join('; '); }
  set cookie(s) {
    const parts = String(s).split(';');
    const eq = parts[0].indexOf('=');
    if (eq < 0) return;
    const name = parts[0].slice(0, eq).trim();
    const val = parts[0].slice(eq + 1).trim();
    let expired = false;
    for (const attr of parts.slice(1)) {
      const i = attr.indexOf('=');
      const key = (i < 0 ? attr : attr.slice(0, i)).trim().toLowerCase();
      const v = i < 0 ? '' : attr.slice(i + 1).trim();
      if (key === 'max-age' && Number(v) <= 0) expired = true;
      if (key === 'expires') { const t = Date.parse(v); if (!isNaN(t) && t <= Date.now()) expired = true; }
    }
    if (expired) cookieJar.delete(name); else cookieJar.set(name, val);
  }
}
Object.defineProperties(Document.prototype, Object.getOwnPropertyDescriptors(ParentMixin));
const document = wrap(0);
globalThis.Document = Document;
globalThis.HTMLDocument = Document;
globalThis.document = document;
"####;
