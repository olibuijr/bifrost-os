//! Network bridge: `fetch` and `XMLHttpRequest` queue a `NetRequest` for the
//! loader and settle when `JsPage::complete_request` hands back the
//! `NetResponse`. Bodies cross as UTF-8 text (binary bodies are lossy).

use super::{arg_bool, arg_str, host, register as register_table, NativeFn};
use boa_engine::{Context, JsResult, JsValue};

/// A request a script made, for the loader to perform.
#[derive(Clone, Debug, PartialEq)]
pub struct NetRequest {
    pub id: u32,
    pub method: String,
    /// Absolute URL.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// The loader's answer to a `NetRequest`. `error` set means a network failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Final URL after redirects (empty keeps the request URL).
    pub url: String,
    pub error: Option<String>,
}

impl NetResponse {
    /// A successful response with a text body.
    pub fn ok(status: u16, body: &str) -> NetResponse {
        NetResponse { status, status_text: "OK".into(), body: body.as_bytes().to_vec(), ..NetResponse::default() }
    }

    /// A network failure.
    pub fn failed(message: &str) -> NetResponse {
        NetResponse { error: Some(message.to_owned()), ..NetResponse::default() }
    }

    /// Adds a response header.
    pub fn header(mut self, name: &str, value: &str) -> NetResponse {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

#[derive(Debug, Default)]
pub(crate) struct Net {
    next_id: u32,
    pub queue: Vec<NetRequest>,
}

impl Net {
    pub fn push(&mut self, method: &str, url: &str, headers: Vec<(String, String)>, body: Option<Vec<u8>>) -> u32 {
        self.next_id += 1;
        self.queue.push(NetRequest {
            id: self.next_id,
            method: method.to_owned(),
            url: url.to_owned(),
            headers,
            body,
        });
        self.next_id
    }

    pub fn take(&mut self) -> Vec<NetRequest> {
        std::mem::take(&mut self.queue)
    }
}

/// `name: value` lines to pairs; blank and malformed lines are skipped.
pub(crate) fn parse_header_lines(s: &str) -> Vec<(String, String)> {
    s.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_owned(), v.trim().to_owned()))
        })
        .collect()
}

pub(crate) fn header_lines(h: &[(String, String)]) -> String {
    h.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n")
}

/// `net_request(method, url, headerLines, body, hasBody)`: queues a request, returns its id.
fn net_request(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let method = arg_str(a, 0, c)?;
    let url = arg_str(a, 1, c)?;
    let headers = parse_header_lines(&arg_str(a, 2, c)?);
    let body = arg_str(a, 3, c)?;
    let body = arg_bool(a, 4).then(|| body.into_bytes());
    let id = host(c).net.borrow_mut().push(&method, &url, headers, body);
    Ok(JsValue::from(f64::from(id)))
}

const TABLE: &[(&str, NativeFn)] = &[("net_request", net_request)];

pub(crate) fn register(ctx: &mut Context) {
    register_table(ctx, TABLE);
}

pub(crate) const JS: &str = r####"
const PENDING = new Map();

class Headers {
  constructor(init) {
    Object.defineProperty(this, '_m', { value: new Map() });
    if (!init) return;
    if (init instanceof Headers) init.forEach((v, k) => this.set(k, v));
    else if (Array.isArray(init)) for (const p of init) this.append(p[0], p[1]);
    else for (const k of Object.keys(init)) this.append(k, init[k]);
  }
  append(k, v) { k = String(k).toLowerCase(); v = String(v); this._m.set(k, this._m.has(k) ? this._m.get(k) + ', ' + v : v); }
  set(k, v) { this._m.set(String(k).toLowerCase(), String(v)); }
  get(k) { k = String(k).toLowerCase(); return this._m.has(k) ? this._m.get(k) : null; }
  has(k) { return this._m.has(String(k).toLowerCase()); }
  delete(k) { this._m.delete(String(k).toLowerCase()); }
  forEach(f, t) { for (const [k, v] of this._m) f.call(t, v, k, this); }
  keys() { return Array.from(this._m.keys())[Symbol.iterator](); }
  values() { return Array.from(this._m.values())[Symbol.iterator](); }
  entries() { return Array.from(this._m.entries())[Symbol.iterator](); }
  [Symbol.iterator]() { return this.entries(); }
}
const headerLines = (h) => Array.from(h.entries()).map((e) => e[0] + ': ' + e[1]).join('\n');
function parseHeaderLines(s) {
  const h = new Headers();
  for (const line of String(s).split('\n')) {
    const i = line.indexOf(':');
    if (i > 0) h.append(line.slice(0, i).trim(), line.slice(i + 1).trim());
  }
  return h;
}
function bodyToString(b) {
  if (typeof b === 'string') return b;
  if (b instanceof URLSearchParams) return b.toString();
  if (b instanceof ArrayBuffer || ArrayBuffer.isView(b)) return new TextDecoder().decode(b);
  return String(b);
}
const abortError = () => { const e = new Error('The operation was aborted.'); e.name = 'AbortError'; return e; };

