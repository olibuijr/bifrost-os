//! Desktops and full-screen spaces: the window sets one output shows, one at a time. A desktop is a plain set of
//! windows that the switch keys move between (Ctrl+Left and Right step, Ctrl+1 to 9 pick a number). A full-screen
//! window gets a space of its own, placed after the desktops, and returns to the desktop it came from when it leaves
//! full screen. Pure model with no Smithay types: the shell maps the active space's windows and unmaps the rest.

use crate::wm::WindowId;

pub type SpaceId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Desktop,
    /// One window in full screen, and the desktop it leaves and returns to.
    FullScreen { window: WindowId, home: SpaceId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Space {
    id: SpaceId,
    kind: Kind,
    windows: Vec<WindowId>,
}

/// The spaces of one output in switch order, and the one shown. There is always at least one desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spaces {
    spaces: Vec<Space>,
    active: SpaceId,
    next: SpaceId,
}

impl Default for Spaces {
    fn default() -> Self { Self::new() }
}

impl Spaces {
    /// One desktop (id 1), shown.
    pub fn new() -> Self {
        Spaces { spaces: vec![Space { id: 1, kind: Kind::Desktop, windows: Vec::new() }], active: 1, next: 2 }
    }

    /// Every space id in switch order.
    pub fn order(&self) -> Vec<SpaceId> { self.spaces.iter().map(|s| s.id).collect() }

    pub fn active(&self) -> SpaceId { self.active }

    pub fn kind(&self, id: SpaceId) -> Option<Kind> { self.find(id).map(|s| s.kind) }

    /// The windows in a space, in the order they were placed.
    pub fn windows(&self, id: SpaceId) -> &[WindowId] {
        match self.find(id) {
            Some(s) => s.windows.as_slice(),
            None => &[],
        }
    }

    pub fn space_of(&self, window: WindowId) -> Option<SpaceId> {
        self.spaces.iter().find(|s| s.windows.contains(&window)).map(|s| s.id)
    }

    /// Whether the window is in the space shown now.
    pub fn shown(&self, window: WindowId) -> bool { self.windows(self.active).contains(&window) }

    pub fn desktops(&self) -> usize { self.spaces.iter().filter(|s| s.kind == Kind::Desktop).count() }

    /// The n-th desktop, counting from 1, as the number keys pick it.
    pub fn desktop_number(&self, n: usize) -> Option<SpaceId> {
        self.spaces.iter().filter(|s| s.kind == Kind::Desktop).nth(n.checked_sub(1)?).map(|s| s.id)
    }

    /// Adds an empty desktop after the last one (before the full-screen spaces); returns its id.
    pub fn add_desktop(&mut self) -> SpaceId {
        let id = self.fresh();
        let at = self.spaces.iter().rposition(|s| s.kind == Kind::Desktop).map_or(0, |i| i + 1);
        self.spaces.insert(at, Space { id, kind: Kind::Desktop, windows: Vec::new() });
        id
    }

    /// Removes a desktop. Its windows move to the nearest other desktop, a full-screen space that came from it now
    /// returns there, and the view moves there if it was shown. The last desktop cannot be removed.
    pub fn remove_desktop(&mut self, id: SpaceId) -> bool {
        if self.desktops() <= 1 {
            return false;
        }
        let Some(i) = self.index(id) else { return false };
        if self.spaces[i].kind != Kind::Desktop {
            return false;
        }
        let Some(target) = self
            .spaces
            .iter()
            .enumerate()
            .filter(|(j, s)| *j != i && s.kind == Kind::Desktop)
            .min_by_key(|(j, _)| (*j as i64 - i as i64).abs())
            .map(|(_, s)| s.id)
        else {
            return false;
        };
        let removed = self.spaces.remove(i);
        if let Some(t) = self.find_mut(target) {
            t.windows.extend(removed.windows);
        }
        for s in &mut self.spaces {
            if let Kind::FullScreen { home, .. } = &mut s.kind {
                if *home == id {
                    *home = target;
                }
            }
        }
        if self.active == id {
            self.active = target;
        }
        true
    }

    /// Shows the space `delta` steps along the switch order (negative goes back). Stops at the first and last space.
    pub fn step(&mut self, delta: i32) -> SpaceId {
        let last = self.spaces.len() as i64 - 1;
        let i = self.index(self.active).unwrap_or(0) as i64;
        self.active = self.spaces[(i + delta as i64).clamp(0, last) as usize].id;
        self.active
    }

