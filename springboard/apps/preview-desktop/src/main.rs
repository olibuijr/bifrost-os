//! Viewer, a document window for images and PDFs: zoom and fit, rotation, page navigation with a
//! thumbnails sidebar, markup (highlight, note, rectangle, rounded rectangle) saved beside the app's
//! data, Adjust Color through the image filter API, and PNG export. Pages and drawing live in
//! `document` and `canvas`; the pure state and maths live in `model`.

mod canvas;
mod document;
mod model;

use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use canvas::{Action, Layout, PictureCache, Scene, Tool};
use document::{Body, Document};
use model::{Adjust, Mark, MarkKind, Norm, View, Zoom, HIGHLIGHT, MARK_BLUE};
use std::path::{Path, PathBuf};

/// A pointer gesture in progress. The coordinates are canvas or sidebar pixels.
enum Gesture {
    /// Dragging the picture or the text.
    Pan { last: (f32, f32) },
    /// Dragging the thumbnails sidebar up or down.
    Scroll { last: f32 },
    /// Dragging out a highlight or a shape.
    Drag { start: (f32, f32), now: (f32, f32), kind: MarkKind },
}

struct Viewer {
    data_dir: PathBuf,
    doc: Option<Document>,
    view: View,
    tool: Tool,
    adjust: Adjust,
    gesture: Option<Gesture>,
    /// The index of the note whose text the keyboard is typing.
    editing: Option<usize>,
    /// The last message: an open error or an export result.
    status: String,
    cache: Option<PictureCache>,
}

impl Viewer {
    fn new() -> Viewer {
        Viewer {
            data_dir: PathBuf::new(),
            doc: None,
            view: View::default(),
            tool: Tool::Pan,
            adjust: Adjust::default(),
            gesture: None,
            editing: None,
            status: String::new(),
            cache: None,
        }
    }

