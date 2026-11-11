//! New Files Window (⌘N, or File > New Window in the empty-area menu): a second Files window with its own folder
//! view. The window is a whole `Files` of its own, so its folder, selection and view never move the main window's.
//! A new window takes its touches and, while it is the key window, the keys, the Edit commands and the File menu
//! (`App::window_event`, `window_input`, `menus_for` in main.rs); it draws, and is dropped when the shell closes it.
//! What one window copies or cuts, every window pastes (`share_clip`).
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
            // the window lays itself out for the surface it is given: the desk sizes the body after the window opens and
            // on every resize, and a layout kept at the size asked for left the toolbar, the scroll range and the click
            // targets where a window of another size has them
            let (w, h) = surface_pt(s.w, s.h, ctx.ui.s);
            if (w, h) != (self.files.w, self.files.h) {
                self.files.w = w;
                self.files.h = h;
                self.files.update_controls();
            }
        }
        self.files.draw(ctx, s, t)
    }
}

/// What New Window asks the shell for. The window is the main window again: its content runs under the title bar
/// (the manifest's `window = "full-size"` for the main one), so the traffic lights sit over the sidebar and the
/// toolbar row is the title row, with no second title bar above it. That needs a shell that takes the window's toolbar
/// controls and blurs behind its sidebar (`chrome`, `shell_takes_window_chrome`); on any other shell the window keeps
/// the shell's title bar, where every press in the toolbar still reaches the app. Either way it is cascaded from the
/// app's windows instead of covering the one it was opened from.
pub(crate) fn new_window_spec(w: f32, h: f32, chrome: bool) -> WindowSpec {
    let spec = WindowSpec::new("Files", w, h).cascaded();
    if chrome { spec.full_size_content() } else { spec }
}

/// A surface's size in points at `scale` px per point, as `App::configure` gives the main window its size.
pub(crate) fn surface_pt(w: usize, h: usize, scale: f32) -> (f32, f32) {
    let k = scale.max(0.01);
    (w as f32 / k, h as f32 / k)
}

/// Whether ⌘W in an extra window closes the window: always a Get Info window, and a browser window that has no
/// second tab to close (with tabs, ⌘W closes the open tab) and no name being typed.
pub(crate) fn closes_window(info_window: bool, tabs: usize, typing: bool) -> bool {
    info_window || (tabs < 2 && !typing)
}

impl Files {
    /// New Window (⌘N): opens a window at the home folder. Returns false when the shell refuses the window.
    pub(crate) fn new_window(&mut self, ctx: &Ctx) -> bool {
        let (w, h) = NEW_WINDOW_PT;
        let Ok(id) = ctx.open_window(new_window_spec(w, h, bifrost_app::app_windows::shell_takes_window_chrome())) else { return false };
        let mut files = Box::new(Files::new());
        files.data_dir = Some(ctx.data_dir.clone());
        // its folder is watched by the main window's stream, and the main window's `tick` runs its share
        files.fx.extra_window = true;
        files.fx.watch_stale = true;
        files.w = w;
        files.h = h;
        // what was copied or cut before the window opened can be pasted in it
        files.clip = self.clip.clone();
        files.update_controls();
        self.windows.push(ExtraWindow::new(id, files));
        true
    }

    /// The folder view of `window`: this one for the main window, else the New Window or Get Info window of that id
    /// (this one again when no such window is open).
    pub(crate) fn window_files(&self, window: WindowId) -> &Files {
        if window.is_main() {
            return self;
        }
        self.windows.iter().find(|e| e.id == window).map_or(self, |e| &*e.files)
    }

    /// The controls of the New Window window `window`, in its points, for the shell's drag band (`App::window_controls`):
    /// the toolbar buttons and the search field take their presses. None for a Get Info window or a window that is gone.
    pub(crate) fn window_nodes(&self, window: WindowId) -> Vec<Node> {
        match self.windows.iter().find(|e| e.id == window) {
            Some(e) if !e.files.info_window => e.files.controls.nodes().to_vec(),
            _ => Vec::new(),
        }
    }

    /// The windows the extra window at `at` opened (Get Info, New Window) join the main window's list: they are
    /// drawn, ticked and closed from there, with the others.
    pub(crate) fn adopt_opened(&mut self, at: usize) {
        let Some(e) = self.windows.get_mut(at) else { return };
        let mut opened = std::mem::take(&mut e.files.windows);
        // a window opened from a New Window window pastes what the others do
        let clip = self.clip.clone();
        for w in &mut opened {
            if !w.files.info_window {
                w.files.clip = clip.clone();
            }
        }
        self.windows.extend(opened);
    }

