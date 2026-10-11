//! `URL` and `URLSearchParams`: parsing and resolution run in Rust over the
//! `url` crate, the classes and form encoding in the prelude.

use super::{arg_str, js_string, register as register_table, NativeFn};
use boa_engine::{Context, JsResult, JsValue};

/// The WHATWG components of `href` resolved against `base` (empty for none):
/// href, protocol, username, password, hostname, port, pathname, search, hash, origin.
pub(crate) fn parse_parts(href: &str, base: &str) -> Option<Vec<String>> {
    let u = if base.is_empty() {
        ::url::Url::parse(href).ok()?
    } else {
        ::url::Url::parse(base).ok()?.join(href).ok()?
    };
    let search = match u.query() {
        Some(q) if !q.is_empty() => format!("?{q}"),
        _ => String::new(),
    };
    let hash = match u.fragment() {
        Some(f) if !f.is_empty() => format!("#{f}"),
        _ => String::new(),
    };
    Some(vec![
        u.as_str().to_owned(),
        format!("{}:", u.scheme()),
        u.username().to_owned(),
        u.password().unwrap_or("").to_owned(),
        u.host_str().unwrap_or("").to_owned(),
        u.port().map(|p| p.to_string()).unwrap_or_default(),
        u.path().to_owned(),
        search,
        hash,
        u.origin().ascii_serialization(),
    ])
}

/// `url_parse(href, base)`: the components joined by U+0001, or null.
fn url_parse(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let href = arg_str(a, 0, c)?;
    let base = arg_str(a, 1, c)?;
    Ok(match parse_parts(&href, &base) {
        Some(p) => js_string(&p.join("\u{1}")),
        None => JsValue::null(),
    })
}

const TABLE: &[(&str, NativeFn)] = &[("url_parse", url_parse)];

pub(crate) fn register(ctx: &mut Context) {
    register_table(ctx, TABLE);
}

pub(crate) const JS: &str = r####"
const formEncode = (s) =>
  encodeURIComponent(s).replace(/%20/g, '+').replace(/[!'()~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
const formDecode = (s) => {
  s = s.replace(/\+/g, ' ');
  try { return decodeURIComponent(s); } catch (e) { return s; }
};

class URLSearchParams {
  constructor(init) {
    Object.defineProperty(this, '_list', { value: [], writable: true });
    Object.defineProperty(this, '_url', { value: null, writable: true });
    if (init === undefined || init === null) return;
    if (init instanceof URLSearchParams) this._list = init._list.map((p) => p.slice());
    else if (typeof init === 'object' && typeof init[Symbol.iterator] === 'function') {
      for (const p of init) this._list.push([String(p[0]), String(p[1])]);
    } else if (typeof init === 'object') {
      for (const k of Object.keys(init)) this._list.push([k, String(init[k])]);
    } else this._parse(String(init));
  }
  _parse(s) {
    this._list = [];
    if (s.startsWith('?')) s = s.slice(1);
    for (const part of s.split('&')) {
      if (part === '') continue;
      const i = part.indexOf('=');
      this._list.push(i < 0 ? [formDecode(part), ''] : [formDecode(part.slice(0, i)), formDecode(part.slice(i + 1))]);
    }
  }
  _update() { if (this._url) this._url._setSearch(this.toString()); }
  get size() { return this._list.length; }
  append(k, v) { this._list.push([String(k), String(v)]); this._update(); }
  delete(k) { k = String(k); this._list = this._list.filter((p) => p[0] !== k); this._update(); }
  get(k) { k = String(k); const p = this._list.find((e) => e[0] === k); return p ? p[1] : null; }
  getAll(k) { k = String(k); return this._list.filter((e) => e[0] === k).map((e) => e[1]); }
  has(k) { k = String(k); return this._list.some((e) => e[0] === k); }
  set(k, v) {
    k = String(k); v = String(v);
    const i = this._list.findIndex((e) => e[0] === k);
    if (i < 0) this._list.push([k, v]);
    else {
      this._list[i][1] = v;
      this._list = this._list.filter((e, j) => j <= i || e[0] !== k);
    }
    this._update();
  }
  sort() { this._list.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)); this._update(); }
  forEach(f, t) { for (const [k, v] of this._list) f.call(t, v, k, this); }
  keys() { return this._list.map((p) => p[0])[Symbol.iterator](); }
  values() { return this._list.map((p) => p[1])[Symbol.iterator](); }
  entries() { return this._list.map((p) => [p[0], p[1]])[Symbol.iterator](); }
  [Symbol.iterator]() { return this.entries(); }
  toString() { return this._list.map((p) => formEncode(p[0]) + '=' + formEncode(p[1])).join('&'); }
}

