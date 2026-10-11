//! Node and Element bindings: the native tree/attribute/text bridge over
//! `Dom`, and the JavaScript `Node`, `CharacterData`, `Text`, `Comment` and
//! `Element` classes with a wrapper cache (one JS object per `NodeId`).

use super::{
    arg_node, arg_opt_node, arg_str, js_string, node_list, node_value, register as register_table, host,
    NativeFn,
};
use crate::shell::browser::dom::{Dom, NodeKind};
use boa_engine::{Context, JsResult, JsValue};

pub(super) fn with_dom<R>(c: &Context, f: impl FnOnce(&Dom) -> R) -> R {
    let h = host(c);
    let d = h.dom.borrow();
    f(&d)
}

pub(super) fn with_dom_mut<R>(c: &Context, f: impl FnOnce(&mut Dom) -> R) -> R {
    let h = host(c);
    let mut d = h.dom.borrow_mut();
    f(&mut d)
}

fn opt_string(s: Option<String>) -> JsValue {
    match s {
        Some(s) => js_string(&s),
        None => JsValue::null(),
    }
}

fn kind(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let k = with_dom(c, |d| match d.kind(n) {
        NodeKind::Document => 9.0,
        NodeKind::Element { .. } => 1.0,
        NodeKind::Text(_) => 3.0,
        NodeKind::Comment(_) => 8.0,
    });
    Ok(JsValue::from(k))
}

fn tag(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(opt_string(with_dom(c, |d| d.tag(n).map(str::to_owned))))
}

fn parent(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(node_value(with_dom(c, |d| d.parent(n))))
}

fn first(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(node_value(with_dom(c, |d| d.first_child(n))))
}

fn last(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(node_value(with_dom(c, |d| d.last_child(n))))
}

fn next(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(node_value(with_dom(c, |d| d.next_sibling(n))))
}

fn prev(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(node_value(with_dom(c, |d| d.prev_sibling(n))))
}

fn children(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let ids: Vec<_> = with_dom(c, |d| d.children(n).collect());
    Ok(node_list(ids.into_iter()))
}

fn create_element(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let t = arg_str(a, 0, c)?;
    Ok(node_value(Some(with_dom_mut(c, |d| d.create_element(&t)))))
}

fn create_text(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let t = arg_str(a, 0, c)?;
    Ok(node_value(Some(with_dom_mut(c, |d| d.create_text(&t)))))
}

fn create_comment(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let t = arg_str(a, 0, c)?;
    Ok(node_value(Some(with_dom_mut(c, |d| d.create_comment(&t)))))
}

fn append(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let p = arg_node(a, 0, c)?;
    let ch = arg_node(a, 1, c)?;
    with_dom_mut(c, |d| d.append_child(p, ch));
    Ok(JsValue::undefined())
}

fn insert_before(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let p = arg_node(a, 0, c)?;
    let ch = arg_node(a, 1, c)?;
    let before = arg_opt_node(a, 2, c)?;
    with_dom_mut(c, |d| d.insert_before(p, ch, before));
    Ok(JsValue::undefined())
}

fn remove(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    with_dom_mut(c, |d| d.remove(n));
    Ok(JsValue::undefined())
}

fn get_attr(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let name = arg_str(a, 1, c)?;
    Ok(opt_string(with_dom(c, |d| d.attr(n, &name).map(str::to_owned))))
}

fn set_attr(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let name = arg_str(a, 1, c)?;
    let value = arg_str(a, 2, c)?;
    with_dom_mut(c, |d| d.set_attr(n, &name, &value));
    Ok(JsValue::undefined())
}

fn remove_attr(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let name = arg_str(a, 1, c)?;
    with_dom_mut(c, |d| d.remove_attr(n, &name));
    Ok(JsValue::undefined())
}

/// Attribute names joined by newlines.
fn attr_names(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let names: Vec<String> = with_dom(c, |d| d.attrs(n).iter().map(|(k, _)| k.clone()).collect());
    Ok(js_string(&names.join("\n")))
}

/// The content of a text or comment node.
fn data(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(js_string(&with_dom(c, |d| d.text(n).unwrap_or("").to_owned())))
}

fn set_data(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let t = arg_str(a, 1, c)?;
    with_dom_mut(c, |d| d.set_text(n, &t));
    Ok(JsValue::undefined())
}

