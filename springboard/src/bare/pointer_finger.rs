//! A mouse as a finger: the pure state machine behind the desktop pointer. It takes mouse events
//! with their time and returns the touch events a finger would make, plus when it next needs a
//! tick. Left button = finger, right button = a finger held for a long press, wheel = a short
//! eased drag at the pointer (so lists scroll with the shell's flick inertia).

use super::input::Event;

/// how long a right click holds the finger down, so the shell's long-press fires
pub const HOLD_MS: f64 = 600.0;
/// panel pixels one wheel notch drags the finger
const STEP_PX: f32 = 60.0;
/// the finger lifts after this long without wheel input
const WHEEL_IDLE_MS: f64 = 120.0;
/// wheel animation tick
const TICK_MS: f64 = 16.0;
/// share of the remaining distance the finger covers each tick
const EASE: f32 = 0.2;
/// farthest the finger travels from where it landed before it is lifted and put down again
const MAX_TRAVEL: f32 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mouse {
    /// the pointer moved (its new position is passed alongside)
    Moved,
    Left(bool),
    Right(bool),
    /// wheel notches: `v` positive = up, `h` positive = right
    Wheel { h: i32, v: i32 },
}

struct Scroll {
    anchor: (f32, f32),
    cur: (f32, f32),
    target: (f32, f32),
    last_input: f64,
    next_tick: f64,
}

impl Scroll {
    fn at(x: f32, y: f32, t: f64) -> Self {
        Scroll { anchor: (x, y), cur: (x, y), target: (x, y), last_input: t, next_tick: t + TICK_MS }
    }
}

#[derive(Default)]
pub struct PointerFinger {
    panel: (f32, f32),
    left: bool,
    /// right button: (finger x, y, time it may lift, released)
    hold: Option<(f32, f32, f64, bool)>,
    scroll: Option<Scroll>,
}

impl PointerFinger {
    pub fn new(w: f32, h: f32) -> Self {
        PointerFinger { panel: (w, h), ..Default::default() }
    }

    /// The left button is held (the cursor shrinks).
    pub fn is_down(&self) -> bool {
        self.left
    }

    fn clamp(&self, x: f32, y: f32) -> (f32, f32) {
        (x.clamp(0.0, (self.panel.0 - 1.0).max(0.0)), y.clamp(0.0, (self.panel.1 - 1.0).max(0.0)))
    }

    fn lift_scroll(&mut self, t: f64, out: &mut Vec<Event>) {
        if let Some(s) = self.scroll.take() {
            out.push(Event::Up { x: s.cur.0, y: s.cur.1, t });
        }
    }

    /// Feeds one mouse event at time `t` ms with the pointer at `at`; `shift` turns the wheel sideways.
    pub fn feed(&mut self, t: f64, at: (f32, f32), shift: bool, ev: Mouse, out: &mut Vec<Event>) {
        let (x, y) = at;
        match ev {
            Mouse::Moved if self.left => out.push(Event::Move { x, y, t }),
            Mouse::Moved => {}
            Mouse::Left(true) if !self.left && self.hold.is_none() => {
                self.lift_scroll(t, out);
                self.left = true;
                out.push(Event::Down { x, y, t });
            }
            Mouse::Left(false) if self.left => {
                self.left = false;
                out.push(Event::Up { x, y, t });
            }
            Mouse::Right(true) if !self.left && self.hold.is_none() => {
                self.lift_scroll(t, out);
                self.hold = Some((x, y, t + HOLD_MS, false));
                out.push(Event::Down { x, y, t });
            }
            Mouse::Right(false) => {
                if let Some(h) = &mut self.hold {
                    h.3 = true;
                }
                self.tick(t, out);
            }
            Mouse::Wheel { h, v } if !self.left && self.hold.is_none() => {
                // wheel up shows what is above: the finger drags down; shift or a side wheel drags sideways
                let (dx, dy) = if shift { (v as f32, 0.0) } else { (-(h as f32), v as f32) };
                let (dx, dy) = (dx * STEP_PX, dy * STEP_PX);
                let mut s = self.scroll.take().unwrap_or_else(|| {
                    out.push(Event::Down { x, y, t });
                    Scroll::at(x, y, t)
                });
                if (s.target.0 + dx - s.anchor.0).abs() > MAX_TRAVEL || (s.target.1 + dy - s.anchor.1).abs() > MAX_TRAVEL {
                    out.push(Event::Up { x: s.cur.0, y: s.cur.1, t });
                    out.push(Event::Down { x, y, t });
                    s = Scroll::at(x, y, t);
                }
                s.target = self.clamp(s.target.0 + dx, s.target.1 + dy);
                s.last_input = t;
                self.scroll = Some(s);
            }
            _ => {}
        }
    }