class URL {
  constructor(url, base) {
    const r = H.url_parse(String(url), base === undefined ? '' : String(base));
    if (r === null) throw new TypeError("Failed to construct 'URL': Invalid URL: " + String(url));
    Object.defineProperty(this, '_p', { value: null, writable: true });
    Object.defineProperty(this, '_sp', { value: new URLSearchParams(), writable: true });
    this._sp._url = this;
    this._load(r);
  }
  _load(r) {
    const f = r.split('\u0001');
    this._p = { href: f[0], protocol: f[1], username: f[2], password: f[3], hostname: f[4], port: f[5], pathname: f[6], search: f[7], hash: f[8], origin: f[9] };
    this._sp._parse(this._p.search);
  }
  _rebuild(p) {
    const hasAuth = /^[a-z][a-z0-9+.-]*:\/\//i.test(this._p.href);
    const cred = p.username ? p.username + (p.password ? ':' + p.password : '') + '@' : '';
    const host = p.hostname + (p.port ? ':' + p.port : '');
    const href = hasAuth ? p.protocol + '//' + cred + host + p.pathname + p.search + p.hash : p.protocol + p.pathname + p.search + p.hash;
    const r = H.url_parse(href, '');
    if (r !== null) this._load(r);
  }
  _with(k, v) { const p = Object.assign({}, this._p); p[k] = v; this._rebuild(p); }
  _setSearch(s) {
    const p = Object.assign({}, this._p);
    p.search = s ? '?' + s : '';
    const keep = this._sp._list;
    this._rebuild(p);
    this._sp._list = keep;
  }
  get href() { return this._p.href; }
  set href(v) { const r = H.url_parse(String(v), ''); if (r === null) throw new TypeError('Invalid URL'); this._load(r); }
  get origin() { return this._p.origin; }
  get protocol() { return this._p.protocol; }
  set protocol(v) { this._with('protocol', String(v).replace(/:?$/, ':')); }
  get username() { return this._p.username; }
  set username(v) { this._with('username', String(v)); }
  get password() { return this._p.password; }
  set password(v) { this._with('password', String(v)); }
  get host() { return this._p.hostname + (this._p.port ? ':' + this._p.port : ''); }
  set host(v) {
    const m = String(v).split(':');
    const p = Object.assign({}, this._p);
    p.hostname = m[0];
    p.port = m[1] || '';
    this._rebuild(p);
  }
  get hostname() { return this._p.hostname; }
  set hostname(v) { this._with('hostname', String(v)); }
  get port() { return this._p.port; }
  set port(v) { this._with('port', String(v)); }
  get pathname() { return this._p.pathname; }
  set pathname(v) { v = String(v); this._with('pathname', v.startsWith('/') ? v : '/' + v); }
  get search() { return this._p.search; }
  set search(v) { v = String(v); this._with('search', v === '' || v === '?' ? '' : v.startsWith('?') ? v : '?' + v); }
  get hash() { return this._p.hash; }
  set hash(v) { v = String(v); this._with('hash', v === '' || v === '#' ? '' : v.startsWith('#') ? v : '#' + v); }
  get searchParams() { return this._sp; }
  toString() { return this._p.href; }
  toJSON() { return this._p.href; }
}
URL.canParse = (u, b) => H.url_parse(String(u), b === undefined ? '' : String(b)) !== null;

globalThis.URL = URL;
globalThis.URLSearchParams = URLSearchParams;
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_urls() {
        let p = parse_parts("https://user:pw@example.com:8080/a/b?x=1&y=2#top", "").unwrap();
        assert_eq!(p[1], "https:");
        assert_eq!(p[2], "user");
        assert_eq!(p[3], "pw");
        assert_eq!(p[4], "example.com");
        assert_eq!(p[5], "8080");
        assert_eq!(p[6], "/a/b");
        assert_eq!(p[7], "?x=1&y=2");
        assert_eq!(p[8], "#top");
        assert_eq!(p[9], "https://example.com:8080");
    }

    #[test]
    fn resolves_against_a_base() {
        let p = parse_parts("../c.png?v=2", "http://h.test/a/b/page.html").unwrap();
        assert_eq!(p[0], "http://h.test/a/c.png?v=2");
        let p = parse_parts("//cdn.test/x.js", "https://h.test/").unwrap();
        assert_eq!(p[0], "https://cdn.test/x.js");
    }

    #[test]
    fn default_port_and_empty_parts_are_empty() {
        let p = parse_parts("https://example.com:443/", "").unwrap();
        assert_eq!(p[5], "");
        assert_eq!(p[7], "");
        assert_eq!(p[8], "");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_parts("not a url", "").is_none());
        assert!(parse_parts("/relative", "").is_none());
    }
}
