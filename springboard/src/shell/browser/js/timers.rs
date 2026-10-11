//! Timers: a queue of due times and ids in Rust, callbacks held by the
//! prelude. `JsPage::run_timers(now_ms)` pops what is due and calls the
//! prelude's `__fire(id, now)`. `setTimeout`, `setInterval`,
//! `requestAnimationFrame`, `requestIdleCallback` and `queueMicrotask` live here.

use super::{arg_bool, arg_num, host, register as register_table, NativeFn};
use boa_engine::{Context, JsResult, JsValue};

#[derive(Clone, Debug, PartialEq)]
struct Timer {
    id: u32,
    due: f64,
    interval: Option<f64>,
    seq: u64,
}

/// The pending timers and the page clock.
#[derive(Debug, Default)]
pub(crate) struct Timers {
    pub now: f64,
    next_id: u32,
    pub seq: u64,
    list: Vec<Timer>,
}

impl Timers {
    /// Schedules a timer `delay` ms from `now`; returns its id (never 0).
    pub fn add(&mut self, delay: f64, repeat: bool) -> u32 {
        let delay = if delay.is_finite() && delay > 0.0 { delay } else { 0.0 };
        self.next_id += 1;
        self.seq += 1;
        let t = Timer {
            id: self.next_id,
            due: self.now + delay,
            interval: repeat.then(|| delay.max(1.0)),
            seq: self.seq,
        };
        let id = t.id;
        self.list.push(t);
        id
    }

    pub fn clear(&mut self, id: u32) {
        self.list.retain(|t| t.id != id);
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Removes and returns the earliest timer due at `now` that was scheduled
    /// before sequence number `start_seq`. An interval is rescheduled with a
    /// fresh sequence number, so it fires once per run.
    pub fn pop_due(&mut self, now: f64, start_seq: u64) -> Option<u32> {
        let mut best: Option<usize> = None;
        for (i, t) in self.list.iter().enumerate() {
            if t.due > now || t.seq > start_seq {
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => (t.due, t.seq) < (self.list[b].due, self.list[b].seq),
            };
            if better {
                best = Some(i);
            }
        }
        let i = best?;
        let id = self.list[i].id;
        match self.list[i].interval {
            Some(iv) => {
                self.seq += 1;
                let t = &mut self.list[i];
                t.due += iv;
                t.seq = self.seq;
            }
            None => {
                self.list.remove(i);
            }
        }
        Some(id)
    }
}

fn timer_add(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let delay = arg_num(a, 0, c)?;
    let repeat = arg_bool(a, 1);
    let h = host(c);
    let id = h.timers.borrow_mut().add(delay, repeat);
    Ok(JsValue::from(f64::from(id)))
}

fn timer_clear(_: &JsValue, a: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    let id = arg_num(a, 0, c)?;
    if id.is_finite() && id >= 0.0 {
        host(c).timers.borrow_mut().clear(id as u32);
    }
    Ok(JsValue::undefined())
}

fn now(_: &JsValue, _: &[JsValue], c: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(host(c).timers.borrow().now))
}

const TABLE: &[(&str, NativeFn)] = &[("timer_add", timer_add), ("timer_clear", timer_clear), ("now", now)];

pub(crate) fn register(ctx: &mut Context) {
    register_table(ctx, TABLE);
}

pub(crate) const JS: &str = r####"
const TIMER_CBS = new Map();
function addTimer(fn, delay, repeat, args, raf) {
  if (typeof fn !== 'function') throw new TypeError('timer callback is not a function');
  const id = H.timer_add(Number(delay) || 0, repeat);
  TIMER_CBS.set(id, { fn, args, repeat, raf });
  return id;
}
function clearTimer(id) {
  id = Number(id);
  if (!(id > 0)) return;
  H.timer_clear(id);
  TIMER_CBS.delete(id);
}
globalThis.setTimeout = function setTimeout(fn, delay, ...args) { return addTimer(fn, delay, false, args, false); };
globalThis.setInterval = function setInterval(fn, delay, ...args) { return addTimer(fn, delay, true, args, false); };
globalThis.clearTimeout = clearTimer;
globalThis.clearInterval = clearTimer;
globalThis.requestAnimationFrame = function requestAnimationFrame(fn) { return addTimer(fn, 16, false, [], true); };
globalThis.cancelAnimationFrame = clearTimer;
globalThis.requestIdleCallback = function requestIdleCallback(fn) {
  return addTimer((t) => fn({ didTimeout: false, timeRemaining() { return 10; } }), 1, false, [], true);
};
globalThis.cancelIdleCallback = clearTimer;
globalThis.queueMicrotask = function queueMicrotask(fn) {
  if (typeof fn !== 'function') throw new TypeError('queueMicrotask callback is not a function');
  Promise.resolve().then(() => { try { fn(); } catch (e) { reportError(e); } });
};
globalThis.__fire = function (id, now) {
  const c = TIMER_CBS.get(id);
  if (!c) return;
  if (!c.repeat) TIMER_CBS.delete(id);
  try { c.fn.apply(globalThis, c.raf ? [now] : c.args); } catch (e) { reportError(e); }
};
Object.defineProperty(globalThis, '__fire', { enumerable: false });
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_in_due_order() {
        let mut t = Timers::default();
        let a = t.add(50.0, false);
        let b = t.add(10.0, false);
        let start = t.seq;
        assert_eq!(t.pop_due(5.0, start), None);
        assert_eq!(t.pop_due(100.0, start), Some(b));
        assert_eq!(t.pop_due(100.0, start), Some(a));
        assert_eq!(t.pop_due(100.0, start), None);
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn equal_due_times_keep_insertion_order() {
        let mut t = Timers::default();
        let a = t.add(10.0, false);
        let b = t.add(10.0, false);
        let start = t.seq;
        assert_eq!(t.pop_due(10.0, start), Some(a));
        assert_eq!(t.pop_due(10.0, start), Some(b));
    }

    #[test]
    fn interval_fires_once_per_run_and_reschedules() {
        let mut t = Timers::default();
        let a = t.add(10.0, true);
        let start = t.seq;
        assert_eq!(t.pop_due(35.0, start), Some(a));
        assert_eq!(t.pop_due(35.0, start), None);
        let start = t.seq;
        assert_eq!(t.pop_due(35.0, start), Some(a));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn timers_added_during_a_run_wait_for_the_next() {
        let mut t = Timers::default();
        let start = t.seq;
        t.now = 100.0;
        let late = t.add(0.0, false);
        assert_eq!(t.pop_due(100.0, start), None);
        let start = t.seq;
        assert_eq!(t.pop_due(100.0, start), Some(late));
    }

    #[test]
    fn clear_removes_and_bad_delays_mean_zero() {
        let mut t = Timers::default();
        let a = t.add(f64::NAN, false);
        let b = t.add(-5.0, false);
        t.clear(a);
        let start = t.seq;
        assert_eq!(t.pop_due(0.0, start), Some(b));
        assert_eq!(t.pop_due(0.0, start), None);
    }
}