fn text_content(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(js_string(&with_dom(c, |d| d.text_content(n))))
}

fn set_text_content(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let t = arg_str(a, 1, c)?;
    with_dom_mut(c, |d| d.set_text_content(n, &t));
    Ok(JsValue::undefined())
}

fn inner_html(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(js_string(&with_dom(c, |d| d.inner_html(n))))
}

fn outer_html(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    Ok(js_string(&with_dom(c, |d| d.outer_html(n))))
}

fn set_inner_html(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let n = arg_node(a, 0, c)?;
    let t = arg_str(a, 1, c)?;
    with_dom_mut(c, |d| d.set_inner_html(n, &t));
    Ok(JsValue::undefined())
}

const TABLE: &[(&str, NativeFn)] = &[
    ("kind", kind),
    ("tag", tag),
    ("parent", parent),
    ("first", first),
    ("last", last),
    ("next", next),
    ("prev", prev),
    ("children", children),
    ("create_element", create_element),
    ("create_text", create_text),
    ("create_comment", create_comment),
    ("append", append),
    ("insert_before", insert_before),
    ("remove", remove),
    ("get_attr", get_attr),
    ("set_attr", set_attr),
    ("remove_attr", remove_attr),
    ("attr_names", attr_names),
    ("data", data),
    ("set_data", set_data),
    ("text_content", text_content),
    ("set_text_content", set_text_content),
    ("inner_html", inner_html),
    ("outer_html", outer_html),
    ("set_inner_html", set_inner_html),
];

pub(crate) fn register(ctx: &mut Context) {
    register_table(ctx, TABLE);
}

pub(crate) const JS: &str = r####"
const WRAPS = new Map();
const isFrag = (id) => H.tag(id) === '#fragment';
const attrNames = (id) => { const s = H.attr_names(id); return s === '' ? [] : s.split('\n'); };
function wrap(id) {
  id = +id;
  if (!(id >= 0)) return null;
  let w = WRAPS.get(id);
  if (w) return w;
  const k = H.kind(id);
  w = k === 9 ? new Document(id) : k === 1 ? new HTMLElement(id) : k === 3 ? new Text(id) : new Comment(id);
  WRAPS.set(id, w);
  return w;
}
const idOf = (n) => {
  if (!(n instanceof Node)) throw new TypeError('argument is not a Node');
  return n._id;
};
const wrapList = (s) => (s === '' ? [] : s.split(',').map((x) => wrap(+x)));
const toNode = (x) => (x instanceof Node ? x : document.createTextNode(String(x)));

function cloneId(id, deep) {
  const k = H.kind(id);
  if (k === 3) return H.create_text(H.data(id));
  if (k === 8) return H.create_comment(H.data(id));
  if (k === 9) throw new Error('NotSupportedError: cannot clone a document');
  const c = H.create_element(H.tag(id));
  for (const n of attrNames(id)) H.set_attr(c, n, H.get_attr(id, n));
  if (deep) for (let ch = H.first(id); ch >= 0; ch = H.next(ch)) H.append(c, cloneId(ch, true));
  return c;
}

