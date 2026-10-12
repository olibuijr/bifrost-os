//! Tile motion for the 2048 board, built on the animation model (`bifrost_app::controls::anim`).
//!
//! A slide that merges tiles makes each merged tile swell and settle, and the new tile grows in. Each tile's scale is a
//! `Sequence`: `Animation`s played one after another, each starting when the one before it ends. This is the simplest
//! form of the reference SDK's action sequence (`SKAction.sequence` of scale actions). The scale of a tile is drawn by
//! shrinking its cell about the centre, so the board needs no node tree.
//!
//! Not animated yet: tiles that only slide (the board is drawn cell by cell, so there is no tile to carry from one
//! cell to the next), fades (a fill has no alpha here), and any camera or zoom over the board. The model draws nothing
//! and reads no clock: the caller passes the time in seconds.

use bifrost_app::controls::anim::{Animation, Curve};

/// Seconds for a new tile to grow from `SPAWN_FROM` to full size.
const GROW_S: f32 = 0.12;
/// The scale a new tile starts at.
const SPAWN_FROM: f32 = 0.5;
/// Seconds for a merged tile to swell up to `MERGE_PEAK`, and the same again to settle back to full size.
const SWELL_S: f32 = 0.08;
/// The largest scale a merged tile reaches.
const MERGE_PEAK: f32 = 1.12;

/// Animations played one after another. Each step starts when the step before it ends, and the first starts at `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    steps: Vec<Animation>,
}

impl Sequence {
    /// A sequence from `from`, starting at clock time `start`. Each step is `(to, duration)` with a linear curve.
    pub fn new(start: f64, from: f32, steps: &[(f32, f32)]) -> Self {
        let mut at = start;
        let mut value = from;
        let mut out = Vec::with_capacity(steps.len());
        for &(to, duration) in steps {
            out.push(Animation::new(value, to, at, duration, Curve::Linear));
            at += duration.max(0.0) as f64;
            value = to;
        }
        Sequence { steps: out }
    }

    /// The value at clock time `now`: the step that is playing, or the last step's end value once the sequence is over.
    /// Before the sequence starts it is the starting value. An empty sequence reads 1.
    pub fn value(&self, now: f64) -> f32 {
        match self.steps.iter().rev().find(|a| now >= a.start) {
            Some(a) => a.value(now),
            None => self.steps.first().map_or(1.0, |a| a.from),
        }
    }

    /// Whether every step has finished by `now`. An empty sequence is finished.
    pub fn is_done(&self, now: f64) -> bool {
        self.steps.last().map_or(true, |a| a.is_done(now))
    }
}

/// The animating tiles of the 2048 board: one sequence per cell that is moving. The board clears them on every slide.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileMotion {
    cells: Vec<((usize, usize), Sequence)>,
}

impl TileMotion {
    /// Forgets every planned motion.
    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// A new tile at `cell` grows from `SPAWN_FROM` to full size, starting at `start`.
    pub fn grow(&mut self, cell: (usize, usize), start: f64) {
        self.cells.push((cell, Sequence::new(start, SPAWN_FROM, &[(1.0, GROW_S)])));
    }

    /// A merged tile at `cell` swells to `MERGE_PEAK` and settles back to full size, starting at `start`.
    pub fn swell(&mut self, cell: (usize, usize), start: f64) {
        self.cells.push((cell, Sequence::new(start, 1.0, &[(MERGE_PEAK, SWELL_S), (1.0, SWELL_S)])));
    }

    /// The scale of the tile in `cell` at `now`. A cell with no motion is drawn at full size, 1.
    pub fn scale(&self, cell: (usize, usize), now: f64) -> f32 {
        self.cells.iter().rev().find(|(c, _)| *c == cell).map_or(1.0, |(_, s)| s.value(now))
    }

    /// How many tiles are still moving at `now`.
    pub fn live(&self, now: f64) -> usize {
        self.cells.iter().filter(|(_, s)| !s.is_done(now)).count()
    }

    /// Whether any tile is still moving at `now`.
    pub fn is_live(&self, now: f64) -> bool {
        self.live(now) > 0
    }
}

/// A rectangle `(x, y, w, h)` shrunk or grown by `k` about its centre. `k` 1 leaves it as it is.
pub fn scaled(rect: (f32, f32, f32, f32), k: f32) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    let (nw, nh) = (w * k, h * k);
    (x + (w - nw) / 2.0, y + (h - nh) / 2.0, nw, nh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_new_tile_grows_from_half_size_in_twelve_hundredths_of_a_second() {
        let s = Sequence::new(10.0, SPAWN_FROM, &[(1.0, GROW_S)]);
        assert!(close(s.value(9.0), 0.5), "before the start the tile is at its first value");
        assert!(close(s.value(10.0), 0.5));
        // halfway through the 0.12 s grow: 0.5 + 0.5 * 0.5
        assert!(close(s.value(10.06), 0.75), "{}", s.value(10.06));
        assert!(close(s.value(10.12), 1.0));
        assert!(close(s.value(20.0), 1.0), "the end value holds after the sequence");
        assert!(!s.is_done(10.06));
        assert!(s.is_done(10.2));
    }

    #[test]
    fn a_merged_tile_swells_to_one_and_an_eighth_and_settles() {
        let s = Sequence::new(0.0, 1.0, &[(MERGE_PEAK, SWELL_S), (1.0, SWELL_S)]);
        // first half: 1.0 up to 1.12 over 0.08 s, so 1.06 at 0.04 s
        assert!(close(s.value(0.04), 1.06));
        // the peak is at 0.08 s, where the second half starts
        assert!(close(s.value(0.08), 1.12));
        // second half back to 1.0 at 0.16 s, so 1.06 at 0.12 s
        assert!(close(s.value(0.12), 1.06));
        assert!(close(s.value(0.16), 1.0));
        assert!(s.is_done(0.2));
    }

    #[test]
    fn an_empty_sequence_is_finished_and_reads_full_size() {
        let s = Sequence::new(0.0, 0.5, &[]);
        assert!(s.is_done(0.0));
        assert!(close(s.value(3.0), 1.0), "no steps: a tile without motion reads full size");
    }

    #[test]
    fn a_cell_without_motion_is_full_size_and_the_motion_ends_on_time() {
        let mut m = TileMotion::default();
        m.grow((1, 2), 5.0);
        assert!(close(m.scale((1, 2), 5.06), 0.75));
        assert!(close(m.scale((0, 0), 5.06), 1.0), "a cell with no motion stays at full size");
        assert!(m.is_live(5.06));
        assert_eq!(m.live(5.06), 1);
        assert!(!m.is_live(5.2));
        m.clear();
        assert!(!m.is_live(5.0));
    }

    #[test]
    fn a_scaled_rect_keeps_its_centre() {
        assert_eq!(scaled((100.0, 200.0, 50.0, 50.0), 0.5), (112.5, 212.5, 25.0, 25.0));
        assert_eq!(scaled((100.0, 200.0, 50.0, 50.0), 1.0), (100.0, 200.0, 50.0, 50.0));
    }
}