class AbortSignal extends EventTarget {
  constructor() { super(); this.aborted = false; this.reason = undefined; this.onabort = null; }
  throwIfAborted() { if (this.aborted) throw this.reason; }
  static abort(r) { const s = new AbortSignal(); s.aborted = true; s.reason = r === undefined ? abortError() : r; return s; }
}
class AbortController {
  constructor() { this.signal = new AbortSignal(); }
  abort(reason) {
    const s = this.signal;
    if (s.aborted) return;
    s.aborted = true;
    s.reason = reason === undefined ? abortError() : reason;
    s.dispatchEvent(new Event('abort'));
  }
}

class Request {
  constructor(input, init) {
    init = init || {};
    const from = input instanceof Request ? input : null;
    this.url = new URL(from ? from.url : String(input), globalThis.location.href).href;
    this.method = String(init.method || (from && from.method) || 'GET').toUpperCase();
    this.headers = new Headers(init.headers !== undefined ? init.headers : from ? from.headers : undefined);
    const b = init.body !== undefined ? init.body : from ? from.body : null;
    this.body = b === null || b === undefined ? null : bodyToString(b);
    if (this.body !== null && !this.headers.has('content-type')) {
      this.headers.set('content-type', b instanceof URLSearchParams ? 'application/x-www-form-urlencoded;charset=UTF-8' : 'text/plain;charset=UTF-8');
    }
    this.signal = init.signal || (from && from.signal) || null;
    this.credentials = init.credentials || 'same-origin';
    this.mode = init.mode || 'cors';
    this.cache = init.cache || 'default';
    this.redirect = init.redirect || 'follow';
  }
  clone() { return new Request(this); }
}

class Response {
  constructor(body, init) {
    init = init || {};
    Object.defineProperty(this, '_body', { value: body === null || body === undefined ? '' : bodyToString(body), writable: true });
    this.status = init.status === undefined ? 200 : init.status;
    this.statusText = init.statusText || '';
    this.headers = init.headers instanceof Headers ? init.headers : new Headers(init.headers);
    this.url = init.url || '';
    this.ok = this.status >= 200 && this.status < 300;
    this.redirected = false;
    this.type = 'basic';
    this.bodyUsed = false;
  }
  _take() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    return Promise.resolve(this._body);
  }
  text() { return this._take(); }
  json() { return this._take().then((t) => JSON.parse(t)); }
  arrayBuffer() { return this._take().then((t) => new TextEncoder().encode(t).buffer); }
  clone() { return new Response(this._body, { status: this.status, statusText: this.statusText, headers: new Headers(this.headers), url: this.url }); }
  static error() { const r = new Response(null, { status: 0 }); r.type = 'error'; return r; }
  static json(data, init) {
    const r = new Response(JSON.stringify(data), init);
    if (!r.headers.has('content-type')) r.headers.set('content-type', 'application/json');
    return r;
  }
}

globalThis.fetch = function fetch(input, init) {
  return new Promise((resolve, reject) => {
    try {
      const req = input instanceof Request && init === undefined ? input : new Request(input, init);
      if (req.signal && req.signal.aborted) { reject(req.signal.reason); return; }
      const id = H.net_request(req.method, req.url, headerLines(req.headers), req.body === null ? '' : req.body, req.body !== null);
      PENDING.set(id, { fetch: { resolve, reject }, url: req.url });
      if (req.signal) req.signal.addEventListener('abort', () => { if (PENDING.delete(id)) reject(req.signal.reason); });
    } catch (e) { reject(e); }
  });
};

class ProgressEvent extends Event {
  constructor(type, init) {
    super(type, init);
    this.loaded = (init && init.loaded) || 0;
    this.total = (init && init.total) || 0;
    this.lengthComputable = !!(init && init.lengthComputable);
  }
}

