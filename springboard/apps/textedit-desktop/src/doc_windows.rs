//! One window per document, as the reference editor keeps them: File > New, Duplicate and a shared text open a second
//! window and leave the first as it was; a file opened while the window in front holds an untouched Untitled takes that
//! window's place (E2E8 10: Cmd+N used to show "Untitled 2" in the same window and hide the first document).
//!
//! The app has one set of page state (scroll, selection drag, find bar, layout cache) and the document controller's
//! active document. That set belongs to the window being handled (`TextEditApp::current`); the other windows keep
//! theirs parked in a [`View`]. Every entry point for a window calls [`TextEditApp::enter`] first, which parks the
//! current state and loads that window's, and makes its document the active one, so the editing code is unchanged.
//! [`TextEditApp::sync_windows`] runs after anything that may add or close a document and gives each document exactly
//! one window: it opens windows for new documents and closes the ones whose document closed.
use super::*;
use bifrost_app::app_windows::{WindowId, WindowSpec};

/// The page state of a window that is not being handled, and the document it shows.
pub(crate) struct View {
    pub(crate) window: WindowId,
    pub(crate) doc: Option<DocId>,
    scroll: f32,
    follow: bool,
    drag: bool,
    drag_unit: select::Unit,
    drag_origin: Range<usize>,
    clicks: select::Clicks,
    cache: Option<Cache>,
    find_open: bool,
    find: String,
    replace: String,
    focus: Focus,
    find_sel: bool,
    note: Option<String>,
    size: Option<(u32, u32)>,
}

impl View {
    /// A new window's state: the page at its top, the find bar closed.
    fn fresh(window: WindowId, doc: DocId, size: Option<(u32, u32)>) -> Self {
        View {
            window,
            doc: Some(doc),
            scroll: 0.0,
            follow: true,
            drag: false,
            drag_unit: select::Unit::Char,
            drag_origin: 0..0,
            clicks: select::Clicks::default(),
            cache: None,
            find_open: false,
            find: String::new(),
            replace: String::new(),
            focus: Focus::Body,
            find_sel: false,
            note: None,
            size,
        }
    }
}

/// Swaps the app's page state with a parked view's (the window and the document are the caller's).
fn swap_state(app: &mut TextEditApp, v: &mut View) {
    std::mem::swap(&mut app.scroll, &mut v.scroll);
    std::mem::swap(&mut app.follow, &mut v.follow);
    std::mem::swap(&mut app.drag, &mut v.drag);
    std::mem::swap(&mut app.drag_unit, &mut v.drag_unit);
    std::mem::swap(&mut app.drag_origin, &mut v.drag_origin);
    std::mem::swap(&mut app.clicks, &mut v.clicks);
    std::mem::swap(&mut app.cache, &mut v.cache);
    std::mem::swap(&mut app.find_open, &mut v.find_open);
    std::mem::swap(&mut app.find, &mut v.find);
    std::mem::swap(&mut app.replace, &mut v.replace);
    std::mem::swap(&mut app.focus, &mut v.focus);
    std::mem::swap(&mut app.find_sel, &mut v.find_sel);
    std::mem::swap(&mut app.note, &mut v.note);
    std::mem::swap(&mut app.size, &mut v.size);
}

/// How a document that has no window yet is placed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// in a window of its own (New, Duplicate, shared text, a document left without a window)
    NewWindow,
    /// in the window in front when that holds an untouched Untitled, else in a window of its own (a file opened)
    ReplaceUntouched,
}

impl TextEditApp {
    /// The window being handled, in px: the main window's is the configured size, another's the size it was drawn at.
    pub(crate) fn px_size(&self, ctx: &Ctx) -> (u32, u32) {
        self.size.unwrap_or((ctx.cfg.w, ctx.cfg.h))
    }

    /// The regions of the window being handled.
    pub(crate) fn geom(&self, ctx: &Ctx) -> view::Geometry {
        let (w, h) = self.px_size(ctx);
        self.geometry(&ctx.ui, w, h)
    }