class Node extends EventTarget {
  constructor(id) {
    super();
    Object.defineProperty(this, '_id', { value: id });
  }
  get nodeType() {
    const k = H.kind(this._id);
    return k === 1 && isFrag(this._id) ? 11 : k;
  }
  get nodeName() {
    const k = H.kind(this._id);
    if (k === 1) return isFrag(this._id) ? '#document-fragment' : H.tag(this._id).toUpperCase();
    return k === 3 ? '#text' : k === 8 ? '#comment' : '#document';
  }
  get parentNode() { return wrap(H.parent(this._id)); }
  get parentElement() { const p = this.parentNode; return p && p.nodeType === 1 ? p : null; }
  get firstChild() { return wrap(H.first(this._id)); }
  get lastChild() { return wrap(H.last(this._id)); }
  get nextSibling() { return wrap(H.next(this._id)); }
  get previousSibling() { return wrap(H.prev(this._id)); }
  get childNodes() { return wrapList(H.children(this._id)); }
  hasChildNodes() { return H.first(this._id) >= 0; }
  get ownerDocument() { return this._id === 0 ? null : document; }
  get isConnected() {
    let n = this._id;
    while (n >= 0) { if (n === 0) return true; n = H.parent(n); }
    return false;
  }
  getRootNode() {
    let n = this._id;
    while (H.parent(n) >= 0) n = H.parent(n);
    return wrap(n);
  }
  contains(o) {
    if (!o) return false;
    let n = o._id;
    while (n >= 0) { if (n === this._id) return true; n = H.parent(n); }
    return false;
  }
  get nodeValue() { const k = H.kind(this._id); return k === 3 || k === 8 ? H.data(this._id) : null; }
  set nodeValue(v) { const k = H.kind(this._id); if (k === 3 || k === 8) H.set_data(this._id, String(v)); }
  get textContent() { return this._id === 0 ? null : H.text_content(this._id); }
  set textContent(v) {
    const k = H.kind(this._id);
    const s = v === null || v === undefined ? '' : String(v);
    if (k === 3 || k === 8) H.set_data(this._id, s);
    else if (k === 1) H.set_text_content(this._id, s);
  }
  appendChild(c) { return this.insertBefore(c, null); }
  insertBefore(c, ref) {
    const cid = idOf(c);
    let refId = ref === null || ref === undefined ? -1 : idOf(ref);
    if (refId >= 0 && H.parent(refId) !== this._id) throw new Error('NotFoundError: the reference node is not a child of this node');
    if (c.contains(this)) throw new Error('HierarchyRequestError: the new child contains the parent');
    if (refId === cid) refId = H.next(cid);
    if (isFrag(cid)) {
      let k;
      while ((k = H.first(cid)) >= 0) H.insert_before(this._id, k, refId);
      return c;
    }
    H.insert_before(this._id, cid, refId);
    return c;
  }
  removeChild(c) {
    if (H.parent(idOf(c)) !== this._id) throw new Error('NotFoundError: the node is not a child of this node');
    H.remove(c._id);
    return c;
  }
  replaceChild(n, old) {
    if (n === old) return old;
    this.insertBefore(n, old);
    this.removeChild(old);
    return old;
  }
  remove() { if (H.parent(this._id) >= 0) H.remove(this._id); }
  before(...xs) { const p = this.parentNode; if (!p) return; for (const x of xs) p.insertBefore(toNode(x), this); }
  after(...xs) { const p = this.parentNode; if (!p) return; const ref = this.nextSibling; for (const x of xs) p.insertBefore(toNode(x), ref); }
  replaceWith(...xs) {
    const p = this.parentNode;
    if (!p) return;
    const ref = this.nextSibling;
    this.remove();
    for (const x of xs) p.insertBefore(toNode(x), ref);
  }
  cloneNode(deep) { return wrap(cloneId(this._id, !!deep)); }
  normalize() {}
  isEqualNode(o) { return !!o && this.nodeType === o.nodeType && this.nodeName === o.nodeName && (this.nodeType === 1 ? H.outer_html(this._id) === H.outer_html(o._id) : this.nodeValue === o.nodeValue); }
  isSameNode(o) { return this === o; }
}
Node.ELEMENT_NODE = 1; Node.TEXT_NODE = 3; Node.COMMENT_NODE = 8; Node.DOCUMENT_NODE = 9; Node.DOCUMENT_FRAGMENT_NODE = 11;

class CharacterData extends Node {
  get data() { return H.data(this._id); }
  set data(v) { H.set_data(this._id, String(v)); }
  get length() { return H.data(this._id).length; }
  get nextElementSibling() { let n = this.nextSibling; while (n && n.nodeType !== 1) n = n.nextSibling; return n; }
  get previousElementSibling() { let n = this.previousSibling; while (n && n.nodeType !== 1) n = n.previousSibling; return n; }
}
class Text extends CharacterData {
  get wholeText() { return this.data; }
}
class Comment extends CharacterData {}

