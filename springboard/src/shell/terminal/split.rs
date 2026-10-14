//! Split panes: two sessions side by side in the Terminal. The pair is on screen while one of its panes is active,
//! a tap on a half makes that pane active, and the keys go to the active pane.
use super::*;
use crate::gfx::pack;

const DIVIDER: u32 = pack(58, 58, 58, 255);

/// The px rects `(left, right)` of two panes that share `area`, with a one-pixel divider between them.
pub fn halves((x, y, w, h): (i32, i32, i32, i32)) -> ((i32, i32, i32, i32), (i32, i32, i32, i32)) {
    let left = w / 2;
    ((x, y, left, h), (x + left + 1, y, (w - left - 1).max(1), h))
}

impl Shell {
    /// The px rect of session `i`: the whole terminal area, or its half while it is one of the shown pair.
    fn pane_area(&self, i: usize) -> (i32, i32, i32, i32) {
        let area = self.term_area();
        match self.term.pair() {
            Some((left, _)) if i == left => halves(area).0,
            Some((_, right)) if i == right => halves(area).1,
            _ => area,
        }
    }

    /// Gives every session its pane. A session already at its size is left alone, so this is cheap to call on
    /// each frame (a resize clears the selection and repaints the grid).
    pub(crate) fn resize_terminals(&mut self) {
        let areas: Vec<(i32, i32, i32, i32)> = (0..self.term.len()).map(|i| self.pane_area(i)).collect();
        for (tab, area) in self.term.iter_mut().zip(areas) {
            if tab.area() != area {
                tab.resize(area);
            }
        }
    }

    /// Draws the active tab: its one pane, or both panes of the shown pair with a divider between them.
    pub(crate) fn draw_panes(&mut self, s: &mut Surface) {
        self.resize_terminals();
        match self.term.pair() {
            Some((left, right)) => {
                for i in [left, right] {
                    if let Some(pane) = self.term.get_mut(i) {
                        pane.draw(s);
                    }
                }
                let (left_rect, _) = halves(self.term_area());
                let (x, y, _, h) = left_rect;
                s.fill_rect((x + left_rect.2) as usize, y as usize, 1, h.max(0) as usize, DIVIDER);
            }
            None => {
                if let Some(tab) = self.term.as_mut() {
                    tab.draw(s);
                }
            }
        }
    }

    /// A tap on one half of the shown pair makes that pane the active one (keys go there from then on).
    pub(crate) fn select_pane_at(&mut self, x: f32) {
        let Some((left, right)) = self.term.pair() else { return };
        let (left_rect, _) = halves(self.term_area());
        let split_x = (left_rect.0 + left_rect.2) as f32;
        self.term.select(if x < split_x { left } else { right });
    }

    /// Splits the active tab: a new shell in the same folder opens beside it and takes the keys.
    pub(super) fn terminal_split(&mut self, t: f64) {
        if self.term.split().is_some() {
            self.terminal_ui.message = "Close the split pane first".into();
            return;
        }
        let at = self.term.active_index();
        let dir = self.term.as_ref().and_then(|tab| tab.cwd());
        if self.new_terminal_tab(dir) {
            let new = self.term.len() - 1;
            self.term.set_split(Some((at, new)));
            self.terminal_close_panel(t);
        }
    }

    /// Closes the active pane of the shown pair; the other pane goes on alone.
    pub(super) fn terminal_close_split(&mut self, t: f64) {
        if self.term.pair().is_none() {
            self.terminal_ui.message = "Select a split pane first".into();
            return;
        }
        let at = self.term.active_index();
        self.close_terminal_tab(at);
        self.terminal_close_panel(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_share_the_area_with_a_one_pixel_divider() {
        let (left, right) = halves((0, 100, 1201, 800));
        assert_eq!(left, (0, 100, 600, 800));
        assert_eq!(right, (601, 100, 600, 800));
        assert_eq!(left.2 + 1 + right.2, 1201, "the panes and the divider fill the width");
    }
}
