//! A navigation stack with the the native toolkit push/pop transition: the incoming page
//! slides in from the right while the one beneath drifts a third of the
//! way left under a light dimming; a pop plays it backwards.

use bifrost_ui::transition::{Ease, Subtype, Transition as Effect};

/// How long a push or pop takes, in ms.
pub const MS: f64 = 350.0;
/// How far (as a fraction of the width) the page underneath drifts.
const UNDER_SHIFT: f32 = 0.3;
/// Opacity of the shade over the page underneath at the end of a push.
const UNDER_DIM: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Push,
    Pop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Transition {
    dir: Dir,
    start: f64,
}

/// Where the two pages of a transition sit at progress `p` (0..=1):
/// x offsets of the page underneath and of the page on top, in px, and
/// the shade alpha (0..=255) over the one underneath.
pub fn placement(dir: Dir, p: f32, w: f32) -> (f32, f32, u8) {
    // the shared push transition: a pop is the same effect played backwards
    let push = Effect::push(Subtype::FromRight).parallax(UNDER_SHIFT, UNDER_DIM).ease(Ease::OutQuartic);
    let tr = if dir == Dir::Pop { push.reversed() } else { push };
    let pl = tr.placement(p, w, 0.0);
    (pl.under_x.round(), pl.over_x.round(), pl.shade_u8())
}

pub struct Nav<P> {
    pages: Vec<P>,
    /// the page a pop removed, drawn on top until its slide-out ends
    leaving: Option<P>,
    transition: Option<Transition>,
}

impl<P> Nav<P> {
    pub fn new(root: P) -> Self {
        Nav { pages: vec![root], leaving: None, transition: None }
    }

    pub fn top(&self) -> &P {
        self.pages.last().unwrap()
    }

    pub fn top_mut(&mut self) -> &mut P {
        self.pages.last_mut().unwrap()
    }

    /// The page beneath the top (the one a Back goes to), if any.
    pub fn under(&self) -> Option<&P> {
        self.pages.len().checked_sub(2).map(|i| &self.pages[i])
    }

    pub fn depth(&self) -> usize {
        self.pages.len()
    }

    pub fn pages(&self) -> &[P] {
        &self.pages
    }

    pub fn pages_mut(&mut self) -> &mut [P] {
        &mut self.pages
    }

    /// Pushes `page`, sliding it in from `t`.
    pub fn push(&mut self, page: P, t: f64) {
        self.finish();
        self.pages.push(page);
        self.transition = Some(Transition { dir: Dir::Push, start: t });
    }

    /// Pops the top page unless it is the root; true when something left.
    pub fn pop(&mut self, t: f64) -> bool {
        self.finish();
        if self.pages.len() < 2 {
            return false;
        }
        self.leaving = self.pages.pop();
        self.transition = Some(Transition { dir: Dir::Pop, start: t });
        true
    }

    /// Pops back to `depth` pages (1 = the root), animating only the last.
    pub fn pop_to(&mut self, depth: usize, t: f64) {
        let depth = depth.max(1);
        if self.pages.len() <= depth {
            return;
        }
        self.finish();
        self.pages.truncate(depth + 1);
        self.pop(t);
    }

    /// Replaces the stack with `root`, without animation.
    pub fn reset(&mut self, root: P) {
        self.pages.clear();
        self.pages.push(root);
        self.leaving = None;
        self.transition = None;
    }

    fn finish(&mut self) {
        self.leaving = None;
        self.transition = None;
    }

    /// Ends a transition whose time is up; true while one is still running.
    pub fn tick(&mut self, t: f64) -> bool {
        if self.transition.is_some_and(|tr| t - tr.start >= MS) {
            self.finish();
        }
        self.transition.is_some()
    }

    pub fn animating(&self) -> bool {
        self.transition.is_some()
    }

    /// The transition in progress, if any.
    pub fn direction(&self) -> Option<Dir> {
        self.transition.map(|tr| tr.dir)
    }

    /// The pages to draw at `t`: the one underneath (if a transition shows
    /// it), the one on top, and their placement.
    pub fn frame(&self, t: f64, w: f32) -> Frame<'_, P> {
        match self.transition {
            Some(tr) => {
                let p = ((t - tr.start) / MS).clamp(0.0, 1.0) as f32;
                let (under_x, over_x, dim) = placement(tr.dir, p, w);
                let (under, over) = match tr.dir {
                    Dir::Push => (self.under(), self.top()),
                    Dir::Pop => (Some(self.top()), self.leaving.as_ref().unwrap_or(self.top())),
                };
                Frame { under: under.map(|u| (u, under_x, dim)), over, over_x }
            }
            None => Frame { under: None, over: self.top(), over_x: 0.0 },
        }
    }
}

/// One frame of the stack: an optional page underneath with its x offset
/// and shade, and the page on top with its x offset.
pub struct Frame<'a, P> {
    pub under: Option<(&'a P, f32, u8)>,
    pub over: &'a P,
    pub over_x: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_slides_in_and_drifts_under() {
        assert_eq!(placement(Dir::Push, 0.0, 1000.0), (0.0, 1000.0, 0));
        assert_eq!(placement(Dir::Push, 1.0, 1000.0), (-300.0, 0.0, 30));
        assert_eq!(placement(Dir::Pop, 0.0, 1000.0), (-300.0, 0.0, 30));
        assert_eq!(placement(Dir::Pop, 1.0, 1000.0), (0.0, 1000.0, 0));
        let (u, o, d) = placement(Dir::Push, 0.5, 1000.0);
        assert!(u < 0.0 && u > -300.0 && o > 0.0 && o < 1000.0 && d > 0 && d < 30);
        assert!(o < 500.0, "ease-out: more than half way at half time");
    }

    #[test]
    fn push_pop_and_pop_to() {
        let mut n = Nav::new("root");
        assert_eq!((n.depth(), *n.top()), (1, "root"));
        assert!(!n.pop(0.0), "the root stays");
        n.push("a", 100.0);
        assert!(n.animating());
        assert_eq!(n.direction(), Some(Dir::Push));
        {
            let f = n.frame(100.0, 1000.0);
            assert_eq!((f.under.map(|u| *u.0), *f.over, f.over_x), (Some("root"), "a", 1000.0));
        }
        assert!(n.tick(300.0));
        assert!(!n.tick(450.0), "done after MS");
        assert_eq!(n.direction(), None);
        assert_eq!(n.frame(500.0, 1000.0).over_x, 0.0);
        n.push("b", 1000.0);
        assert_eq!(n.under(), Some(&"a"));
        assert!(n.pop(2000.0));
        assert_eq!((n.depth(), *n.top()), (2, "a"));
        {
            let f = n.frame(2000.0, 1000.0);
            assert_eq!((f.under.map(|u| *u.0), *f.over, f.over_x), (Some("a"), "b", 0.0), "the leaving page slides out on top");
        }
        n.tick(3000.0);
        assert_eq!(n.frame(3000.0, 1000.0).over, &"a");
        n.push("c", 4000.0);
        n.push("d", 4001.0);
        assert_eq!(n.pages(), &["root", "a", "c", "d"]);
        n.pop_to(1, 5000.0);
        assert_eq!(n.pages(), &["root"]);
        assert_eq!(n.frame(5000.0, 1000.0).over, &"a", "the page just above the target slides out");
        n.pop_to(5, 6000.0);
        assert_eq!(n.depth(), 1);
        n.reset("r2");
        assert!(!n.animating() && n.depth() == 1 && *n.top() == "r2");
        *n.top_mut() = "r3";
        assert_eq!(n.pages_mut()[0], "r3");
    }
}