// ---- inline style, classList, dataset ----
function splitDecls(s) {
  const out = [];
  let cur = '', depth = 0, q = '';
  for (const ch of s) {
    if (q) { cur += ch; if (ch === q) q = ''; }
    else if (ch === '"' || ch === "'") { q = ch; cur += ch; }
    else if (ch === '(') { depth++; cur += ch; }
    else if (ch === ')') { depth--; cur += ch; }
    else if (ch === ';' && depth <= 0) { out.push(cur); cur = ''; }
    else cur += ch;
  }
  out.push(cur);
  return out;
}
function parseStyle(s) {
  const m = [];
  for (const d of splitDecls(s || '')) {
    const i = d.indexOf(':');
    if (i < 0) continue;
    let k = d.slice(0, i).trim();
    if (!k.startsWith('--')) k = k.toLowerCase();
    const v = d.slice(i + 1).trim();
    if (k) m.push([k, v]);
  }
  return m;
}
function writeStyle(el, m) {
  if (m.length === 0) el.removeAttribute('style');
  else el.setAttribute('style', m.map(([k, v]) => k + ': ' + v).join('; ') + ';');
}
const kebab = (p) => (p === 'cssFloat' ? 'float' : p.startsWith('--') ? p : p.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()));
function makeStyle(el) {
  const api = {
    getPropertyValue(n) {
      const k = kebab(String(n));
      const e = parseStyle(el.getAttribute('style')).find((x) => x[0] === k);
      return e ? e[1] : '';
    },
    setProperty(n, v, pri) {
      const k = kebab(String(n));
      const val = v === null || v === undefined ? '' : String(v);
      const m = parseStyle(el.getAttribute('style'));
      const i = m.findIndex((x) => x[0] === k);
      if (val === '') { if (i >= 0) m.splice(i, 1); }
      else {
        const full = pri === 'important' ? val + ' !important' : val;
        if (i >= 0) m[i][1] = full; else m.push([k, full]);
      }
      writeStyle(el, m);
    },
    removeProperty(n) { const old = api.getPropertyValue(n); api.setProperty(n, ''); return old; },
    item(i) { const m = parseStyle(el.getAttribute('style')); return m[i] ? m[i][0] : ''; },
    get length() { return parseStyle(el.getAttribute('style')).length; },
    get cssText() { return el.getAttribute('style') || ''; },
    set cssText(v) { el.setAttribute('style', String(v)); },
  };
  return new Proxy(api, {
    get(t, p) {
      if (typeof p === 'symbol' || p in t) return Reflect.get(t, p, t);
      return t.getPropertyValue(p);
    },
    set(t, p, v) {
      if (p === 'cssText') { t.cssText = v; return true; }
      if (typeof p === 'symbol') return false;
      t.setProperty(p, v);
      return true;
    },
  });
}
function makeClassList(el) {
  const get = () => (el.getAttribute('class') || '').split(/\s+/).filter(Boolean);
  const put = (a) => el.setAttribute('class', a.join(' '));
  const api = {
    add(...c) { const a = get(); for (const x of c.map(String)) if (!a.includes(x)) a.push(x); put(a); },
    remove(...c) { const r = c.map(String); put(get().filter((x) => !r.includes(x))); },
    toggle(c, force) {
      c = String(c);
      const has = get().includes(c);
      const want = force === undefined ? !has : !!force;
      if (want && !has) api.add(c);
      else if (!want && has) api.remove(c);
      return want;
    },
    contains(c) { return get().includes(String(c)); },
    replace(o, n) {
      o = String(o); n = String(n);
      const a = get();
      const i = a.indexOf(o);
      if (i < 0) return false;
      a[i] = n;
      put(a.filter((x, j) => x !== n || j === i));
      return true;
    },
    item(i) { const a = get(); return i < a.length ? a[i] : null; },
    get length() { return get().length; },
    get value() { return el.getAttribute('class') || ''; },
    set value(v) { el.setAttribute('class', String(v)); },
    toString() { return el.getAttribute('class') || ''; },
    forEach(f, t) { get().forEach((x, i) => f.call(t, x, i, api)); },
    [Symbol.iterator]() { return get()[Symbol.iterator](); },
  };
  return api;
}
const camel = (s) => s.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
const dashed = (s) => s.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
function makeDataset(el) {
  const attr = (p) => 'data-' + dashed(p);
  return new Proxy({}, {
    get(t, p) { if (typeof p === 'symbol') return undefined; const v = el.getAttribute(attr(p)); return v === null ? undefined : v; },
    set(t, p, v) { el.setAttribute(attr(String(p)), String(v)); return true; },
    has(t, p) { return typeof p === 'string' && el.hasAttribute(attr(p)); },
    deleteProperty(t, p) { el.removeAttribute(attr(String(p))); return true; },
    ownKeys() { return el.getAttributeNames().filter((n) => n.startsWith('data-')).map((n) => camel(n.slice(5))); },
    getOwnPropertyDescriptor(t, p) {
      if (typeof p === 'string' && el.hasAttribute(attr(p))) return { value: el.getAttribute(attr(p)), writable: true, enumerable: true, configurable: true };
      return undefined;
    },
  });
}

