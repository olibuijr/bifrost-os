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
