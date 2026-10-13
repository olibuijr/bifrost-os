//! All PTYs stay polled, including tabs behind the active one.
use super::Terminal;
#[derive(Default)]
pub struct Sessions {
    items: Vec<Terminal>,
    active: usize,
    /// two sessions split side by side (left, right); see `pair`
    split: Option<(usize, usize)>,
    pub closed: termcore::tabs::ClosedTabs,
}
impl Sessions {
    /// The split pair, whether or not it is on screen (its sessions are indexes into `iter`).
    pub fn split(&self) -> Option<(usize, usize)> {
        self.split
    }

    /// The split pair while the active session is one of its two: the pair is shown side by side.
    pub fn pair(&self) -> Option<(usize, usize)> {
        self.split.filter(|&(left, right)| self.active == left || self.active == right)
    }

    /// Sets or clears the split pair.
    pub fn set_split(&mut self, pair: Option<(usize, usize)>) {
        self.split = pair;
    }

    pub fn as_ref(&self) -> Option<&Terminal> {
        self.items.get(self.active)
    }
    pub fn as_mut(&mut self) -> Option<&mut Terminal> {
        self.items.get_mut(self.active)
    }
    pub fn get(&self, i: usize) -> Option<&Terminal> {
        self.items.get(i)
    }
    pub fn get_mut(&mut self, i: usize) -> Option<&mut Terminal> {
        self.items.get_mut(i)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn is_none(&self) -> bool {
        self.is_empty()
    }
    pub fn is_some(&self) -> bool {
        !self.is_none()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn active_index(&self) -> usize {
        self.active
    }
    pub fn iter(&self) -> impl Iterator<Item = &Terminal> {
        self.items.iter()
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Terminal> {
        self.items.iter_mut()
    }
    pub fn push(&mut self, t: Terminal) {
        self.items.push(t);
        self.select(self.items.len() - 1);
    }
    pub fn select(&mut self, i: usize) {
        if i < self.items.len() {
            self.active = i;
            self.items[i].unread = false;
            self.items[i].bell = false;
        }
    }
    pub fn close(&mut self, i: usize) {
        if i >= self.items.len() {
            return;
        }
        if let Some(dir) = self.items[i].cwd() {
            self.closed.push(dir);
        }
        // a closed pane ends the pair; the other pane goes on alone, and later indexes move down by one
        self.split = self.split.and_then(|(left, right)| {
            if i == left || i == right {
                None
            } else {
                Some((if left > i { left - 1 } else { left }, if right > i { right - 1 } else { right }))
            }
        });
        self.items.remove(i);
        if i <= self.active {
            self.active = self.active.saturating_sub(1);
        }
        self.select(self.active);
    }
    /// Closes every session but `keep`, which becomes the active one; returns how many were closed.
    pub fn close_others(&mut self, keep: usize) -> usize {
        if keep >= self.items.len() {
            return 0;
        }
        let mut closed = 0;
        for i in (0..self.items.len()).rev() {
            if i != keep {
                self.close(i);
                closed += 1;
            }
        }
        self.select(0);
        closed
    }

    /// Moves session `from` so it sits at index `to`, the others shifting over; the active session and the split pair
    /// follow their sessions. False when either index is out of range or nothing moves.
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        let n = self.items.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let session = self.items.remove(from);
        self.items.insert(to, session);
        self.active = moved_index(self.active, from, to);
        self.split = self.split.map(|(left, right)| (moved_index(left, from, to), moved_index(right, from, to)));
        true
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.active = 0;
        self.split = None;
    }
    /// Returns output/exit changes and how many shells exited.
    pub fn pump(&mut self) -> (bool, usize) {
        let mut changed = false;
        let mut exits = Vec::new();
        for (i, t) in self.items.iter_mut().enumerate() {
            let output = t.pump();
            if output && i != self.active {
                t.unread = true;
            }
            // a bell rung on screen has been seen; only background bells mark the tab
            if i == self.active {
                t.bell = false;
            }
            changed |= output;
            if !t.alive() {
                exits.push(i);
            }
        }
        for &i in exits.iter().rev() {
            self.close(i);
        }
        (changed || !exits.is_empty(), exits.len())
    }
}

/// Where the session that sat at index `i` is after the session at `from` moved to `to`.
fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moved_tab_shifts_the_ones_it_passes() {
        // [a b c d e], b (1) moves to 3: [a c d b e]
        let after: Vec<usize> = (0..5).map(|i| moved_index(i, 1, 3)).collect();
        assert_eq!(after, vec![0, 3, 1, 2, 4]);
        // [a b c d e], d (3) moves to 1: [a d b c e]
        let after: Vec<usize> = (0..5).map(|i| moved_index(i, 3, 1)).collect();
        assert_eq!(after, vec![0, 2, 3, 1, 4]);
        let mut seen = after.clone();
        seen.sort();
        assert_eq!(seen, vec![0, 1, 2, 3, 4], "every session keeps one place");
    }

    fn font() -> &'static [u8] {
        Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice())
    }

    fn sessions(n: usize) -> Sessions {
        let mut s = Sessions::default();
        for _ in 0..n {
            s.push(Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x"]).unwrap());
        }
        s
    }

    fn titled(s: &mut Sessions) {
        for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
            s.get_mut(i).unwrap().set_custom_title(name);
        }
    }

    fn order(s: &Sessions) -> String {
        s.iter().enumerate().map(|(i, t)| t.title(i)).collect()
    }

    #[test]
    fn moving_a_tab_keeps_the_active_session_and_the_split_pair_with_their_sessions() {
        let mut s = sessions(4);
        titled(&mut s);
        s.select(1);
        s.set_split(Some((1, 3)));
        assert!(s.move_tab(1, 3));
        assert_eq!(order(&s), "acdb");
        assert_eq!((s.active_index(), s.split()), (3, Some((3, 2))), "b stays active and paired with d");
        assert!(s.move_tab(3, 0));
        assert_eq!(order(&s), "bacd");
        assert_eq!((s.active_index(), s.split()), (0, Some((0, 3))));
        assert!(!s.move_tab(2, 2) && !s.move_tab(0, 9) && !s.move_tab(9, 0), "nothing to move");
    }

    #[test]
    fn close_others_keeps_one_session_active() {
        let mut s = sessions(4);
        titled(&mut s);
        s.set_split(Some((0, 1)));
        assert_eq!(s.close_others(2), 3);
        assert_eq!(s.len(), 1);
        assert_eq!(s.as_ref().and_then(|t| t.custom_title()), Some("c"));
        assert_eq!((s.active_index(), s.split()), (0, None));
        assert_eq!(s.close_others(0), 0, "a lone session stays");
        assert_eq!(s.close_others(5), 0, "an unknown index closes nothing");
    }
}
