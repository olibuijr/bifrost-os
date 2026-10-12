//! `window`/`globalThis`: `console` (into the page's log), `location`,
//! `navigator`, `history`, `screen`, `performance`, `atob`/`btoa`,
//! `TextEncoder`/`TextDecoder`, `matchMedia`, `getComputedStyle`, observer
//! stubs and the other small globals scripts probe for.

use super::{arg_str, host, register as register_table, NativeFn};
use boa_engine::{Context, JsResult, JsValue};

/// Formats a console line the way the page log stores it.
pub(crate) fn log_line(level: &str, msg: &str) -> String {
    match level {
        "warn" => format!("warn: {msg}"),
        "error" => format!("error: {msg}"),
        _ => msg.to_owned(),
    }
}

/// `console(level, message)`: appends to the page log.
fn console(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let level = arg_str(a, 0, c)?;
    let msg = arg_str(a, 1, c)?;
    host(c).log.borrow_mut().push(log_line(&level, &msg));
    Ok(JsValue::undefined())
}

const TABLE: &[(&str, NativeFn)] = &[("console", console)];

pub(crate) fn register(ctx: &mut Context) {
    register_table(ctx, TABLE);
}

pub(crate) const JS: &str = r####"
// ---- console ----
const fmtArg = (a) => {
  if (typeof a === 'string') return a;
  if (a === undefined) return 'undefined';
  if (a === null) return 'null';
  if (a instanceof Node) return '<' + a.nodeName.toLowerCase() + '>';
  if (a instanceof Error) return String(a);
  if (typeof a === 'object') { try { return JSON.stringify(a); } catch (e) { return String(a); } }
  return String(a);
};
const say = (level) => (...args) => { H.console(level, args.map(fmtArg).join(' ')); };
const COUNTS = new Map();
globalThis.console = {
  log: say('log'), info: say('log'), debug: say('log'), dir: say('log'), trace: say('log'),
  warn: say('warn'), error: say('error'),
  assert(cond, ...args) { if (!cond) H.console('error', 'Assertion failed' + (args.length ? ': ' + args.map(fmtArg).join(' ') : '')); },
  count(l) { l = l === undefined ? 'default' : String(l); const n = (COUNTS.get(l) || 0) + 1; COUNTS.set(l, n); H.console('log', l + ': ' + n); },
  group() {}, groupEnd() {}, groupCollapsed() {}, table: say('log'), clear() {},
  time() {}, timeEnd() {}, timeLog() {},
};

// ---- window ----
globalThis.window = globalThis;
globalThis.self = globalThis;
globalThis.top = globalThis;
globalThis.parent = globalThis;
globalThis.frames = globalThis;
for (const k of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
  const f = EventTarget.prototype[k];
  Object.defineProperty(globalThis, k, { value: function (...a) { return f.apply(globalThis, a); }, writable: true, configurable: true });
}

let loc;
try { loc = new URL(H.base_url()); } catch (e) { loc = new URL('about:blank'); }
loc.assign = function (u) { this.href = new URL(String(u), this.href).href; };
loc.replace = loc.assign;
loc.reload = function () {};
Object.defineProperty(globalThis, 'location', {
  get() { return loc; },
  set(v) { loc.href = new URL(String(v), loc.href).href; },
  configurable: true,
});

