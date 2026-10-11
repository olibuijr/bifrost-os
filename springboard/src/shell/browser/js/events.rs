//! The event system: `Event`, `CustomEvent`, `MouseEvent`, `KeyboardEvent`,
//! `EventTarget` and propagation (capture, target, bubble, `stopPropagation`,
//! `stopImmediatePropagation`, `preventDefault`, `once`, listener removal).
//! `__dispatch` is the entry point the Rust side calls for host events.

pub(crate) const JS: &str = r####"
const LISTENERS = new WeakMap();
const reportError = (e) => { try { console.error('Uncaught ' + String(e)); } catch (_) {} };

class Event {
  constructor(type, init) {
    init = init || {};
    this.type = String(type);
    this.bubbles = !!init.bubbles;
    this.cancelable = !!init.cancelable;
    this.composed = !!init.composed;
    this.defaultPrevented = false;
    this.target = null;
    this.currentTarget = null;
    this.eventPhase = 0;
    this.isTrusted = false;
    this.timeStamp = H.now();
    Object.defineProperty(this, '_stop', { value: false, writable: true });
    Object.defineProperty(this, '_stopNow', { value: false, writable: true });
    Object.defineProperty(this, '_path', { value: null, writable: true });
  }
  preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
  stopPropagation() { this._stop = true; }
  stopImmediatePropagation() { this._stop = true; this._stopNow = true; }
  composedPath() { return this._path ? this._path.slice() : []; }
  get cancelBubble() { return this._stop; }
  set cancelBubble(v) { if (v) this._stop = true; }
  get srcElement() { return this.target; }
  get returnValue() { return !this.defaultPrevented; }
  set returnValue(v) { if (!v) this.preventDefault(); }
}
Event.NONE = 0; Event.CAPTURING_PHASE = 1; Event.AT_TARGET = 2; Event.BUBBLING_PHASE = 3;

class CustomEvent extends Event {
  constructor(type, init) {
    super(type, init);
    this.detail = init && init.detail !== undefined ? init.detail : null;
  }
}
class NativeEvent extends Event {
  constructor(type, init) {
    super(type, init);
    this.detail = (init && init.detail) || 0;
    this.view = (init && init.view) || null;
  }
}
class MouseEvent extends NativeEvent {
  constructor(type, init) {
    super(type, init);
    init = init || {};
    this.clientX = init.clientX || 0;
    this.clientY = init.clientY || 0;
    this.screenX = init.screenX || this.clientX;
    this.screenY = init.screenY || this.clientY;
    this.pageX = init.pageX !== undefined ? init.pageX : this.clientX;
    this.pageY = init.pageY !== undefined ? init.pageY : this.clientY;
    this.offsetX = init.offsetX || 0;
    this.offsetY = init.offsetY || 0;
    this.button = init.button || 0;
    this.buttons = init.buttons || 0;
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
    this.altKey = !!init.altKey;
    this.metaKey = !!init.metaKey;
    this.relatedTarget = init.relatedTarget || null;
  }
}
class KeyboardEvent extends NativeEvent {
  constructor(type, init) {
    super(type, init);
    init = init || {};
    this.key = init.key === undefined ? '' : String(init.key);
    this.code = init.code === undefined ? '' : String(init.code);
    this.repeat = !!init.repeat;
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
    this.altKey = !!init.altKey;
    this.metaKey = !!init.metaKey;
  }
}
class FocusEvent extends NativeEvent {
  constructor(type, init) { super(type, init); this.relatedTarget = (init && init.relatedTarget) || null; }
}
class InputEvent extends NativeEvent {
  constructor(type, init) {
    super(type, init);
    this.data = init && init.data !== undefined ? init.data : null;
    this.inputType = (init && init.inputType) || '';
  }
}

