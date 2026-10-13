//! The desktop pointer: a small translucent grey circle (bifrost-style, not an arrow) stamped into
//! the buffer that is presented and then taken off again, so the shell's own frame stays clean and
//! only the old and new cursor rectangles are damaged.

use super::Damage;

/// diameter in px, and the smaller one while the button is pressed
const SIZE: i32 = 12;
const SIZE_PRESSED: i32 = 10;
/// fill and ring, as (grey, alpha out of 255)
const FILL: (u32, u32) = (0x80, 110);
const RING: (u32, u32) = (0xd0, 170);
/// pointer hides after this long without motion
pub const HIDE_MS: f64 = 3000.0;

/// Where the cursor is and how big: the identity of what is on the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub x: i32,
    pub y: i32,
    pub size: i32,
}

impl Spot {
    /// The spot to show, or None when it must be hidden.
    pub fn want(pos: (f32, f32), pressed: bool, present: bool, screen_on: bool, now: f64, last_motion: f64) -> Option<Spot> {
        (present && screen_on && now - last_motion < HIDE_MS).then(|| Spot { x: pos.0 as i32, y: pos.1 as i32, size: if pressed { SIZE_PRESSED } else { SIZE } })
    }
}

/// The pointer's shown spot and what it covers. `scale` is Cursor Size (1x to 2x): the disc grows with it.
pub struct Cursor {
    shown: Option<Spot>,
    saved: Vec<u32>,
    scale: f32,
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor { shown: None, saved: Vec::new(), scale: 1.0 }
    }
}

fn mix(under: u32, grey: u32, alpha: u32) -> u32 {
    let ch = |s: u32| (((under >> s & 0xff) * (255 - alpha) + grey * alpha) / 255) << s;
    0xff00_0000 | ch(16) | ch(8) | ch(0)
}

impl Cursor {
    /// Cursor Size: the disc is the spot's size times `scale` (clamped to 1x to 2x). A new size forgets the shown
    /// spot, since its old pixels are still on the panel: the caller redraws the panel. True when the size changed.
    pub fn set_scale(&mut self, scale: f32) -> bool {
        let scale = if scale.is_finite() { scale.clamp(1.0, 2.0) } else { 1.0 };
        if scale == self.scale {
            return false;
        }
        self.scale = scale;
        self.shown = None;
        true
    }

    /// The rectangle a spot covers at the current size, with a pixel of margin.
    fn rect(&self, s: &Spot) -> (i32, i32, i32, i32) {
        let size = self.diameter(s);
        (s.x - size / 2 - 1, s.y - size / 2 - 1, size + 2, size + 2)
    }

    fn diameter(&self, s: &Spot) -> i32 {
        (s.size as f32 * self.scale).round() as i32
    }

    /// Moves to `want`, damaging the old and the new rectangle; true when anything changed.
    pub fn plan(&mut self, want: Option<Spot>, damage: &mut Damage) -> bool {
        if want == self.shown {
            return false;
        }
        for s in self.shown.iter().chain(want.iter()) {
            let (x, y, w, h) = self.rect(s);
            damage.add(x, y, w, h);
        }
        self.shown = want;
        true
    }

    /// The cursor is on the panel (and will need a present to come off).
    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    /// Forgets the cursor without damage (the panel was fully redrawn without it).
    pub fn forget(&mut self) {
        self.shown = None;
    }

    /// Draws the cursor into `buf` (stride `w`), remembering what it covers.
    pub fn stamp(&mut self, buf: &mut [u32], w: usize, h: usize) {
        let Some(s) = self.shown else { return };
        self.saved.clear();
        let (rx, ry, rw, rh) = self.rect(&s);
        let r = self.diameter(&s) as f32 / 2.0;
        for j in 0..rh {
            for i in 0..rw {
                let (px, py) = (rx + i, ry + j);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                let idx = py as usize * w + px as usize;
                self.saved.push(buf[idx]);
                let (dx, dy) = (px as f32 + 0.5 - s.x as f32, py as f32 + 0.5 - s.y as f32);
                let d = (dx * dx + dy * dy).sqrt();
                if d <= r - 1.0 {
                    buf[idx] = mix(buf[idx], FILL.0, FILL.1);
                } else if d <= r {
                    buf[idx] = mix(buf[idx], RING.0, RING.1);
                }
            }
        }
    }