globalThis.navigator = {
  userAgent: 'Mozilla/5.0 (X11; Linux x86_64) WebKit/537.36 (KHTML, like Gecko) BifrOSt/1.0 WebBrowser/537.36',
  appName: 'Netscape', appVersion: '5.0', product: 'Gecko', vendor: 'SpringBoard',
  platform: 'Linux x86_64', language: 'en-US', languages: ['en-US', 'en'], onLine: true, cookieEnabled: true,
  hardwareConcurrency: 4, maxTouchPoints: 5, doNotTrack: null, webdriver: false,
  sendBeacon() { return false; },
  clipboard: { writeText() { return Promise.resolve(); }, readText() { return Promise.resolve(''); } },
};
globalThis.history = {
  length: 1, state: null, scrollRestoration: 'auto',
  pushState() {}, replaceState() {}, back() {}, forward() {}, go() {},
};
globalThis.screen = { width: 1200, height: 1920, availWidth: 1200, availHeight: 1920, colorDepth: 24, pixelDepth: 24, orientation: { type: 'portrait-primary', angle: 0 } };
globalThis.innerWidth = 1200;
globalThis.innerHeight = 1920;
globalThis.outerWidth = 1200;
globalThis.outerHeight = 1920;
globalThis.devicePixelRatio = 1;
globalThis.scrollX = 0; globalThis.scrollY = 0; globalThis.pageXOffset = 0; globalThis.pageYOffset = 0;
globalThis.scrollTo = function () {};
globalThis.scrollBy = function () {};
globalThis.scroll = function () {};
globalThis.alert = function () {};
globalThis.confirm = function () { return false; };
globalThis.prompt = function () { return null; };
globalThis.open = function () { return null; };
globalThis.close = function () {};
globalThis.focus = function () {};
globalThis.blur = function () {};
globalThis.print = function () {};
globalThis.stop = function () {};
globalThis.getSelection = function () { return null; };
globalThis.postMessage = function (data) {
  setTimeout(() => {
    const ev = new Event('message');
    ev.data = data;
    ev.origin = loc.origin;
    globalThis.dispatchEvent(ev);
  }, 0);
};
globalThis.reportError = reportError;
globalThis.structuredClone = function (v) { return v === undefined ? undefined : JSON.parse(JSON.stringify(v)); };
globalThis.CSS = {
  supports() { return false; },
  escape(s) { return String(s).replace(/[^a-zA-Z0-9_-]/g, (c) => '\\' + c); },
};
globalThis.getComputedStyle = function (el) { return makeStyle(el); };
globalThis.matchMedia = function (q) {
  return {
    matches: false, media: String(q), onchange: null,
    addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent() { return true; },
  };
};
globalThis.performance = {
  now() { return H.now(); },
  timeOrigin: Date.now(),
  mark() {}, measure() {}, clearMarks() {}, clearMeasures() {},
  getEntries() { return []; }, getEntriesByName() { return []; }, getEntriesByType() { return []; },
};
const Observer = (name) => ({ [name]: class { constructor(cb) { this._cb = cb; } observe() {} unobserve() {} disconnect() {} takeRecords() { return []; } } })[name];
globalThis.MutationObserver = Observer('MutationObserver');
globalThis.ResizeObserver = Observer('ResizeObserver');
globalThis.IntersectionObserver = Observer('IntersectionObserver');
globalThis.Image = function Image(w, h) {
  const i = document.createElement('img');
  if (w !== undefined) i.setAttribute('width', w);
  if (h !== undefined) i.setAttribute('height', h);
  return i;
};

// ---- base64 ----
const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const invalidChar = (m) => { const e = new Error(m); e.name = 'InvalidCharacterError'; return e; };
globalThis.btoa = function btoa(s) {
  s = String(s);
  let out = '';
  for (let i = 0; i < s.length; i += 3) {
    const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
    if (a > 255 || b > 255 || c > 255) throw invalidChar('the string contains characters outside of the Latin1 range');
    const n = (a << 16) | ((b || 0) << 8) | (c || 0);
    out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < s.length ? B64[(n >> 6) & 63] : '=') + (i + 2 < s.length ? B64[n & 63] : '=');
  }
  return out;
};
globalThis.atob = function atob(s) {
  s = String(s).replace(/[\t\n\f\r ]/g, '');
  if (s.length % 4 === 0) s = s.replace(/==?$/, '');
  if (s.length % 4 === 1 || /[^A-Za-z0-9+\/]/.test(s)) throw invalidChar('the string to be decoded is not correctly encoded');
  let out = '', bits = 0, acc = 0;
  for (const ch of s) {
    acc = (acc << 6) | B64.indexOf(ch);
    bits += 6;
    if (bits >= 8) { bits -= 8; out += String.fromCharCode((acc >> bits) & 255); acc &= (1 << bits) - 1; }
  }
  return out;
};

// ---- UTF-8 ----
class TextEncoder {
  get encoding() { return 'utf-8'; }
  encode(s) {
    s = s === undefined ? '' : String(s);
    const out = [];
    for (const ch of s) {
      let c = ch.codePointAt(0);
      if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd;
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return new Uint8Array(out);
  }
}
class TextDecoder {
  constructor() { this.encoding = 'utf-8'; }
  decode(buf) {
    if (buf === undefined || buf === null) return '';
    const b = buf instanceof ArrayBuffer ? new Uint8Array(buf) : new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    let out = '';
    for (let i = 0; i < b.length;) {
      const c = b[i];
      let cp, n;
      if (c < 0x80) { cp = c; n = 1; }
      else if (c >> 5 === 6) { cp = c & 31; n = 2; }
      else if (c >> 4 === 14) { cp = c & 15; n = 3; }
      else if (c >> 3 === 30) { cp = c & 7; n = 4; }
      else { out += '�'; i++; continue; }
      if (i + n > b.length) { out += '�'; break; }
      let ok = true;
      for (let j = 1; j < n; j++) {
        const x = b[i + j];
        if ((x & 0xc0) !== 0x80) { ok = false; break; }
        cp = (cp << 6) | (x & 63);
      }
      if (!ok || cp > 0x10ffff) { out += '�'; i++; continue; }
      out += String.fromCodePoint(cp);
      i += n;
    }
    return out;
  }
}
globalThis.TextEncoder = TextEncoder;
globalThis.TextDecoder = TextDecoder;
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_carry_the_level_for_warnings_and_errors() {
        assert_eq!(log_line("log", "hi"), "hi");
        assert_eq!(log_line("warn", "careful"), "warn: careful");
        assert_eq!(log_line("error", "boom"), "error: boom");
    }
}
