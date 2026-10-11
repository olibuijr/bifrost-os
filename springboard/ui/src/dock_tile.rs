//! The app's Dock tile (`NativeDockTile`): the badge text, whether the tile carries the app-icon badge, and whether the
//! tile must be drawn again. The shell paints the tile; this model keeps the rules every app sees the same way.
/// Characters the Dock shows in a badge. Longer labels are cut here, so the app and the shell agree on what is visible.
pub const BADGE_CHARS: usize = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockTile {
    label: String,
    app_icon_badge: bool,
    dirty: bool,
}

impl DockTile {
    pub fn new() -> Self {
        DockTile::default()
    }

    /// The full badge label as the app set it (`badgeLabel`). An empty label clears the badge.
    pub fn badge_label(&self) -> &str {
        &self.label
    }

    /// Sets the badge label and marks the tile for redrawing only when the text changes.
    pub fn set_badge_label(&mut self, label: &str) {
        if self.label != label {
            self.label = label.into();
            self.dirty = true;
        }
    }

    /// The badge text the Dock shows: the first [`BADGE_CHARS`] characters, or None when there is no label.
    pub fn displayed_badge(&self) -> Option<String> {
        if self.label.is_empty() {
            return None;
        }
        Some(self.label.chars().take(BADGE_CHARS).collect())
    }

    /// Whether the tile is badged with the app's icon (`showsApplicationBadge`).
    pub fn shows_application_badge(&self) -> bool {
        self.app_icon_badge
    }

    pub fn set_shows_application_badge(&mut self, on: bool) {
        if self.app_icon_badge != on {
            self.app_icon_badge = on;
            self.dirty = true;
        }
    }

    /// Asks for a redraw of the tile's content (`display()`), even when nothing changed.
    pub fn display(&mut self) {
        self.dirty = true;
    }

    /// True once after each change or request for a redraw. The shell calls it before painting the tile.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tile_has_no_badge_and_needs_no_redraw() {
        let mut tile = DockTile::new();
        assert_eq!(tile.badge_label(), "");
        assert_eq!(tile.displayed_badge(), None);
        assert!(!tile.shows_application_badge());
        assert!(!tile.take_dirty());
    }

    #[test]
    fn the_dock_shows_at_most_four_characters_counted_as_characters() {
        let mut tile = DockTile::new();
        tile.set_badge_label("12345");
        assert_eq!(tile.badge_label(), "12345");
        assert_eq!(tile.displayed_badge().as_deref(), Some("1234"));
        tile.set_badge_label("åéíóú");
        assert_eq!(tile.displayed_badge().as_deref(), Some("åéíó"));
    }

    #[test]
    fn an_empty_label_clears_the_badge() {
        let mut tile = DockTile::new();
        tile.set_badge_label("3");
        tile.set_badge_label("");
        assert_eq!(tile.displayed_badge(), None);
        assert_eq!(tile.badge_label(), "");
    }

    #[test]
    fn redraw_is_requested_only_when_the_tile_changes() {
        let mut tile = DockTile::new();
        tile.set_badge_label("7");
        assert!(tile.take_dirty());
        assert!(!tile.take_dirty());
        tile.set_badge_label("7");
        assert!(!tile.take_dirty());
        tile.set_shows_application_badge(true);
        assert!(tile.take_dirty());
        tile.set_shows_application_badge(true);
        assert!(!tile.take_dirty());
    }

    #[test]
    fn display_forces_a_redraw_even_without_a_change() {
        let mut tile = DockTile::new();
        tile.display();
        assert!(tile.take_dirty());
        assert!(!tile.take_dirty());
    }

    #[test]
    fn the_application_badge_is_independent_of_the_label() {
        let mut tile = DockTile::new();
        tile.set_shows_application_badge(true);
        tile.set_badge_label("9");
        assert!(tile.shows_application_badge());
        assert_eq!(tile.displayed_badge().as_deref(), Some("9"));
        tile.set_shows_application_badge(false);
        assert_eq!(tile.displayed_badge().as_deref(), Some("9"));
    }
}
