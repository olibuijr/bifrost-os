//! New Files Window (⌘N, or File > New Window in the empty-area menu): a second Files window with its own folder
//! view. The window is a whole `Files` of its own, so its folder, selection and view never move the main window's.
//! A new window takes its touches and, while it is the key window, the keys, the Edit commands and the File menu
//! (`App::window_event`, `window_input`, `menus_for` in main.rs); it draws, and is dropped when the shell closes it.
//! What one window copies or cuts, every window pastes (`share_clip`).
use crate::*;
use bifrost_app::app_windows::{WindowId, WindowSpec};

/// Size of a window opened with New Window from a window that has no size yet, in points.
const NEW_WINDOW_PT: (f32, f32) = (900.0, 600.0);

/// The size New Window asks for, in points: the size (`w`, `h`) of the window it is made from, as the reference file
/// manager's new window has the size of the one in front. (It asked for a fixed size, smaller than the first window.)
pub(crate) fn new_window_size(w: f32, h: f32) -> (f32, f32) {
    if w >= 1.0 && h >= 1.0 { (w, h) } else { NEW_WINDOW_PT }
}

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
    pub(crate) fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
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
        // not `App::draw`, which is the main window's: it would take this window for the key window whenever the
        // main one is (`App::draw_window` tells this window whether it has the keys)
        self.files.render(&ctx.ui, s)
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

/// An extra window's input as the handlers written for the main window take it: a touch and a secondary click arrive
/// in the window's points (`ToApp::WindowTouch`, `ToApp::Context` with a window) and are hit-tested in px.
pub(crate) fn window_input_in_px(ev: Input, px_per_pt: f32) -> Input {
    match ev {
        Input::Context { x, y } => Input::Context { x: x * px_per_pt, y: y * px_per_pt },
        other => bifrost_app::app_windows::window_touch_in_px(other, px_per_pt),
    }
}

/// Where a window asks the shell to open a menu (`Ctx::context_menu`) for the point (x, y) of its own points: the
/// main window names it in surface pixels, an extra window in its points (the shell reads the point in the units of
/// the window the pointer last used, as it sends that window's touches).
pub(crate) fn menu_point(extra_window: bool, x: f32, y: f32, px_per_pt: f32) -> (f32, f32) {
    if extra_window { (x, y) } else { (x * px_per_pt, y * px_per_pt) }
}

/// Whether ⌘W in an extra window closes the window: always a Get Info window, and a browser window that has no
/// second tab to close (with tabs, ⌘W closes the open tab) and no name being typed.
pub(crate) fn closes_window(info_window: bool, tabs: usize, typing: bool) -> bool {
    info_window || (tabs < 2 && !typing)
}