    /// Makes `window` the one being handled: the current state is parked, the window's loaded, and its document becomes
    /// the active one. A window this app does not know is ignored.
    pub(crate) fn enter(&mut self, window: WindowId) {
        if window == self.current {
            return;
        }
        let Some(i) = self.views.iter().position(|v| v.window == window) else { return };
        let mut v = self.views.remove(i);
        let doc = v.doc;
        swap_state(self, &mut v);
        v.window = self.current;
        v.doc = self.current_doc;
        self.views.push(v);
        self.current = window;
        self.current_doc = doc;
        if let Some(d) = doc {
            self.docs.set_active(d);
        }
    }

    /// Loads the parked main window and drops the state of the window being handled (it closed).
    fn leave_to_main(&mut self) {
        let Some(i) = self.views.iter().position(|v| v.window.is_main()) else { return };
        let mut v = self.views.remove(i);
        swap_state(self, &mut v);
        self.current = WindowId::MAIN;
        self.current_doc = v.doc;
        if let Some(d) = v.doc {
            self.docs.set_active(d);
        }
    }

    /// The page of the window being handled starts at the top of a document it now shows.
    fn reset_page(&mut self) {
        self.scroll = 0.0;
        self.follow = true;
        self.revision += 1;
    }

    /// Whether a document is an Untitled nobody has typed in (a file opened next takes its window).
    fn untouched(&self, id: DocId) -> bool {
        self.docs.file_name(id).is_none() && !self.docs.is_edited(id) && self.docs.get(id).is_some_and(|d| d.text.char_count() == 0)
    }

    fn shown(&self, id: DocId) -> bool {
        self.current_doc == Some(id) || self.views.iter().any(|v| v.doc == Some(id))
    }

    /// Opens a window for `doc`, cascaded from this one and as large, and makes it the one being handled. False when the
    /// shell's window limit is reached.
    fn open_doc_window(&mut self, ctx: &Ctx, doc: DocId) -> bool {
        let (w, h) = self.px_size(ctx);
        let title = self.docs.window_title(doc).unwrap_or_else(|| "Untitled".into());
        // full-size content, as the main window (`window = "full-size"`): the page draws its own title row under the
        // lights, so the shell adds none (E2E9 4: second windows showed the title twice)
        let spec = WindowSpec::new(&title, w as f32 / ctx.ui.s, h as f32 / ctx.ui.s).cascaded().full_size_content();
        let Ok(window) = ctx.open_window(spec) else { return false };
        let mut v = View::fresh(window, doc, Some((w, h)));
        swap_state(self, &mut v);
        v.window = self.current;
        v.doc = self.current_doc;
        self.views.push(v);
        self.current = window;
        self.current_doc = Some(doc);
        self.docs.set_active(doc);
        true
    }

    /// Opens a window for `doc` and leaves the window being handled as it is (a document restored without a window).
    fn open_parked_window(&mut self, ctx: &Ctx, doc: DocId) -> bool {
        let (current, current_doc) = (self.current, self.current_doc);
        if !self.open_doc_window(ctx, doc) {
            return false;
        }
        self.enter(current);
        debug_assert_eq!(self.current_doc, current_doc);
        true
    }

    /// Places a document that has no window yet.
    fn place(&mut self, ctx: &Ctx, doc: DocId, how: Place) {
        if self.docs.get(doc).is_none() {
            return;
        }
        if self.current_doc == Some(doc) {
            return;
        }
        // a file opened again: its window comes forward instead of a second copy
        let parked = self.views.iter().find(|v| v.doc == Some(doc)).map(|v| v.window);
        if let Some(window) = parked {
            self.enter(window);
            return;
        }
        match self.current_doc {
            None => {
                self.current_doc = Some(doc);
                self.reset_page();
            }
            Some(old) if how == Place::ReplaceUntouched && self.untouched(old) => {
                self.current_doc = Some(doc);
                self.reset_page();
                self.docs.set_active(doc);
                // clean, so it closes at once
                let _ = self.docs.close(ctx, old);
            }
            Some(_) => {
                if !self.open_doc_window(ctx, doc) {
                    // no room for another window: the document shows here, as it did before windows
                    self.current_doc = Some(doc);
                    self.reset_page();
                }
            }
        }
    }