    /// One window copied or cut (`from`: the extra window at that place, or None for this, the main window): every
    /// other window takes the same items for its Paste. Without it a Cut in one window was pasted as a copy in
    /// another, because only the window that cut knew the items were cut.
    pub(crate) fn share_clip(&mut self, from: Option<usize>) {
        let clip = match from {
            Some(i) => match self.windows.get(i) {
                Some(e) => e.files.clip.clone(),
                None => return,
            },
            None => self.clip.clone(),
        };
        if from.is_some() {
            self.clip = clip.clone();
        }
        for (i, e) in self.windows.iter_mut().enumerate() {
            if Some(i) != from && !e.files.info_window {
                e.files.clip = clip.clone();
            }
        }
    }

    /// Shares what any window copied or cut since the last look, on whatever way it did (a key, a menu item, the
    /// context menu, a control call): the windows are compared by their copy counts, so no route has to remember to
    /// share. The newest copy wins when two windows copied since.
    pub(crate) fn share_clips(&mut self) {
        if self.fx.clip_rev != self.fx.clip_shared {
            self.fx.clip_shared = self.fx.clip_rev;
            self.share_clip(None);
        }
        for i in 0..self.windows.len() {
            let fx = &mut self.windows[i].files.fx;
            if fx.clip_rev != fx.clip_shared {
                fx.clip_shared = fx.clip_rev;
                self.share_clip(Some(i));
            }
        }
    }

    /// Drops the state of a window the shell or the app closed. Returns true when one was dropped.
    pub(crate) fn close_extra(&mut self, window: WindowId) -> bool {
        let before = self.windows.len();
        self.windows.retain(|e| e.id != window);
        // its folder needs no watching any more
        self.fx.watch_stale = true;
        self.windows.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fixture, folder};

    /// A main window with one New Window window (id 4), each showing the folders `a` and `b`.
    fn two_windows() -> Files {
        let mut main = fixture(vec![folder("a"), folder("b")]);
        let second = fixture(vec![folder("a"), folder("b")]);
        main.windows.push(ExtraWindow::new(WindowId(4), Box::new(second)));
        main
    }

    #[test]
    fn the_edit_and_file_menus_follow_the_window_that_has_the_keys() {
        let mut main = two_windows();
        // an item is selected in the second window only
        main.windows[0].files.selection = vec![1];
        assert!(!main.window_files(WindowId::MAIN).edit_commands().supports(Command::Copy), "nothing selected in the main window");
        let second = main.window_files(WindowId(4));
        assert!(second.edit_commands().supports(Command::Copy) && second.edit_commands().supports(Command::Cut));
        assert!(second.edit_commands().supports(Command::Paste));
        let enabled = |files: &Files, id: u32| files.file_menu_items().iter().find(|i| i.id == id).is_some_and(|i| i.enabled);
        assert!(enabled(main.window_files(WindowId(4)), MENU_DUPLICATE) && enabled(main.window_files(WindowId(4)), MENU_INFO));
        assert!(!enabled(main.window_files(WindowId::MAIN), MENU_DUPLICATE) && !enabled(main.window_files(WindowId::MAIN), MENU_INFO));
        // a window that is gone falls back to the main window
        assert!(!main.window_files(WindowId(9)).edit_commands().supports(Command::Copy));
        // a Get Info window has nothing the menus act on
        main.windows[0].files.info_window = true;
        assert!(!main.window_files(WindowId(4)).edit_commands().supports(Command::SelectAll));
        assert!(main.window_files(WindowId(4)).file_menu_items().iter().all(|i| !i.enabled));
    }

    #[test]
    fn a_command_acts_on_the_window_it_was_sent_to() {
        let mut main = two_windows();
        main.windows[0].files.selection = vec![1];
        // Copy in the second window (what `window_event` routes there) copies its selection, not the main window's
        main.windows[0].files.copy_selected(false);
        assert_eq!(main.windows[0].files.clip, Some((vec![PathBuf::from("/fixture/b")], false)));
        assert_eq!(main.clip, None);
        // Select All and the view keys change that window alone
        main.windows[0].files.select_all();
        assert!(main.windows[0].files.chord(3, CHORD_META), "⌘2: the list view");
        assert_eq!(main.windows[0].files.selection.len(), 2);
        assert!(!main.windows[0].files.grid && main.grid);
        assert!(main.selection.is_empty());
    }

    #[test]
    fn what_one_window_copies_or_cuts_every_window_pastes() {
        let mut main = two_windows();
        main.windows.push(ExtraWindow::new(WindowId(5), Box::new(fixture(vec![]))));
        let mut info = fixture(vec![]);
        info.info_window = true;
        main.windows.push(ExtraWindow::new(WindowId(6), Box::new(info)));
        // a cut in the second window
        main.windows[0].files.selection = vec![0];
        main.windows[0].files.copy_selected(true);
        main.share_clip(Some(0));
        let cut = Some((vec![PathBuf::from("/fixture/a")], true));
        assert_eq!(main.clip, cut, "the main window pastes it as a move");
        assert_eq!(main.windows[1].files.clip, cut);
        assert_eq!(main.windows[2].files.clip, None, "a Get Info window pastes nothing");
        // then a copy in the main window replaces it everywhere
        main.selection = vec![1];
        main.copy_selected(false);
        main.share_clip(None);
        let copied = Some((vec![PathBuf::from("/fixture/b")], false));
        assert_eq!(main.windows[0].files.clip, copied);
        assert_eq!(main.windows[1].files.clip, copied);
        main.share_clip(Some(9));
        assert_eq!(main.clip, copied, "an unknown window changes nothing");
    }

