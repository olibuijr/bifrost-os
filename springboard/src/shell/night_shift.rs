//! Night Shift: a warm colour shift on the final frame. Settings edits `NightShift`;
//! the render loop in `bin/shell.rs` reads the published copy (atomics) and tints the
//! frame it presents, so no app or layer needs to know about it.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

use super::list::{Group, Item, Spec, Ends};
use super::notify::schedule::Schedule;
use super::pages::{Action, PageId, Slider};
use super::prefs::Prefs;
use crate::gfx::Damage;

/// Default warmth, 0..1 (bifrost starts in the middle).
const DEFAULT_WARMTH: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NightShift {
    /// the daily From/To window is on
    pub scheduled: bool,
    /// "Manually Enable" switch: on regardless of the clock
    pub manual: bool,
    pub start: u16,
    pub end: u16,
    pub warmth: f32,
}

impl Default for NightShift {
    fn default() -> Self {
        NightShift { scheduled: false, manual: false, start: 22 * 60, end: 7 * 60, warmth: DEFAULT_WARMTH }
    }
}

/// What a row of the Night Shift page does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NightEdit {
    Scheduled,
    Manual,
    From,
    To,
}

impl NightShift {
    pub fn load(p: &Prefs) -> Self {
        let d = NightShift::default();
        let minute = |k: &str, v: u16| p.get_parsed::<u16>(k).map_or(v, |m| m % 1440);
        NightShift {
            scheduled: p.get_bool("night_scheduled", false),
            manual: p.get_bool("night_manual", false),
            start: minute("night_from", d.start),
            end: minute("night_to", d.end),
            warmth: p.get_parsed::<f32>("night_warmth").map_or(d.warmth, |v| v.clamp(0.0, 1.0)),
        }
    }

    pub fn save(&self, p: &mut Prefs) {
        let flag = |b: bool| if b { "1" } else { "0" };
        let _ = p.set_many(&[
            ("night_scheduled", flag(self.scheduled)),
            ("night_manual", flag(self.manual)),
            ("night_from", &self.start.to_string()),
            ("night_to", &self.end.to_string()),
            ("night_warmth", &format!("{:.2}", self.warmth)),
        ]);
    }

    fn window(&self) -> Schedule {
        Schedule { enabled: self.scheduled, start: self.start, end: self.end }
    }

    /// Whether the tint applies at local `minute` of the day.
    pub fn active(&self, minute: u16) -> bool {
        self.manual || self.window().active(minute)
    }

    /// Value shown on the Display & Brightness row.
    pub fn label(&self) -> &'static str {
        if self.manual {
            "On"
        } else if self.scheduled {
            "Scheduled"
        } else {
            "Off"
        }
    }

    /// Puts this state where the render loop reads it.
    pub fn publish(&self) {
        MANUAL.store(self.manual, Relaxed);
        SCHEDULED.store(self.scheduled, Relaxed);
        START.store(self.start as u32, Relaxed);
        END.store(self.end as u32, Relaxed);
        WARMTH.store((self.warmth * 1000.0).round() as u32, Relaxed);
    }

    /// Applies a time picked for the From or To row.
    pub fn set_edge(&mut self, from: bool, minute: u16) {
        if from {
            self.start = minute % 1440;
        } else {
            self.end = minute % 1440;
        }
    }

    pub fn spec(&self, clock_24h: bool) -> Spec {
        let clock = |m: u16| crate::status::clock_in((m / 60) as i32, (m % 60) as i32, clock_24h);
        Spec {
            id: PageId::NightShift,
            title: "Night Shift".into(),
            large: false,
            groups: vec![
                Group::new(vec![
                    Item::new("Scheduled").switch(self.scheduled, Action::Night(NightEdit::Scheduled)),
                    Item::new("From").value(clock(self.start)).action(Action::Night(NightEdit::From)),
                    Item::new("To").value(clock(self.end)).action(Action::Night(NightEdit::To)),
                ]),
                Group::new(vec![Item::new("Manually Enable").switch(self.manual, Action::Night(NightEdit::Manual))])
                    .footer("Shifts the colours of the display to the warmer end of the spectrum. A window can run past midnight."),
                Group::new(vec![Item::slider(Ends::Warmth, self.warmth, Action::Slider(Slider::Warmth))]).header("COLOR TEMPERATURE"),
            ],
        }
    }
}

