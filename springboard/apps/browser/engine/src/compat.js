(function () {
  'use strict';
  var w = window;
  function def(name, value) {
    try { Object.defineProperty(w, name, { value: value, writable: true, configurable: true, enumerable: false }); } catch (e) {}
  }

  // requestIdleCallback / cancelIdleCallback
  if (typeof w.requestIdleCallback !== 'function') {
    def('requestIdleCallback', function (cb, opts) {
      var start = Date.now();
      var timeout = opts && opts.timeout > 0 ? Math.min(opts.timeout, 50) : 1;
      return setTimeout(function () {
        cb({ didTimeout: false, timeRemaining: function () { return Math.max(0, 50 - (Date.now() - start)); } });
      }, timeout);
    });
    def('cancelIdleCallback', function (id) { clearTimeout(id); });
  }

  // IntersectionObserver: viewport (or root element) intersection, re-evaluated on scroll, resize and a slow tick.
  if (typeof w.IntersectionObserver === 'undefined') {
    var IO = function IntersectionObserver(cb, opts) {
      if (typeof cb !== 'function') throw new TypeError('IntersectionObserver callback must be a function');
      opts = opts || {};
      this.root = opts.root || null;
      this.rootMargin = opts.rootMargin || '0px';
      var t = opts.threshold;
      this.thresholds = Array.isArray(t) ? t.slice().sort() : [t == null ? 0 : t];
      this._cb = cb; this._t = new Map(); this._pending = false;
      var self = this;
      this._check = function () { self._run(); };
      w.addEventListener('scroll', this._check, { passive: true, capture: true });
      w.addEventListener('resize', this._check, { passive: true });
      this._timer = setInterval(this._check, 250);
    };
    var margin = function (s) {
      var p = String(s).trim().split(/\s+/).map(function (v) { return parseFloat(v) || 0; });
      if (p.length === 1) p = [p[0], p[0], p[0], p[0]];
      else if (p.length === 2) p = [p[0], p[1], p[0], p[1]];
      else if (p.length === 3) p = [p[0], p[1], p[2], p[1]];
      return p;
    };
    IO.prototype._rootRect = function () {
      var m = margin(this.rootMargin), r;
      if (this.root && this.root.getBoundingClientRect) r = this.root.getBoundingClientRect();
      else r = { left: 0, top: 0, right: w.innerWidth, bottom: w.innerHeight };
      return { left: r.left - m[3], top: r.top - m[0], right: r.right + m[1], bottom: r.bottom + m[2] };
    };
    IO.prototype._run = function () {
      var root = this._rootRect(), out = [], self = this;
      this._t.forEach(function (prev, el) {
        var b = el.getBoundingClientRect();
        var l = Math.max(b.left, root.left), tp = Math.max(b.top, root.top);
        var r = Math.min(b.right, root.right), bt = Math.min(b.bottom, root.bottom);
        var iw = r - l, ih = bt - tp;
        var touching = iw >= 0 && ih >= 0;
        var area = b.width * b.height;
        var ratio = touching ? (area > 0 ? (iw * ih) / area : 1) : 0;
        var isI = touching && (ratio > 0 || area === 0);
        var idx = 0;
        if (isI) for (var i = 0; i < self.thresholds.length; i++) if (ratio >= self.thresholds[i]) idx = i + 1;
        if (prev.seen && prev.idx === idx && prev.isI === isI) return;
        prev.idx = idx; prev.isI = isI; prev.seen = true;
        out.push({
          time: performance.now(), target: el, isIntersecting: isI, intersectionRatio: ratio,
          boundingClientRect: b, rootBounds: root,
          intersectionRect: isI ? { left: l, top: tp, right: r, bottom: bt, width: iw, height: ih, x: l, y: tp } : { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0 }
        });
      });
      if (out.length) { try { this._cb.call(this, out, this); } catch (e) { setTimeout(function () { throw e; }); } }
    };
    IO.prototype.observe = function (el) {
      if (!el || this._t.has(el)) return;
      this._t.set(el, { idx: 0, isI: false, seen: false });
      var self = this;
      if (!this._pending) { this._pending = true; setTimeout(function () { self._pending = false; self._run(); }, 0); }
    };
    IO.prototype.unobserve = function (el) { this._t.delete(el); };
    IO.prototype.disconnect = function () {
      this._t.clear();
      w.removeEventListener('scroll', this._check, true);
      w.removeEventListener('resize', this._check);
      clearInterval(this._timer);
    };
    IO.prototype.takeRecords = function () { return []; };
    def('IntersectionObserver', IO);
  }

  // ResizeObserver: polls observed element sizes on animation frames.
  if (typeof w.ResizeObserver === 'undefined') {
    var RO = function ResizeObserver(cb) {
      if (typeof cb !== 'function') throw new TypeError('ResizeObserver callback must be a function');
      this._cb = cb; this._t = new Map(); this._raf = 0;
    };
    RO.prototype._tick = function () {
      var self = this; this._raf = 0;
      var out = [];
      this._t.forEach(function (last, el) {
        var r = el.getBoundingClientRect();
        if (last.w !== r.width || last.h !== r.height) {
          last.w = r.width; last.h = r.height;
          var box = { inlineSize: r.width, blockSize: r.height };
          out.push({ target: el, contentRect: { x: 0, y: 0, left: 0, top: 0, width: r.width, height: r.height, right: r.width, bottom: r.height }, contentBoxSize: [box], borderBoxSize: [box], devicePixelContentBoxSize: [box] });
        }
      });
      if (out.length) { try { this._cb.call(this, out, this); } catch (e) { setTimeout(function () { throw e; }); } }
      if (this._t.size) this._raf = w.requestAnimationFrame(function () { self._tick(); });
    };
    RO.prototype.observe = function (el) {
      if (!el || this._t.has(el)) return;
      this._t.set(el, { w: -1, h: -1 });
      var self = this;
      if (!this._raf) this._raf = w.requestAnimationFrame(function () { self._tick(); });
    };
    RO.prototype.unobserve = function (el) { this._t.delete(el); };
    RO.prototype.disconnect = function () { this._t.clear(); if (this._raf) w.cancelAnimationFrame(this._raf); this._raf = 0; };
    def('ResizeObserver', RO);
  }

  // Small guarded gap fillers (SpiderMonkey and Servo 0.7.0 already ship structuredClone, AbortSignal.timeout/any,
  // Array at/findLast, CSS.supports and scrollIntoView; these cover only what the audit found absent).
  try {
    if (typeof w.matchMedia === 'function') {
      var mql = w.matchMedia('all');
      if (mql && typeof mql.addEventListener !== 'function' && typeof mql.addListener === 'function') {
        var P = Object.getPrototypeOf(mql);
        P.addEventListener = function (t, cb) { if (t === 'change') this.addListener(cb); };
        P.removeEventListener = function (t, cb) { if (t === 'change') this.removeListener(cb); };
      }
    }
  } catch (e) {}
  if (typeof w.AbortSignal === 'function' && typeof w.AbortSignal.prototype.throwIfAborted !== 'function') {
    try {
      Object.defineProperty(w.AbortSignal.prototype, 'throwIfAborted', { configurable: true, writable: true, value: function () {
        if (this.aborted) throw this.reason !== undefined ? this.reason : new DOMException('This operation was aborted', 'AbortError');
      } });
    } catch (e) {}
  }
  if (typeof w.Array.prototype.group === 'undefined' && typeof Object.groupBy !== 'function') {
    try { Object.defineProperty(Object, 'groupBy', { configurable: true, writable: true, value: function (items, fn) {
      var o = Object.create(null), i = 0;
      for (var x of items) { var k = fn(x, i++); (o[k] || (o[k] = [])).push(x); }
      return o;
    } }); } catch (e) {}
  }
})();