    /// Puts back what `stamp` covered.
    pub fn restore(&mut self, buf: &mut [u32], w: usize, h: usize) {
        let Some(s) = self.shown else { return };
        let (rx, ry, rw, rh) = self.rect(&s);
        let mut it = self.saved.iter();
        for j in 0..rh {
            for i in 0..rw {
                let (px, py) = (rx + i, ry + j);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                if let Some(&v) = it.next() {
                    buf[py as usize * w + px as usize] = v;
                }
            }
        }
    }
}

/// Side of the cursor plane's sprite, in px (the display's cursor size on every panel that has one).
pub const SPRITE: i32 = 64;

/// The pointer as a plane sprite: the same grey dot as `stamp`, drawn centred in a SPRITE square with
/// premultiplied alpha (the plane's default blend), so the display moves it without a frame.
pub fn sprite() -> Vec<u32> {
    let mut v = vec![0u32; (SPRITE * SPRITE) as usize];
    let r = SIZE as f32 / 2.0;
    let c = SPRITE as f32 / 2.0;
    for y in 0..SPRITE {
        for x in 0..SPRITE {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let (grey, alpha) = if d <= r - 1.0 {
                FILL
            } else if d <= r {
                RING
            } else {
                continue;
            };
            let a = alpha;
            let g = grey * a / 255;
            v[(y * SPRITE + x) as usize] = a << 24 | g << 16 | g << 8 | g;
        }
    }
    v
}

/// What the display's cursor plane must do for the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneCmd {
    /// show the sprite with its top-left corner at (x, y)
    Show(i32, i32),
    Move(i32, i32),
    Hide,
}

/// The cursor plane's state: where the sprite was last sent. Decides only what to send.
#[derive(Default)]
pub struct Plane {
    at: Option<(i32, i32)>,
}

impl Plane {
    /// The command that brings the plane to `want` (the sprite's centre is the pointer), or None when it
    /// already shows it there.
    pub fn command(&mut self, want: Option<Spot>) -> Option<PlaneCmd> {
        let to = want.map(|s| (s.x - SPRITE / 2, s.y - SPRITE / 2));
        let cmd = match (self.at, to) {
            (None, None) => return None,
            (Some(_), None) => PlaneCmd::Hide,
            (None, Some((x, y))) => PlaneCmd::Show(x, y),
            (Some(a), Some(b)) if a == b => return None,
            (Some(_), Some((x, y))) => PlaneCmd::Move(x, y),
        };
        self.at = to;
        Some(cmd)
    }