    fn open_path(&mut self, path: &Path) {
        self.stop_editing();
        match document::open(path, &self.data_dir) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.view = View::default();
                self.cache = None;
                self.status.clear();
            }
            Err(e) => {
                self.doc = None;
                self.status = format!("Could not open {}: {e}", path.display());
            }
        }
    }

    fn save(&self) {
        if let Some(doc) = &self.doc {
            let _ = document::save_marks(&self.data_dir, &doc.path, &doc.marks);
        }
    }

    /// The picture geometry for the current page and view, or `None` for a text page or no document.
    fn placed(&self, lay: &Layout) -> Option<canvas::Placed> {
        let doc = self.doc.as_ref()?;
        let page = doc.pages.get(self.view.page)?;
        let Body::Picture(img) = &page.body else { return None };
        let (cx, cy, cw, ch) = lay.canvas();
        let (rw, rh) = model::turn_size(img.w, img.h, self.view.quarters);
        let scale = self.view.zoom.scale(rw, rh, cw as usize, ch as usize);
        Some(canvas::place((cx, cy, cw, ch), rw, rh, scale, self.view.pan))
    }

    /// The mark a drag in progress would make.
    fn draft(&self, lay: &Layout) -> Option<Mark> {
        let Some(Gesture::Drag { start, now, kind }) = &self.gesture else { return None };
        let p = self.placed(lay)?;
        let q = self.view.quarters;
        let a = canvas::to_source(&p, q, start.0, start.1);
        let b = canvas::to_source(&p, q, now.0, now.1);
        Some(Mark { kind: *kind, page: self.view.page, rect: Norm::from_corners(a, b), color: color_for(*kind), text: String::new() })
    }

    /// The size the zoom steps work from: the turned picture, or the canvas for text.
    fn page_dims(&self, lay: &Layout) -> (usize, usize) {
        let (_, _, cw, ch) = lay.canvas();
        let picture = self.doc.as_ref().and_then(|d| d.pages.get(self.view.page)).and_then(|p| match &p.body {
            Body::Picture(img) => Some(model::turn_size(img.w, img.h, self.view.quarters)),
            Body::Text(_) => None,
        });
        picture.unwrap_or((cw as usize, ch as usize))
    }

    fn scroll_max(&self, ui: &Ui, lay: &Layout) -> f32 {
        let Some(doc) = &self.doc else { return 0.0 };
        (canvas::rows_height(&canvas::rows(ui, doc)) - (lay.h - lay.bar)).max(0) as f32
    }

    fn go_page(&mut self, page: usize) {
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        if self.view.go(page, count) {
            self.stop_editing();
        }
    }

    /// Ends the note being typed. A note left empty is removed.
    fn stop_editing(&mut self) {
        let Some(i) = self.editing.take() else { return };
        if let Some(doc) = &mut self.doc {
            if doc.marks.get(i).is_some_and(|m| m.text.is_empty()) {
                doc.marks.remove(i);
            }
        }
        self.save();
    }

    fn act(&mut self, ui: &Ui, lay: &Layout, action: Action) -> bool {
        match action {
            Action::Previous | Action::Next => {
                let delta = if action == Action::Previous { -1 } else { 1 };
                let count = self.doc.as_ref().map_or(0, |d| d.count());
                if count > 0 && self.view.step(delta, count) {
                    self.stop_editing();
                }
            }
            Action::ZoomIn | Action::ZoomOut => {
                let (w, h) = self.page_dims(lay);
                let (_, _, cw, ch) = lay.canvas();
                self.view.zoom = self.view.zoom.stepped(w, h, cw as usize, ch as usize, action == Action::ZoomIn);
            }
            Action::ActualSize => self.view.zoom = Zoom::Factor(1.0),
            Action::FitWindow => self.view.zoom = Zoom::Fit,
            Action::RotateLeft => self.view.turn(-1),
            Action::RotateRight => self.view.turn(1),
            Action::UseTool(tool) => {
                self.stop_editing();
                self.tool = tool;
            }
            Action::Brighter => self.adjust.exposure = (self.adjust.exposure + 0.5).clamp(-2.0, 2.0),
            Action::Darker => self.adjust.exposure = (self.adjust.exposure - 0.5).clamp(-2.0, 2.0),
            Action::Saturate => self.adjust.saturation = (self.adjust.saturation + 0.25).clamp(0.0, 2.0),
            Action::Desaturate => self.adjust.saturation = (self.adjust.saturation - 0.25).clamp(0.0, 2.0),
            Action::ResetColour => self.adjust = Adjust::default(),
            Action::ExportPng => {
                if let Some(doc) = &self.doc {
                    self.status = match document::export_png(doc, self.view.page, self.view.quarters, &self.adjust) {
                        Ok(path) => format!("Exported {}", path.display()),
                        Err(e) => format!("Export failed: {e}"),
                    };
                }
            }
        }
        true
    }

    fn touch(&mut self, ui: &Ui, width: i32, height: i32, kind: TouchKind, x: f32, y: f32) -> bool {
        let lay = Layout::new(ui, width, height);
        match kind {
            TouchKind::Down => self.touch_down(ui, &lay, x, y),
            TouchKind::Move => self.touch_move(ui, &lay, x, y),
            TouchKind::Up | TouchKind::Cancel => self.touch_up(&lay, x, y),
        }
    }

    fn touch_down(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        if let Some(action) = canvas::button_hit(ui, lay, x, y) {
            return self.act(ui, lay, action);
        }
        if y < lay.bar as f32 {
            return false;
        }
        if x < lay.side as f32 {
            let Some(doc) = &self.doc else { return false };
            let rows = canvas::rows(ui, doc);
            if let Some(page) = canvas::sidebar_hit(lay, &rows, self.view.sidebar_scroll, x, y) {
                self.go_page(page);
            }
            self.gesture = Some(Gesture::Scroll { last: y });
            return true;
        }
        if self.doc.is_none() {
            return false;
        }
        self.stop_editing();
        let kind = self.tool.mark_kind();
        match kind {
            None => self.gesture = Some(Gesture::Pan { last: (x, y) }),
            Some(MarkKind::Note) => {
                if let Some(p) = self.placed(lay) {
                    let a = canvas::to_source(&p, self.view.quarters, x, y);
                    let rect = Norm { x: a.0, y: a.1, w: 0.0, h: 0.0 };
                    if let Some(doc) = &mut self.doc {
                        doc.marks.push(Mark { kind: MarkKind::Note, page: self.view.page, rect, color: MARK_BLUE, text: String::new() });
                        self.editing = Some(doc.marks.len() - 1);
                    }
                    self.save();
                }
            }
            Some(kind) => self.gesture = Some(Gesture::Drag { start: (x, y), now: (x, y), kind }),
        }
        true
    }

    fn touch_move(&mut self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> bool {
        match &mut self.gesture {
            Some(Gesture::Pan { last }) => {
                self.view.pan.0 += x - last.0;
                self.view.pan.1 += y - last.1;
                *last = (x, y);
                if let Some(p) = self.placed(lay) {
                    self.view.pan = p.pan;
                }
                true
            }
            Some(Gesture::Scroll { last }) => {
                let delta = y - *last;
                *last = y;
                let max = self.scroll_max(ui, lay);
                self.view.sidebar_scroll = (self.view.sidebar_scroll - delta).clamp(0.0, max);
                true
            }
            Some(Gesture::Drag { now, .. }) => {
                *now = (x, y);
                true
            }
            None => false,
        }
    }

    fn touch_up(&mut self, lay: &Layout, x: f32, y: f32) -> bool {
        let Some(gesture) = self.gesture.take() else { return false };
        if let Gesture::Drag { start, kind, .. } = gesture {
            let moved = (x - start.0).abs() + (y - start.1).abs();
            if let Some(p) = self.placed(lay) {
                let q = self.view.quarters;
                let a = canvas::to_source(&p, q, start.0, start.1);
                let b = canvas::to_source(&p, q, x, y);
                let rect = Norm::from_corners(a, b);
                if moved >= 4.0 && rect.w > 0.0 && rect.h > 0.0 {
                    let page = self.view.page;
                    if let Some(doc) = &mut self.doc {
                        doc.marks.push(Mark { kind, page, rect, color: color_for(kind), text: String::new() });
                    }
                    self.save();
                }
            }
        }
        true
    }

    fn key(&mut self, ctx: &Ctx, key: KeyInput) -> bool {
        match &key {
            KeyInput::Text(text) => {
                let Some(i) = self.editing else { return false };
                if let Some(m) = self.doc.as_mut().and_then(|d| d.marks.get_mut(i)) {
                    m.text.push_str(text);
                }
                true
            }
            KeyInput::Code(8) => {
                let Some(i) = self.editing else { return false };
                if let Some(m) = self.doc.as_mut().and_then(|d| d.marks.get_mut(i)) {
                    m.text.pop();
                }
                true
            }
            KeyInput::Code(13) | KeyInput::Code(27) => {
                if self.editing.is_none() {
                    return false;
                }
                self.stop_editing();
                true
            }
            KeyInput::Code(37) => self.key_page(ctx, Action::Previous),
            KeyInput::Code(39) => self.key_page(ctx, Action::Next),
            _ => false,
        }
    }

    fn key_page(&mut self, ctx: &Ctx, action: Action) -> bool {
        let (w, h) = self.size(ctx);
        let lay = Layout::new(&ctx.ui, w, h);
        self.act(&ctx.ui, &lay, action)
    }

    /// The window size the last frame was drawn at. Layout is recomputed from it for input.
    fn size(&self, ctx: &Ctx) -> (i32, i32) {
        (ctx.cfg.w as i32, ctx.cfg.h as i32)
    }
}