    /// Gives every document one window after documents were added or closed, and keeps the windows' titles.
    pub(crate) fn sync_windows(&mut self, ctx: &Ctx) {
        let notices = std::mem::take(&mut self.notices);
        // a window with no document yet shows the active one (the first document, or one made after all were closed)
        if self.current_doc.is_none() {
            let active = self.docs.active().filter(|a| !self.views.iter().any(|v| v.doc == Some(*a)));
            if let Some(a) = active {
                self.current_doc = Some(a);
                if notices.iter().any(|n| matches!(n, Notice::Created(d) | Notice::Opened(d) if *d == a)) {
                    self.reset_page();
                }
            }
        }
        // windows whose document closed
        if self.current_doc.is_some_and(|d| self.docs.get(d).is_none()) {
            self.current_doc = None;
        }
        for v in &mut self.views {
            if v.doc.is_some_and(|d| self.docs.get(d).is_none()) {
                v.doc = None;
            }
        }
        if !self.current.is_main() && self.current_doc.is_none() {
            let gone = self.current;
            self.leave_to_main();
            let _ = ctx.close_window(gone);
        }
        let mut closed = Vec::new();
        self.views.retain(|v| {
            let keep = v.window.is_main() || v.doc.is_some();
            if !keep {
                closed.push(v.window);
            }
            keep
        });
        for window in closed {
            let _ = ctx.close_window(window);
        }
        // the main window cannot close: with its document gone it takes over another window's
        if self.current.is_main() && self.current_doc.is_none() {
            if let Some(i) = self.views.iter().position(|v| !v.window.is_main()) {
                let mut v = self.views.remove(i);
                let size = self.size;
                swap_state(self, &mut v);
                self.size = size;
                self.current_doc = v.doc;
                let _ = ctx.close_window(v.window);
            }
        }
        // new documents
        for n in &notices {
            match n {
                Notice::Created(d) => self.place(ctx, *d, Place::NewWindow),
                Notice::Opened(d) => self.place(ctx, *d, Place::ReplaceUntouched),
                _ => {}
            }
        }
        // any other document without a window (the first one, or one whose window the shell closed while it was edited)
        for id in self.docs.ids() {
            if !self.shown(id) {
                if self.current_doc.is_none() {
                    self.current_doc = Some(id);
                    self.reset_page();
                } else if !self.open_parked_window(ctx, id) {
                    break;
                }
            }
        }
        if let Some(d) = self.current_doc {
            self.docs.set_active(d);
        }
        self.sync_titles(ctx);
        self.sync_text_state(ctx);
    }

    /// The shell's titles of the document windows follow their documents (renamed, saved, edited).
    pub(crate) fn sync_titles(&self, ctx: &Ctx) {
        let mut windows: Vec<(WindowId, DocId)> = self.views.iter().filter_map(|v| v.doc.map(|d| (v.window, d))).collect();
        if let Some(d) = self.current_doc {
            windows.push((self.current, d));
        }
        for (window, doc) in windows {
            if window.is_main() {
                continue;
            }
            let Some(title) = self.docs.window_title(doc) else { continue };
            let stale = ctx.windows().get(window).is_some_and(|w| w.title != title);
            if stale {
                let _ = ctx.set_window_title(window, &title);
            }
        }
    }

    /// The shell closed a document window: its document closes with it. An edited one is not closed: the document
    /// controller raises the close confirmation (Don't Save, Cancel, Save) as File > Close does, `sync_windows` gives the
    /// document its window back with the confirmation on it as a sheet, and the document (and that window) close only
    /// when it is answered with Don't Save, or with Save once saved. Cancel keeps both. The shell has already taken the
    /// window down when it says so, so the document comes back in a new window rather than staying in the old one.
    pub(crate) fn doc_window_closed(&mut self, ctx: &Ctx, window: WindowId) -> bool {
        if window.is_main() {
            return false;
        }
        let doc = if self.current == window {
            let doc = self.current_doc;
            self.leave_to_main();
            doc
        } else {
            match self.views.iter().position(|v| v.window == window) {
                Some(i) => self.views.remove(i).doc,
                None => return false,
            }
        };
        if let Some(d) = doc {
            let notices = self.docs.close(ctx, d);
            self.notices.extend(notices);
            if let Some(d) = self.current_doc {
                self.docs.set_active(d);
            }
        }
        self.sync_windows(ctx);
        // an edited document asks first: its new window comes forward with the confirmation on it
        if let Some(d) = doc.filter(|d| self.docs.alert_doc() == Some(*d)) {
            if let Some(w) = self.views.iter().find(|v| v.doc == Some(d)).map(|v| v.window) {
                self.enter(w);
            }
        }
        true
    }