    #[test]
    fn a_new_window_is_a_full_size_window_where_the_shell_gives_it_the_main_windows_chrome() {
        let spec = new_window_spec(900.0, 600.0, true);
        assert!(spec.full_size && spec.cascade);
        assert_eq!((spec.title.as_str(), spec.width, spec.height), ("Files", 900.0, 600.0));
        // a shell that does not: the shell's title bar stays, and the toolbar keeps its presses
        let spec = new_window_spec(900.0, 600.0, false);
        assert!(!spec.full_size && spec.cascade);
    }

    #[test]
    fn the_toolbar_controls_of_an_extra_window_are_its_own() {
        let mut main = two_windows();
        main.windows[0].files.w = 700.;
        main.windows[0].files.update_controls();
        main.update_controls();
        let own: Vec<_> = main.windows[0].files.controls.nodes().iter().map(|n| (n.id, n.bounds)).collect();
        let told: Vec<_> = main.window_nodes(WindowId(4)).iter().map(|n| (n.id, n.bounds)).collect();
        assert!(!told.is_empty(), "the toolbar buttons");
        assert_eq!(told, own);
        assert!(main.window_nodes(WindowId(9)).is_empty(), "a window that is gone has none");
        // a Get Info window keeps the shell's title bar: it names no controls
        main.windows[0].files.info_window = true;
        assert!(main.window_nodes(WindowId(4)).is_empty());
    }

    #[test]
    fn a_window_is_laid_out_for_the_surface_it_draws_into() {
        assert_eq!(surface_pt(1200, 800, 1.25), (960.0, 640.0));
        assert_eq!(surface_pt(900, 600, 1.0), (900.0, 600.0));
        let (w, h) = surface_pt(100, 50, 0.0);
        assert!(w.is_finite() && h.is_finite(), "a zero scale divides by nothing");
    }

    /// A copy that did not come through a window's own input (the control socket, a menu item) stayed in the window
    /// that made it until the next keyboard copy.
    #[test]
    fn a_copy_made_on_any_way_reaches_every_window() {
        let mut main = two_windows();
        // the main window copies outside `event` (what a control call or a menu item does)
        main.selection = vec![0];
        main.copy_selected(false);
        assert_eq!(main.windows[0].files.clip, None, "not shared yet");
        main.share_clips();
        let copied = Some((vec![PathBuf::from("/fixture/a")], false));
        assert_eq!(main.windows[0].files.clip, copied);
        // nothing new: a second look changes nothing, and a window's own later state is not overwritten
        main.windows[0].files.clip = None;
        main.share_clips();
        assert_eq!(main.windows[0].files.clip, None);
        // an extra window cuts: the main window and the others take it
        main.windows.push(ExtraWindow::new(WindowId(5), Box::new(fixture(vec![]))));
        main.windows[0].files.selection = vec![1];
        main.windows[0].files.copy_selected(true);
        main.share_clips();
        let cut = Some((vec![PathBuf::from("/fixture/b")], true));
        assert_eq!(main.clip, cut);
        assert_eq!(main.windows[1].files.clip, cut);
    }

    #[test]
    fn cmd_w_closes_an_extra_window_that_has_no_tab_to_close() {
        assert!(closes_window(true, 1, false), "a Get Info window");
        assert!(closes_window(true, 3, true));
        assert!(closes_window(false, 1, false), "a browser window with its one tab");
        assert!(!closes_window(false, 2, false), "with tabs the key closes the open tab");
        assert!(!closes_window(false, 1, true), "not while a name is typed");
    }

    #[test]
    fn windows_an_extra_window_opened_join_the_main_list() {
        let mut main = two_windows();
        main.clip = Some((vec![PathBuf::from("/fixture/a")], false));
        main.windows[0].files.windows.push(ExtraWindow::new(WindowId(7), Box::new(fixture(vec![]))));
        main.adopt_opened(0);
        assert!(main.windows[0].files.windows.is_empty());
        assert_eq!(main.windows.iter().map(|e| e.id).collect::<Vec<_>>(), vec![WindowId(4), WindowId(7)]);
        assert_eq!(main.windows[1].files.clip, main.clip, "the new window pastes what the others do");
        main.adopt_opened(9);
    }

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