impl Files {
    /// New Window (⌘N): opens a window that shows the folder of the window it is made from (this one), in its view
    /// and at its size; the shell cascades it from the app's windows. Returns false when the shell refuses the window.
    pub(crate) fn new_window(&mut self, ctx: &Ctx) -> bool {
        let (w, h) = new_window_size(self.w, self.h);
        let Ok(id) = ctx.open_window(new_window_spec(w, h, bifrost_app::app_windows::shell_takes_window_chrome())) else { return false };
        let mut files = Box::new(Files::new());
        files.show_like(self);
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

    /// A window just made (`Files::new`, at the home folder in the default view) takes the folder and the view of the
    /// window it was opened from: the same folder with a history of its own, the view mode, the sort, the grouping,
    /// the sizes, and whether hidden files, the sidebar and the status bar show. (A new window opened at the home
    /// folder in the icon view, whatever the window in front showed.)
    pub(crate) fn show_like(&mut self, from: &Files) {
        self.grid = from.grid;
        self.columns = from.columns;
        self.sort = from.sort;
        self.show_hidden = from.show_hidden;
        self.sidebar_hidden = from.sidebar_hidden;
        self.status_bar = from.status_bar;
        self.vo.gallery = from.vo.gallery;
        self.vo.asc = from.vo.asc;
        self.vo.group = from.vo.group;
        self.vo.icon = from.vo.icon;
        self.vo.text = from.vo.text;
        self.vo.col_w = from.vo.col_w;
        if self.path != from.path {
            // the folder is the first of this window's history: Back does not lead to the home folder it never showed
            self.history.clear();
            self.at = 0;
            self.navigate(from.path.clone(), true);
        }
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

    fn ctx_at(scale: f32) -> Ctx {
        use bifrost_app::kit::{Font, Ui};
        let font = Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let cfg = bifrost_app::proto::Configure { w: 1920, h: 1260, scale, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(font, scale), cfg)
    }

    fn menu_sent(ctx: &Ctx) -> Option<(f32, f32, Vec<ContextItem>)> {
        ctx.take_sent().into_iter().find_map(|m| match m {
            bifrost_app::proto::ToShell::ContextMenu { x, y, items } => Some((x, y, items)),
            _ => None,
        })
    }

    #[test]
    fn a_menu_point_is_in_pixels_for_the_main_window_and_in_points_for_an_extra_one() {
        assert_eq!(menu_point(false, 100.0, 40.0, 2.0), (200.0, 80.0));
        assert_eq!(menu_point(true, 100.0, 40.0, 2.0), (100.0, 40.0));
        assert!(matches!(window_input_in_px(Input::Context { x: 100.0, y: 40.0 }, 2.0), Input::Context { x, y } if (x, y) == (200.0, 80.0)));
        assert!(matches!(window_input_in_px(Input::ContextAction(7), 2.0), Input::ContextAction(7)));
    }

    /// E2E7: a secondary click in a second Files window did nothing. It selects the item under the pointer in that
    /// window, opens that window's menu at the pointer, and the choice acts on that window's selection.
    #[test]
    fn a_secondary_click_in_a_second_window_opens_its_menu_and_the_choice_acts_there() {
        let ctx = ctx_at(2.0);
        let mut main = two_windows();
        main.windows[0].files.fx.extra_window = true;
        main.selection = vec![0];
        // a point on the second item of the second window, in its points
        let second = &main.windows[0].files;
        let at = (0..150).flat_map(|i| (0..100).map(move |j| (i as f32 * 6.0, j as f32 * 6.0))).find(|&(x, y)| second.item_at(x, y) == Some(1));
        let (x, y) = at.expect("the second item is in the window");
        assert!(main.window_event(&ctx, WindowId(4), Input::Context { x, y }));
        let (mx, my, items) = menu_sent(&ctx).expect("the window asks for its menu");
        assert_eq!((mx, my), (x, y), "at the pointer, in the window's points");
        assert!(items.iter().any(|i| i.id == MENU_COPY), "the item menu");
        assert_eq!(main.windows[0].files.selection, vec![1], "the item under the pointer, in that window");
        assert_eq!(main.selection, vec![0], "the main window's selection is its own");
        // the choice comes back for that window: Copy copies its item, not the main window's
        main.window_event(&ctx, WindowId(4), Input::ContextAction(MENU_COPY));
        assert_eq!(main.clip, Some((vec![PathBuf::from("/fixture/b")], false)), "what it copied every window pastes");
        // a choice for a window that is gone acts nowhere
        assert!(!main.window_event(&ctx, WindowId(9), Input::ContextAction(MENU_COPY)));
        // the same click in the main window is in pixels, and so is the point of its menu
        main.selection.clear();
        assert!(main.window_event(&ctx, WindowId::MAIN, Input::Context { x: x * 2.0, y: y * 2.0 }));
        let (mx, my, _) = menu_sent(&ctx).expect("the main window's menu");
        assert_eq!((mx, my), (x * 2.0, y * 2.0));
        assert_eq!(main.selection, vec![1]);
    }

    /// A toolbar pull-down opened in a second window opens under that window's button, and its choice sorts that
    /// window alone.
    #[test]
    fn a_toolbar_menu_of_a_second_window_opens_in_its_points_and_its_choice_acts_there() {
        let ctx = ctx_at(2.0);
        let mut main = two_windows();
        main.windows[0].files.fx.extra_window = true;
        main.windows[0].files.update_controls();
        main.update_controls();
        // where the menu was queued for: under the button, in the window's points
        let queued = |files: &Files| match files.sys.effects.last() {
            Some(Effect::PopUp(x, y, _)) => (*x, *y),
            _ => panic!("the Sort button queues its menu"),
        };
        main.windows[0].files.bar_action(BAR_SORT);
        let want = queued(&main.windows[0].files);
        main.windows[0].files.flush_services(&ctx);
        let (x, y, _) = menu_sent(&ctx).expect("the second window's Sort menu");
        assert_eq!((x, y), want, "an extra window names the point in its points");
        main.bar_action(BAR_SORT);
        let want = queued(&main);
        main.flush_services(&ctx);
        let (x, y, _) = menu_sent(&ctx).expect("the main window's Sort menu");
        assert_eq!((x, y), (want.0 * 2.0, want.1 * 2.0), "the main window in pixels");
        main.window_event(&ctx, WindowId(4), Input::ContextAction(MENU_SORT_SIZE));
        assert!(matches!(main.windows[0].files.sort, Sort::Size), "the window the menu was opened in");
        assert!(matches!(main.sort, Sort::Name), "the main window keeps its order");
    }

    /// E2E8 R12: press, hold and drag on a pull-down opened nothing. The press itself asks for the menu, in an extra
    /// window at a point of its own points, and the release that follows asks for nothing.
    #[test]
    fn a_press_on_a_pull_down_of_a_second_window_asks_for_its_menu_at_once() {
        let ctx = ctx_at(2.0);
        let mut main = two_windows();
        main.windows[0].files.fx.extra_window = true;
        main.windows[0].files.update_controls();
        let sort = main.windows[0].files.bar_layout().rect_of(BAR_SORT).unwrap();
        // an extra window's touches arrive in its points
        let touch = |kind: TouchKind| Input::Touch { kind, id: 0, x: sort.x + sort.w / 2., y: sort.y + sort.h / 2., t: 0., source: bifrost_app::TouchSource::Pointer, mods: bifrost_app::proto::ModifierFlags::EMPTY };
        assert!(main.window_event(&ctx, WindowId(4), touch(TouchKind::Down)));
        let (x, y, items) = menu_sent(&ctx).expect("the press asks for the Sort menu");
        assert_eq!(Some((x, y)), main.windows[0].files.bar_layout().menu_anchor(BAR_SORT, crate::barmenu::BAR_MENU_GAP), "under the button, in the window's points");
        assert_eq!(items.len(), main.windows[0].files.group_sort_items().len());
        assert!(main.windows[0].files.down.is_none(), "the press is not kept for a click");
        main.window_event(&ctx, WindowId(4), touch(TouchKind::Up));
        assert!(menu_sent(&ctx).is_none(), "a release that still reaches the window asks for no second menu");
        // the main window, in pixels
        main.update_controls();
        let sort = main.bar_layout().rect_of(BAR_SORT).unwrap();
        let touch = |kind: TouchKind| Input::Touch { kind, id: 0, x: (sort.x + sort.w / 2.) * 2.0, y: (sort.y + sort.h / 2.) * 2.0, t: 0., source: bifrost_app::TouchSource::Pointer, mods: bifrost_app::proto::ModifierFlags::EMPTY };
        assert!(main.window_event(&ctx, WindowId::MAIN, touch(TouchKind::Down)));
        let (x, y, _) = menu_sent(&ctx).expect("the main window's Sort menu");
        assert_eq!((x, y), (sort.x * 2.0, (sort.bottom() + crate::barmenu::BAR_MENU_GAP) * 2.0));
        main.window_event(&ctx, WindowId::MAIN, touch(TouchKind::Up));
        assert!(menu_sent(&ctx).is_none());
    }

    /// E2E8 R04a: File > New Window opened at the home folder, in the icon view, smaller than the window in front.
    #[test]
    fn a_new_window_takes_the_folder_the_view_and_the_size_of_the_window_it_is_made_from() {
        assert_eq!(new_window_size(960.0, 630.0), (960.0, 630.0), "the size of the window it is made from");
        assert_eq!(new_window_size(0.0, 0.0), NEW_WINDOW_PT, "a window that has no size yet");
        let root = std::env::temp_dir().join(format!("files-newwindow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let mut from = fixture(vec![]);
        from.path = root.join("sub");
        from.grid = false;
        from.columns = true;
        from.sort = Sort::Date;
        from.vo.asc = false;
        from.vo.col_w = 240.0;
        from.show_hidden = true;
        from.status_bar = true;
        // the new window as `Files::new` leaves it: at another folder, with that folder as its history
        let mut made = fixture(vec![]);
        made.home = root.clone();
        made.history = vec![PathBuf::from("/fixture")];
        made.show_like(&from);
        assert_eq!(made.path, root.join("sub"), "the folder of the window it was made from");
        assert_eq!((made.history.clone(), made.at), (vec![root.join("sub")], 0), "which is where its own history starts");
        assert!(made.columns && !made.grid && !made.vo.gallery, "its view");
        assert!(matches!(made.sort, Sort::Date) && !made.vo.asc && made.vo.col_w == 240.0, "its sort and sizes");
        assert!(made.show_hidden && made.status_bar && !made.sidebar_hidden);
        // the gallery is a view like the others; the same folder is not opened again
        from.columns = false;
        from.vo.gallery = true;
        made.show_like(&from);
        assert!(made.vo.gallery && !made.columns && !made.grid);
        assert_eq!(made.history.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
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