const CACHES = new WeakMap();
function cached(el, key, make) {
  let m = CACHES.get(el);
  if (!m) { m = {}; CACHES.set(el, m); }
  return m[key] || (m[key] = make(el));
}

function query(root, sel, first) {
  const s = H.query(root._id, String(sel), first);
  return s === '' ? [] : s.split(',').map((x) => wrap(+x));
}

// ParentNode behaviour shared by Element and Document.
const ParentMixin = {
  get children() { return wrapList(H.children(this._id)).filter((n) => n.nodeType === 1); },
  get childElementCount() { return this.children.length; },
  get firstElementChild() { return this.children[0] || null; },
  get lastElementChild() { const c = this.children; return c[c.length - 1] || null; },
  append(...xs) { for (const x of xs) this.appendChild(toNode(x)); },
  prepend(...xs) { const f = this.firstChild; for (const x of xs) this.insertBefore(toNode(x), f); },
  replaceChildren(...xs) { while (this.firstChild) this.removeChild(this.firstChild); this.append(...xs); },
  querySelector(sel) { return query(this, sel, true)[0] || null; },
  querySelectorAll(sel) { return query(this, sel, false); },
  getElementsByTagName(t) { return query(this, String(t).toLowerCase() === '*' ? '*' : String(t).toLowerCase(), false); },
  getElementsByClassName(c) {
    const names = String(c).split(/\s+/).filter(Boolean);
    return names.length ? query(this, names.map((n) => '.' + n).join(''), false) : [];
  },
};

const REFLECT = {
  id: 'id', className: 'class', title: 'title', lang: 'lang', dir: 'dir', alt: 'alt', name: 'name', type: 'type',
  value: 'value', placeholder: 'placeholder', rel: 'rel', target: 'target', htmlFor: 'for', method: 'method',
  role: 'role', tabIndexAttr: 'tabindex',
};
const URLISH = { href: 'href', src: 'src', action: 'action' };
const BOOLS = {
  disabled: 'disabled', checked: 'checked', hidden: 'hidden', selected: 'selected', readOnly: 'readonly',
  required: 'required', autofocus: 'autofocus', multiple: 'multiple', open: 'open',
};