    /// Advances the wheel ease and the right-click hold up to `t`.
    pub fn tick(&mut self, t: f64, out: &mut Vec<Event>) {
        if let Some((x, y, lift, true)) = self.hold {
            if t >= lift {
                self.hold = None;
                out.push(Event::Up { x, y, t });
            }
        }
        let Some(mut s) = self.scroll.take() else { return };
        if t - s.last_input >= WHEEL_IDLE_MS {
            out.push(Event::Up { x: s.cur.0, y: s.cur.1, t });
            return;
        }
        while t >= s.next_tick {
            s.cur = (s.cur.0 + (s.target.0 - s.cur.0) * EASE, s.cur.1 + (s.target.1 - s.cur.1) * EASE);
            s.next_tick += TICK_MS;
            out.push(Event::Move { x: s.cur.0, y: s.cur.1, t });
        }
        self.scroll = Some(s);
    }

    /// When `tick` next has work (ms); None while idle.
    pub fn deadline(&self) -> Option<f64> {
        let hold = self.hold.filter(|h| h.3).map(|h| h.2);
        let scroll = self.scroll.as_ref().map(|s| s.next_tick.min(s.last_input + WHEEL_IDLE_MS));
        match (hold, scroll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pf() -> PointerFinger {
        PointerFinger::new(1000.0, 800.0)
    }

    fn feed(p: &mut PointerFinger, t: f64, ev: Mouse) -> Vec<Event> {
        let mut out = vec![];
        p.feed(t, (100.0, 400.0), false, ev, &mut out);
        out
    }

    #[test]
    fn left_button_is_a_finger_that_drags() {
        let mut p = pf();
        assert_eq!(feed(&mut p, 0.0, Mouse::Left(true)), vec![Event::Down { x: 100.0, y: 400.0, t: 0.0 }]);
        assert!(p.is_down());
        assert_eq!(feed(&mut p, 5.0, Mouse::Moved).len(), 1);
        assert_eq!(feed(&mut p, 9.0, Mouse::Left(false)), vec![Event::Up { x: 100.0, y: 400.0, t: 9.0 }]);
        assert!(feed(&mut p, 10.0, Mouse::Moved).is_empty(), "hovering is not a touch");
    }

    #[test]
    fn right_click_holds_the_finger_for_the_long_press() {
        let mut p = pf();
        assert_eq!(feed(&mut p, 0.0, Mouse::Right(true)).len(), 1);
        assert!(feed(&mut p, 100.0, Mouse::Right(false)).is_empty(), "an early release waits out the hold");
        assert_eq!(p.deadline(), Some(HOLD_MS));
        let mut out = vec![];
        p.tick(HOLD_MS, &mut out);
        assert_eq!(out, vec![Event::Up { x: 100.0, y: 400.0, t: HOLD_MS }]);
        assert_eq!(p.deadline(), None);
    }

    #[test]
    fn right_release_after_the_hold_lifts_at_once() {
        let mut p = pf();
        feed(&mut p, 0.0, Mouse::Right(true));
        assert_eq!(feed(&mut p, 900.0, Mouse::Right(false)).len(), 1);
    }

    #[test]
    fn wheel_is_an_eased_drag_that_lifts_when_idle() {
        let mut p = pf();
        let down = feed(&mut p, 0.0, Mouse::Wheel { h: 0, v: 1 });
        assert_eq!(down, vec![Event::Down { x: 100.0, y: 400.0, t: 0.0 }]);
        let mut out = vec![];
        p.tick(48.0, &mut out);
        let ys: Vec<f32> = out.iter().map(|e| if let Event::Move { y, .. } = e { *y } else { 0.0 }).collect();
        assert_eq!(ys.len(), 3);
        assert!(ys[0] > 400.0 && ys[1] > ys[0] && ys[2] < 460.0, "moves down toward the 60 px target: {ys:?}");
        out.clear();
        p.tick(121.0, &mut out);
        assert!(matches!(out.last(), Some(Event::Up { .. })));
        assert_eq!(p.deadline(), None);
    }

    #[test]
    fn notches_accumulate_and_shift_goes_sideways() {
        let mut p = pf();
        let mut out = vec![];
        p.feed(0.0, (100.0, 400.0), true, Mouse::Wheel { h: 0, v: 1 }, &mut out);
        p.feed(10.0, (100.0, 400.0), true, Mouse::Wheel { h: 0, v: 1 }, &mut out);
        out.clear();
        p.tick(100.0, &mut out);
        let Some(Event::Move { x, y, .. }) = out.last().copied() else { panic!("no move: {out:?}") };
        assert!(x > 100.0 && y == 400.0);
    }
}
