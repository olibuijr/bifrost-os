//! The stacking order of the scene windows of one app in the window server. Each scene window is one surface; the stack
//! keeps them back to front, so raising a window, closing one (the next window in front takes focus) and hit testing in
//! front-to-back order all work on the same list.
//!
//! The window server keeps one stack per app. A surface id is whatever the server uses for the window; the stack does
//! not look at the surface itself, which keeps the rules free of compositor types.
//!
//! ```ignore
//! let mut stack = SceneStack::default();
//! stack.push(surface_a);
//! stack.push(surface_b);            // b is in front
//! stack.raise(surface_a);           // a is in front
//! let hits = stack.front_to_back(); // [a, b]
//! ```

/// The surface ids of one app's scene windows, back to front.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneStack {
    order: Vec<u32>,
}

impl SceneStack {
    /// Adds a surface in front. A surface already in the stack is raised instead of added twice.
    pub fn push(&mut self, surface: u32) {
        self.raise(surface);
    }

    /// Moves a surface to the front, adding it when it is new. Returns whether the front changed.
    pub fn raise(&mut self, surface: u32) -> bool {
        if self.front() == Some(surface) {
            return false;
        }
        self.order.retain(|&s| s != surface);
        self.order.push(surface);
        true
    }

    /// Removes a surface. Returns whether it was in the stack.
    pub fn remove(&mut self, surface: u32) -> bool {
        let before = self.order.len();
        self.order.retain(|&s| s != surface);
        self.order.len() != before
    }

    /// The surface in front, which has focus.
    pub fn front(&self) -> Option<u32> {
        self.order.last().copied()
    }

    pub fn contains(&self, surface: u32) -> bool {
        self.order.contains(&surface)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The surfaces back to front, the order they are painted in.
    pub fn back_to_front(&self) -> &[u32] {
        &self.order
    }

    /// The surfaces front to back, the order a pointer hit is tested in.
    pub fn front_to_back(&self) -> Vec<u32> {
        self.order.iter().rev().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_pushed_surface_is_in_front() {
        let mut s = SceneStack::default();
        assert!(s.is_empty());
        s.push(1);
        s.push(2);
        s.push(3);
        assert_eq!(s.front(), Some(3));
        assert_eq!(s.back_to_front(), &[1, 2, 3]);
        assert_eq!(s.front_to_back(), vec![3, 2, 1]);
    }

    #[test]
    fn raising_moves_a_surface_to_the_front_once() {
        let mut s = SceneStack::default();
        s.push(1);
        s.push(2);
        assert!(s.raise(1));
        assert_eq!(s.back_to_front(), &[2, 1]);
        assert!(!s.raise(1), "a surface already in front does not change the front");
        s.push(1);
        assert_eq!(s.len(), 2, "pushing an existing surface does not duplicate it");
    }

    #[test]
    fn closing_the_front_surface_leaves_the_next_one_in_front() {
        let mut s = SceneStack::default();
        s.push(1);
        s.push(2);
        s.push(3);
        assert!(s.remove(3));
        assert_eq!(s.front(), Some(2));
        assert!(!s.remove(3));
        assert!(!s.contains(3));
    }

    #[test]
    fn removing_from_an_empty_stack_is_harmless() {
        let mut s = SceneStack::default();
        assert!(!s.remove(7));
        assert_eq!(s.front(), None);
        assert!(s.front_to_back().is_empty());
    }
}
