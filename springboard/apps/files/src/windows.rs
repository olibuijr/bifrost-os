//! New Files Window (⌘N, or File > New Window in the empty-area menu): a second Files window with its own folder
//! view. The window is a whole `Files` of its own, so its folder, selection and view never move the main window's.
//! Keys and menus stay with the main window; a new window takes touches, draws, and is dropped when the shell closes it.
use crate::*;
use bifrost_app::app_windows::{WindowId, WindowSpec};

/// Size of a window opened with New Window, in points.
const NEW_WINDOW_PT: (f32, f32) = (900.0, 600.0);

/// A window opened with New Window: its id and the folder view it shows.
pub(crate) struct ExtraWindow {
    pub(crate) id: WindowId,
    pub(crate) files: Box<Files>,
    /// The pixel size of the surface the last draw went into. The SDK gives a window fresh, empty buffers whenever its
    /// size changes (the desk resizes the body after it opens), and `Files::render` repaints only what changed.
    drawn: Option<(usize, usize)>,
}

impl ExtraWindow {
    pub(crate) fn new(id: WindowId, files: Box<Files>) -> Self {
        Self { id, files, drawn: None }
    }

    /// Draws the window. A surface of another size than the last one is blank, so everything is painted again: without
    /// that the state matched the last paint, nothing was redrawn and the new buffers went out empty (a transparent
    /// window body).
    pub(crate) fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if self.drawn != Some((s.w, s.h)) {
            self.files.painted_state.clear();
            self.drawn = Some((s.w, s.h));
        }
        self.files.draw(ctx, s, t)
    }
}

impl Files {
    /// New Window (⌘N): opens a window at the home folder. Returns false when the shell refuses the window.
    pub(crate) fn new_window(&mut self, ctx: &Ctx) -> bool {
        let (w, h) = NEW_WINDOW_PT;
        let Ok(id) = ctx.open_window(WindowSpec::new("Files", w, h)) else { return false };
        let mut files = Box::new(Files::new());
        files.data_dir = Some(ctx.data_dir.clone());
        files.w = w;
        files.h = h;
        files.update_controls();
        self.windows.push(ExtraWindow::new(id, files));
        true
    }

    /// Drops the state of a window the shell or the app closed. Returns true when one was dropped.
    pub(crate) fn close_extra(&mut self, window: WindowId) -> bool {
        let before = self.windows.len();
        self.windows.retain(|e| e.id != window);
        self.windows.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_extra_window_is_dropped_once() {
        let mut files = Files::new();
        assert!(files.windows.is_empty(), "the main window is not in the list");
        files.windows.push(ExtraWindow::new(WindowId(1), Box::new(Files::new())));
        files.windows.push(ExtraWindow::new(WindowId(2), Box::new(Files::new())));
        assert!(!files.close_extra(WindowId(9)), "an unknown window is left alone");
        assert!(files.close_extra(WindowId(1)));
        assert!(!files.close_extra(WindowId(1)), "a window is removed once");
        assert_eq!(files.windows.iter().map(|e| e.id).collect::<Vec<_>>(), vec![WindowId(2)]);
    }
}
