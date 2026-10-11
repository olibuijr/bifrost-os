//! `localStorage` and `sessionStorage`: in-memory `Storage` objects with the
//! method API and property access (`localStorage.key = "v"`).

pub(crate) const JS: &str = r####"
class Storage {
  constructor() { Object.defineProperty(this, '_m', { value: new Map(), configurable: true }); }
  get length() { return this._m.size; }
  key(i) { const k = Array.from(this._m.keys()); return i >= 0 && i < k.length ? k[i] : null; }
  getItem(k) { k = String(k); return this._m.has(k) ? this._m.get(k) : null; }
  setItem(k, v) { this._m.set(String(k), String(v)); }
  removeItem(k) { this._m.delete(String(k)); }
  clear() { this._m.clear(); }
}
function makeStorage() {
  return new Proxy(new Storage(), {
    get(t, p) {
      if (typeof p === 'symbol' || p in t) return Reflect.get(t, p, t);
      return t._m.has(p) ? t._m.get(p) : undefined;
    },
    set(t, p, v) { t.setItem(p, v); return true; },
    has(t, p) { return p in t || (typeof p === 'string' && t._m.has(p)); },
    deleteProperty(t, p) { t._m.delete(String(p)); return true; },
    ownKeys(t) { return Array.from(t._m.keys()); },
    getOwnPropertyDescriptor(t, p) {
      if (typeof p === 'string' && t._m.has(p)) return { value: t._m.get(p), writable: true, enumerable: true, configurable: true };
      return undefined;
    },
  });
}
globalThis.Storage = Storage;
globalThis.localStorage = makeStorage();
globalThis.sessionStorage = makeStorage();
"####;