class EventTarget {
  addEventListener(type, cb, opts) {
    if (cb === null || cb === undefined) return;
    const o = typeof opts === 'boolean' ? { capture: opts } : opts || {};
    let m = LISTENERS.get(this);
    if (!m) { m = new Map(); LISTENERS.set(this, m); }
    type = String(type);
    let arr = m.get(type);
    if (!arr) { arr = []; m.set(type, arr); }
    const capture = !!o.capture;
    if (arr.some((l) => l.cb === cb && l.capture === capture)) return;
    const entry = { cb, capture, once: !!o.once, removed: false };
    arr.push(entry);
    if (o.signal) o.signal.addEventListener('abort', () => this.removeEventListener(type, cb, capture));
  }
  removeEventListener(type, cb, opts) {
    const capture = typeof opts === 'boolean' ? opts : !!(opts && opts.capture);
    const m = LISTENERS.get(this);
    const arr = m && m.get(String(type));
    if (!arr) return;
    for (let i = 0; i < arr.length; i++) {
      if (arr[i].cb === cb && arr[i].capture === capture) { arr[i].removed = true; arr.splice(i, 1); break; }
    }
  }
  dispatchEvent(ev) {
    if (!(ev instanceof Event)) throw new TypeError('argument is not an Event');
    return dispatch(this, ev);
  }
}

// The next target up the propagation path: parent node, then window.
function eventParent(t) {
  if (t instanceof Node) return t._id === 0 ? globalThis : wrap(H.parent(t._id));
  return null;
}

function invoke(target, ev, capturePass) {
  ev.currentTarget = target;
  if (!capturePass) {
    const h = target['on' + ev.type];
    if (typeof h === 'function') {
      try { h.call(target, ev); } catch (e) { reportError(e); }
      if (ev._stopNow) return;
    }
  }
  const m = LISTENERS.get(target);
  const arr = m && m.get(ev.type);
  if (!arr) return;
  for (const l of arr.slice()) {
    if (l.removed || l.capture !== capturePass) continue;
    if (l.once) target.removeEventListener(ev.type, l.cb, l.capture);
    try {
      if (typeof l.cb === 'function') l.cb.call(target, ev);
      else if (l.cb && typeof l.cb.handleEvent === 'function') l.cb.handleEvent(ev);
    } catch (e) { reportError(e); }
    if (ev._stopNow) return;
  }
}

function dispatch(target, ev) {
  const path = [];
  for (let t = target; t; t = eventParent(t)) path.push(t);
  ev.target = target;
  ev._path = path;
  ev._stop = false;
  ev._stopNow = false;
  ev.eventPhase = 1;
  for (let i = path.length - 1; i > 0 && !ev._stop; i--) invoke(path[i], ev, true);
  ev.eventPhase = 2;
  if (!ev._stop) invoke(path[0], ev, true);
  if (!ev._stop) invoke(path[0], ev, false);
  ev.eventPhase = 3;
  if (ev.bubbles) for (let i = 1; i < path.length && !ev._stop; i++) invoke(path[i], ev, false);
  ev.eventPhase = 0;
  ev.currentTarget = null;
  return !ev.defaultPrevented;
}

const MOUSE_TYPES = ['click', 'dblclick', 'mousedown', 'mouseup', 'mousemove', 'mouseover', 'mouseout', 'mouseenter', 'mouseleave', 'contextmenu', 'auxclick'];
// Called by the host: kind, bubbles, cancelable, clientX, clientY, detail, key, button.
globalThis.__dispatch = function (id, kind, bubbles, cancelable, cx, cy, detail, key, button) {
  const target = wrap(id);
  const init = { bubbles, cancelable, clientX: cx, clientY: cy, detail, button, key };
  let ev;
  if (MOUSE_TYPES.includes(kind)) ev = new MouseEvent(kind, init);
  else if (kind === 'keydown' || kind === 'keyup' || kind === 'keypress') ev = new KeyboardEvent(kind, init);
  else if (kind === 'focus' || kind === 'blur' || kind === 'focusin' || kind === 'focusout') ev = new FocusEvent(kind, init);
  else if (detail !== 0) ev = new CustomEvent(kind, init);
  else ev = new Event(kind, init);
  ev.isTrusted = true;
  return dispatch(target, ev);
};
Object.defineProperty(globalThis, '__dispatch', { enumerable: false });

globalThis.Event = Event;
globalThis.CustomEvent = CustomEvent;
globalThis.NativeEvent = NativeEvent;
globalThis.MouseEvent = MouseEvent;
globalThis.KeyboardEvent = KeyboardEvent;
globalThis.FocusEvent = FocusEvent;
globalThis.InputEvent = InputEvent;
globalThis.EventTarget = EventTarget;
"####;