class XMLHttpRequest extends EventTarget {
  constructor() {
    super();
    this.readyState = 0;
    this.status = 0;
    this.statusText = '';
    this.responseText = '';
    this.response = '';
    this.responseType = '';
    this.responseURL = '';
    this.timeout = 0;
    this.withCredentials = false;
    this.upload = new EventTarget();
    Object.defineProperty(this, '_m', { value: { method: 'GET', url: '', req: new Headers(), res: null, id: null }, writable: true });
  }
  open(method, url) {
    this._m = { method: String(method).toUpperCase(), url: new URL(String(url), globalThis.location.href).href, req: new Headers(), res: null, id: null };
    this._state(1);
  }
  setRequestHeader(k, v) {
    if (this.readyState !== 1) throw new Error('InvalidStateError: the request is not opened');
    this._m.req.append(k, v);
  }
  send(body) {
    if (this.readyState !== 1) throw new Error('InvalidStateError: the request is not opened');
    const m = this._m;
    const has = body !== undefined && body !== null && m.method !== 'GET' && m.method !== 'HEAD';
    const text = has ? bodyToString(body) : '';
    m.id = H.net_request(m.method, m.url, headerLines(m.req), text, has);
    PENDING.set(m.id, { xhr: this, url: m.url });
    this._emit('loadstart');
  }
  abort() {
    const m = this._m;
    if (m.id !== null && PENDING.delete(m.id)) {
      m.id = null;
      this.readyState = 4;
      this._emit('readystatechange');
      this._emit('abort');
      this._emit('loadend');
      this.readyState = 0;
    }
  }
  getResponseHeader(n) { return this._m.res ? this._m.res.get(n) : null; }
  getAllResponseHeaders() {
    return this._m.res ? Array.from(this._m.res.entries()).map((e) => e[0] + ': ' + e[1] + '\r\n').join('') : '';
  }
  overrideMimeType() {}
  _emit(type) { this.dispatchEvent(new ProgressEvent(type)); }
  _state(s) { this.readyState = s; this._emit('readystatechange'); }
  _finish(status, statusText, headers, body, url, error) {
    this._m.id = null;
    if (error) {
      this.readyState = 4;
      this._emit('readystatechange');
      this._emit('error');
      this._emit('loadend');
      return;
    }
    this.status = status;
    this.statusText = statusText;
    this._m.res = headers;
    this.responseURL = url;
    this._state(2);
    this._state(3);
    this.responseText = body;
    if (this.responseType === 'json') {
      try { this.response = JSON.parse(body); } catch (e) { this.response = null; }
    } else this.response = body;
    this._state(4);
    this._emit('load');
    this._emit('loadend');
  }
}
XMLHttpRequest.UNSENT = 0; XMLHttpRequest.OPENED = 1; XMLHttpRequest.HEADERS_RECEIVED = 2; XMLHttpRequest.LOADING = 3; XMLHttpRequest.DONE = 4;

// Called by the host with a finished request.
globalThis.__netDone = function (id, status, statusText, lines, body, url, error) {
  const p = PENDING.get(id);
  if (!p) return;
  PENDING.delete(id);
  const headers = parseHeaderLines(lines);
  if (p.fetch) {
    if (error) p.fetch.reject(new TypeError('Failed to fetch: ' + error));
    else p.fetch.resolve(new Response(body, { status, statusText, headers, url: url || p.url }));
  } else p.xhr._finish(status, statusText, headers, body, url || p.url, error);
};
Object.defineProperty(globalThis, '__netDone', { enumerable: false });

globalThis.Headers = Headers;
globalThis.Request = Request;
globalThis.Response = Response;
globalThis.AbortController = AbortController;
globalThis.AbortSignal = AbortSignal;
globalThis.ProgressEvent = ProgressEvent;
globalThis.XMLHttpRequest = XMLHttpRequest;
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_get_increasing_ids_and_drain() {
        let mut n = Net::default();
        let a = n.push("GET", "http://a/", vec![], None);
        let b = n.push("POST", "http://b/", vec![("x".into(), "y".into())], Some(b"hi".to_vec()));
        assert_eq!((a, b), (1, 2));
        let q = n.take();
        assert_eq!(q.len(), 2);
        assert_eq!(q[1].body.as_deref(), Some(&b"hi"[..]));
        assert!(n.take().is_empty());
    }

    #[test]
    fn header_lines_round_trip() {
        let h = vec![("accept".to_owned(), "text/html".to_owned()), ("x-a".to_owned(), "b: c".to_owned())];
        assert_eq!(parse_header_lines(&header_lines(&h)), h);
        assert!(parse_header_lines("garbage\n: nameless\n").is_empty());
    }

    #[test]
    fn response_builders() {
        let r = NetResponse::ok(200, "x").header("content-type", "text/plain");
        assert_eq!(r.body, b"x");
        assert_eq!(r.headers.len(), 1);
        assert_eq!(NetResponse::failed("boom").error.as_deref(), Some("boom"));
    }
}