    /// Shows a space by id; false if there is none.
    pub fn switch_to(&mut self, id: SpaceId) -> bool {
        let found = self.find(id).is_some();
        if found {
            self.active = id;
        }
        found
    }

    /// Puts a window that has no space yet on the desktop in view (a full-screen view opens its desktop). A window
    /// that already has a space stays where it is.
    pub fn place(&mut self, window: WindowId) {
        if self.space_of(window).is_some() {
            return;
        }
        let home = self.desktop_of(self.active);
        if let Some(s) = self.find_mut(home) {
            s.windows.push(window);
        }
    }

    /// The window goes full screen: it leaves its desktop for a space of its own, after the others, and that space is
    /// shown. A window already full screen just gets its space shown.
    pub fn enter_full_screen(&mut self, window: WindowId) -> SpaceId {
        if let Some(id) = self.full_screen_space(window) {
            self.active = id;
            return id;
        }
        let home = match self.space_of(window) {
            Some(s) => self.desktop_of(s),
            None => self.desktop_of(self.active),
        };
        for s in &mut self.spaces {
            s.windows.retain(|w| *w != window);
        }
        let id = self.fresh();
        self.spaces.push(Space { id, kind: Kind::FullScreen { window, home }, windows: vec![window] });
        self.active = id;
        id
    }

    /// The full-screen window leaves full screen: its space goes and it returns to its desktop, which is shown if the
    /// full-screen space was. Returns that desktop.
    pub fn exit_full_screen(&mut self, window: WindowId) -> Option<SpaceId> {
        let i = self.spaces.iter().position(|s| matches!(s.kind, Kind::FullScreen { window: w, .. } if w == window))?;
        let home = match self.spaces[i].kind {
            Kind::FullScreen { home, .. } => home,
            Kind::Desktop => return None,
        };
        let space = self.spaces.remove(i);
        let target = if self.find(home).is_some() { home } else { self.spaces[0].id };
        if let Some(t) = self.find_mut(target) {
            t.windows.extend(space.windows);
        }
        if self.active == space.id {
            self.active = target;
        }
        Some(target)
    }

    /// Moves a window to a desktop (the shell's Move To menu). A full-screen window does not move; false too for an
    /// unknown id or a space that is not a desktop.
    pub fn move_to(&mut self, window: WindowId, desktop: SpaceId) -> bool {
        if self.kind(desktop) != Some(Kind::Desktop) || self.full_screen_space(window).is_some() {
            return false;
        }
        for s in &mut self.spaces {
            s.windows.retain(|w| *w != window);
        }
        if let Some(s) = self.find_mut(desktop) {
            s.windows.push(window);
        }
        true
    }

    /// The window closed: it is removed from its space. A full-screen space that loses its window goes, and the view
    /// returns to its desktop.
    pub fn forget(&mut self, window: WindowId) {
        for s in &mut self.spaces {
            s.windows.retain(|w| *w != window);
        }
        self.exit_full_screen(window);
    }

    fn full_screen_space(&self, window: WindowId) -> Option<SpaceId> {
        self.spaces.iter().find(|s| matches!(s.kind, Kind::FullScreen { window: w, .. } if w == window)).map(|s| s.id)
    }

    /// The desktop a space belongs to: itself for a desktop, its home for a full-screen space.
    fn desktop_of(&self, id: SpaceId) -> SpaceId {
        match self.kind(id) {
            Some(Kind::FullScreen { home, .. }) => home,
            _ => id,
        }
    }

    fn fresh(&mut self) -> SpaceId {
        let id = self.next;
        self.next += 1;
        id
    }