    /// Draws `window` (not the main one) into `s` at its own size, then hands the state back to the key window.
    pub(crate) fn draw_doc_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface) -> Damage {
        self.enter(window);
        if self.current != window {
            return Damage::new(s.w, s.h);
        }
        self.size = Some((s.w as u32, s.h as u32));
        let d = self.paint(ctx, s);
        self.enter(ctx.key_window());
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::ToShell;

    fn window(w: u32, h: u32) -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = bifrost_app::Configure { w, h, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    /// E2E9 4: a document window draws its own title row, so it asks the shell for full-size content.
    #[test]
    fn a_second_window_asks_for_full_size_content() {
        let ctx = window(1200, 780);
        let mut app = TextEditApp::new();
        app.configure(&ctx);
        ctx.take_sent();
        assert!(app.menu_action(&ctx, item::NEW));
        let flags: Vec<u32> = ctx.take_sent().into_iter().filter_map(|m| if let ToShell::WindowOpen { flags, .. } = m { Some(flags) } else { None }).collect();
        assert_eq!(flags.len(), 1);
        assert_ne!(flags[0] & bifrost_app::proto::window_flags::FULL_SIZE, 0, "no shell title row over the page's own");
    }

    fn opened(ctx: &Ctx) -> Vec<String> {
        ctx.take_sent().into_iter().filter_map(|m| if let ToShell::WindowOpen { title, .. } = m { Some(title) } else { None }).collect()
    }

    /// E2E8 10: File > New with the empty Untitled in front keeps that window and opens "Untitled 2" in a second one.
    #[test]
    fn file_new_opens_a_second_window_and_keeps_the_first() {
        let ctx = window(1200, 780);
        let mut app = TextEditApp::new();
        app.configure(&ctx);
        let first = app.docs.active().expect("the window starts with a document");
        assert_eq!(app.current_doc, Some(first));
        ctx.take_sent();
        assert!(app.menu_action(&ctx, item::NEW));
        assert_eq!(opened(&ctx), ["Untitled 2"], "one new window, for the new document");
        assert_eq!(app.docs.len(), 2);
        let second = app.docs.active().unwrap();
        assert_ne!(second, first);
        assert_eq!(app.docs.display_name(second).as_deref(), Some("Untitled 2"));
        // the main window still shows the first document
        app.enter(WindowId::MAIN);
        assert_eq!(app.docs.active(), Some(first));
        assert_eq!(app.window_title(), "Untitled");
        assert_eq!(ctx.windows().list().len(), 2, "the main window and the new one");
    }

    /// Typing goes to the document of the window it is typed in; each window keeps its own page.
    #[test]
    fn each_window_edits_its_own_document() {
        let ctx = window(1200, 780);
        let mut app = TextEditApp::new();
        app.configure(&ctx);
        let first = app.docs.active().unwrap();
        app.menu_action(&ctx, item::NEW);
        let second = app.docs.active().unwrap();
        let w2 = app.current;
        assert!(!w2.is_main());
        app.window_event(&ctx, w2, Input::Key(KeyInput::Text("two".into())));
        app.window_event(&ctx, WindowId::MAIN, Input::Key(KeyInput::Text("one".into())));
        assert_eq!(app.docs.get(first).unwrap().text.text(), "one");
        assert_eq!(app.docs.get(second).unwrap().text.text(), "two");
    }

    /// Closing the second document closes its window; the first stays in the main window.
    #[test]
    fn closing_a_document_closes_its_window() {
        let ctx = window(1200, 780);
        let mut app = TextEditApp::new();
        app.configure(&ctx);
        let first = app.docs.active().unwrap();
        app.menu_action(&ctx, item::NEW);
        let w2 = app.current;
        ctx.take_sent();
        app.window_input(&ctx, w2, bifrost_app::proto::WindowInput::MenuAction(item::CLOSE));
        assert!(ctx.take_sent().iter().any(|m| matches!(m, ToShell::WindowClose { id } if *id == w2.0)), "its window closes");
        assert_eq!(app.docs.len(), 1);
        assert_eq!(app.current, WindowId::MAIN);
        assert_eq!(app.docs.active(), Some(first));
    }

    /// The red button on an edited document window does not lose the document: the close confirmation comes up on the
    /// document's new window, Cancel keeps it, Don't Save closes it and its window.
    #[test]
    fn the_red_button_on_an_edited_document_asks_before_closing() {
        let ctx = window(1200, 780);
        let mut app = TextEditApp::new();
        app.configure(&ctx);
        app.menu_action(&ctx, item::NEW);
        let w2 = app.current;
        let second = app.docs.active().unwrap();
        app.window_event(&ctx, w2, Input::Key(KeyInput::Text("unsaved".into())));
        assert!(app.docs.is_edited(second));
        let _ = ctx.close_window(w2);
        ctx.take_sent();
        assert!(app.window_closed(&ctx, w2));
        assert_eq!(app.docs.len(), 2, "the edited document stays open");
        assert_eq!(app.docs.alert_doc(), Some(second), "the close confirmation waits for an answer");
        let w3 = app.current;
        assert!(!w3.is_main() && w3 != w2, "the document is back in a window of its own");
        assert_eq!(app.current_doc, Some(second));
        assert!(app.close_alert_here());
        assert!(!app.type_into("x"), "typing does not reach the document behind the sheet");
        use bifrost_app::document::alert;
        // E2E9 6: the kit's alert sheet, hanging from the title bar, the whole name in its title, Save the default
        let sheet = app.close_sheet().expect("the confirmation is a sheet on this window");
        assert!(sheet.title.contains("Untitled 2"), "the name is not cut: {}", sheet.title);
        assert_eq!(sheet.default_button(), Some(alert::SAVE));
        assert_eq!(sheet.cancel_button(), Some(alert::CANCEL));
        // Cancel: the document and its window stay
        assert!(app.window_event(&ctx, w3, Input::Key(KeyInput::Code(27))));
        assert!(app.docs.alert_doc().is_none());
        assert_eq!(app.docs.len(), 2);
        assert!(!ctx.take_sent().iter().any(|m| matches!(m, ToShell::WindowClose { id } if *id == w3.0)));
        // File > Close asks again; Don't Save closes the document and its window
        app.window_input(&ctx, w3, bifrost_app::proto::WindowInput::MenuAction(item::CLOSE));
        assert_eq!(app.docs.alert_doc(), Some(second));
        assert_eq!(app.docs.len(), 2);
        app.window_input(&ctx, w3, bifrost_app::proto::WindowInput::MenuAction(Action::CloseAnswer(alert::DONT_SAVE as u8).id().unwrap()));
        assert_eq!(app.docs.len(), 1);
        assert!(ctx.take_sent().iter().any(|m| matches!(m, ToShell::WindowClose { id } if *id == w3.0)), "its window closes");
        assert_eq!(app.current, WindowId::MAIN);
    }

    /// The shell closing a clean document window closes its document.
    #[test]
    fn the_red_button_on_a_document_window_closes_its_document() {
        let ctx = window(1200, 780);
        let mut app = TextEditApp::new();
        app.configure(&ctx);
        app.menu_action(&ctx, item::NEW);
        let w2 = app.current;
        let _ = ctx.close_window(w2);
        assert!(app.window_closed(&ctx, w2));
        assert_eq!(app.docs.len(), 1);
        assert_eq!(app.current, WindowId::MAIN);
        assert!(app.views.is_empty());
    }
}