    /// The plane shows the pointer (it needs a hide when the pointer times out).
    pub fn is_shown(&self) -> bool {
        self.at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_is_a_centred_dot_with_a_clear_corner() {
        let s = sprite();
        assert_eq!(s.len(), (SPRITE * SPRITE) as usize);
        let centre = s[(SPRITE / 2 * SPRITE + SPRITE / 2) as usize];
        assert_eq!(centre >> 24, FILL.1, "the fill is translucent, not opaque");
        assert_eq!(s[0], 0, "the corner is clear");
        assert!(s.iter().all(|&p| p >> 24 <= 255 && (p >> 16 & 0xff) <= (p >> 24)), "premultiplied: colour never above alpha");
    }

    #[test]
    fn plane_shows_moves_and_hides_only_on_change() {
        let mut p = Plane::default();
        let at = |x: f32, y: f32| Spot::want((x, y), false, true, true, 0.0, 0.0);
        assert_eq!(p.command(None), None);
        assert_eq!(p.command(at(100.0, 50.0)), Some(PlaneCmd::Show(100 - SPRITE / 2, 50 - SPRITE / 2)));
        assert!(p.is_shown());
        assert_eq!(p.command(at(100.0, 50.0)), None, "same spot: nothing to send");
        assert_eq!(p.command(at(101.0, 50.0)), Some(PlaneCmd::Move(101 - SPRITE / 2, 50 - SPRITE / 2)));
        assert_eq!(p.command(None), Some(PlaneCmd::Hide));
        assert!(!p.is_shown());
        assert_eq!(p.command(None), None, "already hidden");
    }

    #[test]
    fn hides_without_a_pointer_screen_or_after_three_seconds() {
        let at = (50.0, 40.0);
        assert!(Spot::want(at, false, true, true, 100.0, 0.0).is_some());
        assert!(Spot::want(at, false, false, true, 100.0, 0.0).is_none());
        assert!(Spot::want(at, false, true, false, 100.0, 0.0).is_none());
        assert!(Spot::want(at, false, true, true, 3000.0, 0.0).is_none());
        assert!(Spot::want(at, true, true, true, 0.0, 0.0).unwrap().size < Spot::want(at, false, true, true, 0.0, 0.0).unwrap().size);
    }

    #[test]
    fn plan_damages_only_the_old_and_new_rects() {
        let mut c = Cursor::default();
        let mut d = Damage::new(200, 200);
        assert!(c.plan(Spot::want((20.0, 20.0), false, true, true, 0.0, 0.0), &mut d));
        assert!(d.pixels() < 300);
        d.clear();
        assert!(!c.plan(Spot::want((20.0, 20.0), false, true, true, 1.0, 0.0), &mut d), "unchanged: no damage");
        assert!(d.is_empty());
        assert!(c.plan(Spot::want((150.0, 150.0), false, true, true, 2.0, 0.0), &mut d));
        assert!(d.pixels() < 600);
    }

    #[test]
    fn stamp_draws_then_restore_leaves_the_frame_clean() {
        let (w, h) = (40, 30);
        let mut buf = vec![0xff10_2030u32; w * h];
        let clean = buf.clone();
        let mut c = Cursor::default();
        c.plan(Spot::want((20.0, 15.0), false, true, true, 0.0, 0.0), &mut Damage::new(w, h));
        c.stamp(&mut buf, w, h);
        assert_ne!(buf[15 * w + 20], clean[0]);
        c.restore(&mut buf, w, h);
        assert_eq!(buf, clean);
    }

    #[test]
    fn cursor_size_grows_the_disc_and_a_new_size_forgets_the_spot() {
        let (w, h) = (60, 60);
        let at = Spot::want((30.0, 30.0), false, true, true, 0.0, 0.0).unwrap();
        let mut c = Cursor::default();
        let mut d = Damage::new(w, h);
        c.plan(Some(at), &mut d);
        let small = d.pixels();
        assert!(c.set_scale(2.0), "a new size is a change");
        assert!(!c.set_scale(2.0), "the same size is not");
        assert!(!c.is_shown(), "the old spot is forgotten until it is planned again");
        let mut d = Damage::new(w, h);
        c.plan(Some(at), &mut d);
        assert!(d.pixels() > small, "twice the disc damages more");
        let mut buf = vec![0xff10_2030u32; w * h];
        let clean = buf.clone();
        c.stamp(&mut buf, w, h);
        assert_ne!(buf[30 * w + 30 + 10], clean[0], "the grown disc covers 10 px from the centre");
        c.restore(&mut buf, w, h);
        assert_eq!(buf, clean);
        assert!(c.set_scale(f32::NAN), "a non-finite size falls back to 1x, a change from 2x");
        assert!(!c.set_scale(1.0), "1x is already the size");
    }

    #[test]
    fn a_cursor_at_the_edge_does_not_overrun_the_buffer() {
        let (w, h) = (20, 20);
        let mut buf = vec![0u32; w * h];
        let mut c = Cursor::default();
        c.plan(Spot::want((0.0, 0.0), false, true, true, 0.0, 0.0), &mut Damage::new(w, h));
        c.stamp(&mut buf, w, h);
        c.restore(&mut buf, w, h);
        assert!(buf.iter().all(|&p| p == 0));
    }
}