    fn index(&self, id: SpaceId) -> Option<usize> { self.spaces.iter().position(|s| s.id == id) }
    fn find(&self, id: SpaceId) -> Option<&Space> { self.spaces.iter().find(|s| s.id == id) }
    fn find_mut(&mut self, id: SpaceId) -> Option<&mut Space> { self.spaces.iter_mut().find(|s| s.id == id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_window_opens_on_the_desktop_in_view_and_is_shown() {
        let mut s = Spaces::new();
        s.place(7);
        assert_eq!(s.space_of(7), Some(1));
        assert!(s.shown(7));
        s.place(7);
        assert_eq!(s.windows(1), &[7], "placing twice does not duplicate");
    }

    #[test]
    fn switching_steps_along_the_order_and_stops_at_the_ends() {
        let mut s = Spaces::new();
        let two = s.add_desktop();
        let three = s.add_desktop();
        assert_eq!(s.order(), vec![1, two, three]);
        s.place(1);
        assert_eq!(s.step(1), two);
        s.place(2);
        assert!(!s.shown(1), "a window on another desktop is hidden");
        assert_eq!(s.step(5), three, "stops at the last desktop");
        assert_eq!(s.step(-9), 1, "stops at the first");
        assert!(s.shown(1));
    }

    #[test]
    fn number_keys_pick_the_nth_desktop() {
        let mut s = Spaces::new();
        let two = s.add_desktop();
        assert_eq!(s.desktop_number(2), Some(two));
        assert_eq!(s.desktop_number(3), None);
        assert_eq!(s.desktop_number(0), None);
        assert!(s.switch_to(two));
        assert_eq!(s.active(), two);
        assert!(!s.switch_to(99));
    }

    #[test]
    fn a_full_screen_window_gets_its_own_space_after_the_desktops_and_returns_home() {
        let mut s = Spaces::new();
        let two = s.add_desktop();
        s.place(5);
        let fs = s.enter_full_screen(5);
        assert_eq!(s.kind(fs), Some(Kind::FullScreen { window: 5, home: 1 }));
        assert_eq!(s.order().last(), Some(&fs));
        assert_eq!(s.active(), fs);
        assert!(s.shown(5));
        assert_eq!(s.step(-2), 1, "the desktop left behind is two steps back, past the other desktop");
        assert!(!s.shown(5));
        assert_eq!(s.step(2), fs, "the full-screen space is reached by stepping past the desktops");
        assert_eq!(s.exit_full_screen(5), Some(1));
        assert_eq!(s.space_of(5), Some(1));
        assert_eq!(s.active(), 1, "leaving full screen from its space shows the desktop it came from");
        assert_eq!(s.kind(fs), None);
        assert_eq!(s.order(), vec![1, two]);
    }

    #[test]
    fn entering_full_screen_twice_does_not_make_a_second_space() {
        let mut s = Spaces::new();
        let a = s.enter_full_screen(3);
        assert_eq!(s.enter_full_screen(3), a);
        assert_eq!(s.order().len(), 2);
    }

    #[test]
    fn removing_a_desktop_moves_its_windows_and_full_screen_homes_to_the_nearest_desktop() {
        let mut s = Spaces::new();
        let two = s.add_desktop();
        let three = s.add_desktop();
        s.switch_to(three);
        s.place(9);
        let fs = s.enter_full_screen(4);
        assert!(s.remove_desktop(three));
        assert_eq!(s.active(), two, "the view moves off a removed desktop");
        assert_eq!(s.space_of(9), Some(two));
        assert_eq!(s.kind(fs), Some(Kind::FullScreen { window: 4, home: two }));
    }

    #[test]
    fn the_last_desktop_cannot_be_removed_and_full_screen_spaces_are_not_desktops() {
        let mut s = Spaces::new();
        assert!(!s.remove_desktop(1));
        let fs = s.enter_full_screen(2);
        assert!(!s.remove_desktop(fs));
        assert_eq!(s.desktops(), 1);
    }

    #[test]
    fn a_window_moves_to_another_desktop_unless_it_is_full_screen() {
        let mut s = Spaces::new();
        let two = s.add_desktop();
        s.place(1);
        assert!(s.move_to(1, two));
        assert_eq!(s.space_of(1), Some(two));
        assert!(!s.move_to(1, 99), "unknown desktop");
        let fs = s.enter_full_screen(2);
        assert!(!s.move_to(2, two), "a full-screen window stays in its space");
        assert_eq!(s.space_of(2), Some(fs));
    }

    #[test]
    fn a_closed_window_leaves_no_space_and_its_full_screen_space_goes() {
        let mut s = Spaces::new();
        s.place(1);
        s.enter_full_screen(2);
        s.forget(1);
        assert_eq!(s.space_of(1), None);
        s.forget(2);
        assert_eq!(s.order(), vec![1]);
        assert_eq!(s.active(), 1);
    }
}