/// The colour a mark of this kind is drawn in.
fn color_for(kind: MarkKind) -> u32 {
    match kind {
        MarkKind::Highlight => HIGHLIGHT,
        _ => MARK_BLUE,
    }
}

/// The local path for an open request: a `file:` URL or an absolute path.
fn path_from_url(url: &str) -> Option<PathBuf> {
    if url.starts_with("file:") {
        return url::Url::parse(url).ok()?.to_file_path().ok();
    }
    let path = Path::new(url);
    path.is_absolute().then(|| path.to_path_buf())
}

fn item(action: Action, enabled: bool, checked: bool) -> AppMenuItem {
    let mut m = AppMenuItem::new(action.id(), action.label()).enabled(enabled).checked(checked);
    m = match action {
        Action::ZoomIn => m.key("cmd+="),
        Action::ZoomOut => m.key("cmd+-"),
        Action::ActualSize => m.key("cmd+0"),
        Action::FitWindow => m.key("cmd+9"),
        Action::RotateLeft => m.key("cmd+l"),
        Action::RotateRight => m.key("cmd+r"),
        Action::ExportPng => m.key("cmd+shift+e"),
        _ => m,
    };
    m
}

impl App for Viewer {
    fn id(&self) -> &str {
        "is.olibuijr.preview.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // the app's private storage holds the markup sidecars
        if self.data_dir.as_os_str().is_empty() {
            self.data_dir = ctx.data_dir.clone();
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let open = self.doc.is_some();
        let count = self.doc.as_ref().map_or(0, |d| d.count());
        let picture = self.doc.as_ref().is_some_and(|d| matches!(d.pages.get(self.view.page).map(|p| &p.body), Some(Body::Picture(_))));
        let tool_items: Vec<AppMenuItem> = [Tool::Pan, Tool::Highlight, Tool::Note, Tool::Rect, Tool::Rounded]
            .into_iter()
            .map(|t| item(Action::UseTool(t), open, self.tool == t))
            .collect();
        vec![
            AppMenu { title: "File".into(), items: vec![item(Action::ExportPng, picture, false)] },
            AppMenu {
                title: "View".into(),
                items: vec![
                    item(Action::ZoomIn, open, false),
                    item(Action::ZoomOut, open, false),
                    item(Action::ActualSize, open, self.view.zoom == Zoom::Factor(1.0)),
                    item(Action::FitWindow, open, self.view.zoom == Zoom::Fit),
                    AppMenuItem::separator(),
                    item(Action::RotateLeft, picture, false),
                    item(Action::RotateRight, picture, false),
                    AppMenuItem::separator(),
                    item(Action::Previous, count > 1 && self.view.page > 0, false),
                    item(Action::Next, count > 1 && self.view.page + 1 < count, false),
                ],
            },
            AppMenu { title: "Tools".into(), items: tool_items },
            AppMenu {
                title: "Adjust".into(),
                items: vec![
                    item(Action::Brighter, open, false),
                    item(Action::Darker, open, false),
                    item(Action::Saturate, open, false),
                    item(Action::Desaturate, open, false),
                    item(Action::ResetColour, open && !self.adjust.is_identity(), false),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(action) = Action::from_id(id) else { return false };
        let lay = Layout::new(&ctx.ui, ctx.cfg.w as i32, ctx.cfg.h as i32);
        self.act(&ctx.ui, &lay, action)
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Open(url) => match path_from_url(&url) {
                Some(path) => {
                    self.open_path(&path);
                    true
                }
                None => false,
            },
            Input::Key(key) => self.key(ctx, key),
            Input::Touch { kind, x, y, .. } => {
                let (w, h) = self.size(ctx);
                self.touch(&ctx.ui, w, h, kind, x, y)
            }
            _ => false,
        }
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        self.stop_editing();
        self.save();
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let lay = Layout::new(&ctx.ui, s.w as i32, s.h as i32);
        let draft = self.draft(&lay);
        let scene = Scene {
            doc: self.doc.as_ref(),
            view: self.view,
            tool: self.tool,
            adjust: self.adjust,
            draft,
            editing: self.editing,
            status: &self.status,
        };
        canvas::draw(s, &ctx.ui, &lay, &scene, &mut self.cache);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(Viewer::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_requests_take_file_urls_and_absolute_paths_only() {
        assert_eq!(path_from_url("/tmp/a.png"), Some(PathBuf::from("/tmp/a.png")));
        assert_eq!(path_from_url("file:///tmp/a.pdf"), Some(PathBuf::from("/tmp/a.pdf")));
        assert_eq!(path_from_url("relative/a.png"), None);
    }

    #[test]
    fn highlight_and_shapes_have_their_colours() {
        assert_eq!(color_for(MarkKind::Highlight), HIGHLIGHT);
        assert_eq!(color_for(MarkKind::Rect), MARK_BLUE);
    }
}