class Element extends Node {
  get tagName() { return H.tag(this._id).toUpperCase(); }
  get localName() { return H.tag(this._id); }
  get namespaceURI() { return 'http://www.w3.org/1999/xhtml'; }
  getAttribute(n) { return H.get_attr(this._id, String(n).toLowerCase()); }
  setAttribute(n, v) { H.set_attr(this._id, String(n).toLowerCase(), String(v)); }
  removeAttribute(n) { H.remove_attr(this._id, String(n).toLowerCase()); }
  hasAttribute(n) { return H.get_attr(this._id, String(n).toLowerCase()) !== null; }
  hasAttributes() { return attrNames(this._id).length > 0; }
  getAttributeNames() { return attrNames(this._id); }
  toggleAttribute(n, force) {
    const has = this.hasAttribute(n);
    const want = force === undefined ? !has : !!force;
    if (want && !has) this.setAttribute(n, '');
    else if (!want && has) this.removeAttribute(n);
    return want;
  }
  get attributes() {
    return attrNames(this._id).map((n) => {
      const value = H.get_attr(this._id, n);
      return { name: n, value, nodeName: n, nodeValue: value };
    });
  }
  get classList() { return cached(this, 'classList', makeClassList); }
  get dataset() { return cached(this, 'dataset', makeDataset); }
  get style() { return cached(this, 'style', makeStyle); }
  set style(v) { this.style.cssText = v; }
  get innerHTML() { return H.inner_html(this._id); }
  set innerHTML(v) { H.set_inner_html(this._id, v === null || v === undefined ? '' : String(v)); }
  get outerHTML() { return H.outer_html(this._id); }
  set outerHTML(v) {
    const p = this.parentNode;
    if (!p) return;
    const tmp = document.createElement('div');
    tmp.innerHTML = String(v);
    while (tmp.firstChild) p.insertBefore(tmp.firstChild, this);
    this.remove();
  }
  get innerText() { return H.text_content(this._id); }
  set innerText(v) { this.textContent = v; }
  insertAdjacentHTML(pos, html) {
    const tmp = document.createElement('div');
    tmp.innerHTML = String(html);
    const kids = Array.from(tmp.childNodes);
    this._adjacent(pos, kids);
  }
  insertAdjacentText(pos, text) { this._adjacent(pos, [document.createTextNode(String(text))]); }
  insertAdjacentElement(pos, el) { this._adjacent(pos, [el]); return el; }
  _adjacent(pos, nodes) {
    switch (String(pos).toLowerCase()) {
      case 'beforebegin': { const p = this.parentNode; if (p) for (const n of nodes) p.insertBefore(n, this); break; }
      case 'afterbegin': { const f = this.firstChild; for (const n of nodes) this.insertBefore(n, f); break; }
      case 'beforeend': for (const n of nodes) this.appendChild(n); break;
      case 'afterend': { const p = this.parentNode; const ref = this.nextSibling; if (p) for (const n of nodes) p.insertBefore(n, ref); break; }
      default: throw new Error('SyntaxError: invalid insertAdjacent position');
    }
  }
  matches(sel) { return H.matches(this._id, String(sel)); }
  webkitMatchesSelector(sel) { return this.matches(sel); }
  closest(sel) { return wrap(H.closest(this._id, String(sel))); }
  get nextElementSibling() { let n = this.nextSibling; while (n && n.nodeType !== 1) n = n.nextSibling; return n; }
  get previousElementSibling() { let n = this.previousSibling; while (n && n.nodeType !== 1) n = n.previousSibling; return n; }
  getBoundingClientRect() { return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON() { return this; } }; }
  getClientRects() { return []; }
  get offsetWidth() { return 0; }
  get offsetHeight() { return 0; }
  get offsetTop() { return 0; }
  get offsetLeft() { return 0; }
  get offsetParent() { return null; }
  get clientWidth() { return 0; }
  get clientHeight() { return 0; }
  get clientTop() { return 0; }
  get clientLeft() { return 0; }
  get scrollWidth() { return 0; }
  get scrollHeight() { return 0; }
  get scrollTop() { return 0; }
  set scrollTop(v) {}
  get scrollLeft() { return 0; }
  set scrollLeft(v) {}
  scrollIntoView() {}
  scrollTo() {}
  scrollBy() {}
  focus() { document._active = this; }
  blur() { if (document._active === this) document._active = null; }
  click() { this.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })); }
  animate() { return { cancel() {}, finish() {}, play() {}, pause() {}, finished: Promise.resolve(), onfinish: null }; }
  attachShadow() { throw new Error('NotSupportedError: shadow DOM is not supported'); }
  get tabIndex() { const v = this.getAttribute('tabindex'); return v === null ? -1 : parseInt(v, 10) || 0; }
  set tabIndex(v) { this.setAttribute('tabindex', v); }
  get isContentEditable() { return this.getAttribute('contenteditable') === 'true'; }
}
class HTMLElement extends Element {}

for (const [p, a] of Object.entries(REFLECT)) {
  Object.defineProperty(Element.prototype, p, {
    get() { const v = this.getAttribute(a); return v === null ? '' : v; },
    set(v) { this.setAttribute(a, v); },
    configurable: true,
  });
}
for (const [p, a] of Object.entries(URLISH)) {
  Object.defineProperty(Element.prototype, p, {
    get() {
      const v = this.getAttribute(a);
      if (v === null) return '';
      try { return new URL(v, globalThis.location.href).href; } catch (e) { return v; }
    },
    set(v) { this.setAttribute(a, v); },
    configurable: true,
  });
}
for (const [p, a] of Object.entries(BOOLS)) {
  Object.defineProperty(Element.prototype, p, {
    get() { return this.hasAttribute(a); },
    set(v) { if (v) this.setAttribute(a, ''); else this.removeAttribute(a); },
    configurable: true,
  });
}
Object.defineProperties(Element.prototype, Object.getOwnPropertyDescriptors(ParentMixin));

globalThis.Node = Node;
globalThis.CharacterData = CharacterData;
globalThis.Text = Text;
globalThis.Comment = Comment;
globalThis.Element = Element;
globalThis.HTMLElement = HTMLElement;
"####;