static MANUAL: AtomicBool = AtomicBool::new(false);
static SCHEDULED: AtomicBool = AtomicBool::new(false);
static START: AtomicU32 = AtomicU32::new(22 * 60);
static END: AtomicU32 = AtomicU32::new(7 * 60);
static WARMTH: AtomicU32 = AtomicU32::new(500);

/// The published state.
pub fn published() -> NightShift {
    NightShift {
        scheduled: SCHEDULED.load(Relaxed),
        manual: MANUAL.load(Relaxed),
        start: START.load(Relaxed) as u16,
        end: END.load(Relaxed) as u16,
        warmth: WARMTH.load(Relaxed) as f32 / 1000.0,
    }
}

/// Per-channel multipliers (out of 256) for red, green, blue; `None` when the screen is untouched.
pub fn multipliers(minute: u16) -> Option<[u32; 3]> {
    let s = published();
    if !s.active(minute) || s.warmth <= 0.0 {
        return None;
    }
    let w = s.warmth;
    Some([256, (256.0 * (1.0 - 0.28 * w)) as u32, (256.0 * (1.0 - 0.62 * w)) as u32])
}

/// One pixel times the multipliers. Frames are `gfx::pack` order (red in the low byte, then green, blue, alpha), the
/// same bytes the GPU presenter uploads; the old XRGB reading cut red and kept blue, a cold tint instead of a warm one.
fn shade(s: u32, m: [u32; 3]) -> u32 {
    (s & 0xFF00_0000) | (((s >> 16 & 0xFF) * m[2] >> 8) << 16) | (((s >> 8 & 0xFF) * m[1] >> 8) << 8) | ((s & 0xFF) * m[0] >> 8)
}

/// Tints `src` into `dst` (same size, `w` pixels wide) over the damaged areas.
pub fn tint(src: &[u32], dst: &mut [u32], w: usize, damage: &Damage, m: [u32; 3]) {
    for a in damage.areas() {
        for y in a.y0..a.y1 {
            let row = y * w;
            for (d, &s) in dst[row + a.x0..row + a.x1].iter_mut().zip(&src[row + a.x0..row + a.x1]) {
                *d = shade(s, m);
            }
        }
    }
}

/// Tints the damaged areas of a buffer that is rebuilt there every frame (the panel copy).
pub fn tint_in_place(buf: &mut [u32], w: usize, damage: &Damage, m: [u32; 3]) {
    for a in damage.areas() {
        for y in a.y0..a.y1 {
            for d in &mut buf[y * w + a.x0..y * w + a.x1] {
                *d = shade(*d, m);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_and_manual_decide_when_it_is_active() {
        let n = NightShift { scheduled: true, ..Default::default() };
        assert!(n.active(23 * 60) && n.active(60) && !n.active(12 * 60));
        assert!(NightShift { manual: true, ..Default::default() }.active(12 * 60));
        assert!(!NightShift::default().active(23 * 60));
    }

    #[test]
    fn tint_keeps_red_and_cuts_blue() {
        let mut d = Damage::new(3, 1);
        d.all();
        let mut out = [0u32; 3];
        // pack order: 0xAABBGGRR, so 0xFF00_00FF is pure red and 0xFFFF_0000 pure blue
        tint(&[0xFFFF_FFFF, 0xFF00_00FF, 0xFFFF_0000], &mut out, 3, &d, [256, 200, 100]);
        assert_eq!(out[0], 0xFF63_C7FF);
        assert_eq!(out[1], 0xFF00_00FF, "red is kept");
        assert_eq!(out[2], 0xFF63_0000, "blue is cut");
        assert_eq!(out[1], crate::gfx::pack(255, 0, 0, 255));
    }

    #[test]
    fn prefs_round_trip() {
        let mut p = Prefs::memory();
        let n = NightShift { scheduled: true, manual: false, start: 1300, end: 400, warmth: 0.75 };
        n.save(&mut p);
        assert_eq!(NightShift::load(&p), n);
    }
}
