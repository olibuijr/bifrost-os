//! `bifrost_app::canvas_view`: the drawing canvas an app embeds (`PKCanvasView`). It takes the app's touch frames
//! (a mouse or trackpad is one indirect touch, a pen tablet or a stylus a pencil touch) and the raw stylus reports of a
//! digitizer, turns them into strokes of a [`Drawing`] with the ink of the selected tool, and keeps undo, redo and the
//! lasso selection. The app asks it for the picture of the drawing ([`CanvasView::render_view`]) and for the saved
//! bytes ([`CanvasView::data_representation`]).
//!
//! ```ignore
//! impl App for Sketch {
//!     fn wants_touch_events(&self) -> bool { true }
//!     fn touches(&mut self, ctx: &Ctx, ev: &TouchEvent) -> bool {
//!         self.canvas.touches(&ev.in_view(ctx.cfg.scale, self.origin.0, self.origin.1))
//!     }
//! }
//! ```
//!
//! The tools come from the canvas's [`ToolPicker`] (`tool_picker_mut`): pen, marker and pencil ink, the two erasers and
//! the lasso. An ink stroke records each point's time, pressure (force), azimuth and altitude; the renderer widens a
//! pencil mark when the pen leans. The vector eraser removes whole strokes, the bitmap eraser cuts the parts it passes
//! over. The lasso selects the strokes it encloses; dragging inside the selection moves it.
//!
//! On the desktop the pointer draws with the mouse or trackpad and no finger-specific behaviour applies:
//! [`DrawingPolicy::Default`] accepts every input, and a finger is only refused by [`DrawingPolicy::PencilOnly`] (or by
//! the default policy on a tablet once a pencil has been seen, as on the reference tablet). A mouse or trackpad is
//! never refused. A pen tablet reports pressure and tilt through [`CanvasView::stylus`]; the compositor does not route
//! tablet-tool events to apps yet, so on the PC today the pen arrives as a pointer without pressure.
//!
//! Gaps against the reference canvas: no handwriting recogniser, no marker blend mode in the render, no scroll view
//! (the view keeps a zoom and an offset instead).

use bifrost_ui::cg::path::Affine;
use bifrost_ui::drawing::{Drawing, DrawingError, Stroke, StrokePoint};
use bifrost_ui::form_factor::FormFactor;
use bifrost_ui::lasso::{Lasso, Selection};
use bifrost_ui::stylus::{StylusProfile, StylusReport};
use bifrost_ui::tool_picker::{EraserKind, Tool, ToolPicker};
use bifrost_ui::touch::{Touch, TouchEvent, TouchPhase, TouchType};
use bifrost_ui::undo::UndoManager;
use bifrost_ui::Rect;
use std::rc::Rc;

pub use bifrost_ui::drawing::Raster;

/// The touch id of the stylus path ([`CanvasView::stylus`]); touch frames never use it.
pub const STYLUS_ID: u32 = u32::MAX;
/// Points one stroke keeps; a longer gesture stops adding points (the stroke still ends normally).
pub const MAX_STROKE_POINTS: usize = 20_000;
/// The largest saved drawing [`CanvasView::set_drawing_data`] reads, in bytes.
pub const MAX_DATA_BYTES: usize = 32 << 20;
/// Undo steps kept.
pub const UNDO_LEVELS: usize = 100;
/// The radius of both erasers, in drawing points, until [`CanvasView::set_eraser_radius`] changes it.
pub const DEFAULT_ERASER_RADIUS: f32 = 8.0;
/// Most erase steps one pointer move is split into, so a jump across the canvas still erases what it crossed.
const MAX_ERASE_STEPS: usize = 256;
/// How far outside the selection's box a press still grabs it, in view points.
const GRAB_SLOP: f32 = 8.0;
/// The zoom range.
pub const ZOOM_RANGE: (f32, f32) = (0.05, 32.0);

/// Which inputs draw (`PKCanvasView.drawingPolicy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawingPolicy {
    /// Every input draws, except that on a tablet a finger stops drawing once a pencil has been seen.
    #[default]
    Default,
    /// Every input draws, finger included.
    AnyInput,
    /// A finger does not draw. A pencil, a mouse and a trackpad do.
    PencilOnly,
}

/// Why saved drawing bytes could not be read.
#[derive(Clone, Debug, PartialEq)]
pub enum DataError {
    /// More than [`MAX_DATA_BYTES`] bytes.
    TooLarge,
    /// The bytes are not text.
    NotText,
    /// The text is not a saved drawing.
    Format(DrawingError),
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::TooLarge => write!(f, "the drawing is larger than {MAX_DATA_BYTES} bytes"),
            DataError::NotText => f.write_str("the drawing is not text"),
            DataError::Format(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DataError {}

/// One input sample in drawing coordinates.
#[derive(Clone, Copy, Debug)]
struct Sample {
    x: f32,
    y: f32,
    force: f32,
    azimuth: f32,
    altitude: f32,
    at_ms: f64,
}

impl Sample {
    fn point(&self, start_ms: f64) -> StrokePoint {
        let mut p = StrokePoint::at(self.x, self.y);
        p.time_offset = ((self.at_ms - start_ms) / 1000.0).max(0.0) as f32;
        p.force = if self.force.is_finite() { self.force.clamp(0.0, 1.0) } else { 0.0 };
        p.azimuth = self.azimuth;
        p.altitude = self.altitude;
        p
    }
}

/// The gesture in progress.
enum Gesture {
    Ink { id: u32, stroke: Stroke, start_ms: f64 },
    Erase { id: u32, kind: EraserKind, last: (f32, f32), before: Rc<Drawing>, changed: bool },
    Lasso { id: u32, lasso: Lasso },
    Move { id: u32, last: (f32, f32), before: Option<Rc<Drawing>> },
}

impl Gesture {
    fn id(&self) -> u32 {
        match self {
            Gesture::Ink { id, .. } | Gesture::Erase { id, .. } | Gesture::Lasso { id, .. } | Gesture::Move { id, .. } => *id,
        }
    }
}

/// The drawing canvas: strokes, the tool picker, undo and the lasso selection (see the module doc).
pub struct CanvasView {
    drawing: Drawing,
    picker: ToolPicker,
    policy: DrawingPolicy,
    form: FormFactor,
    pencil_seen: bool,
    gesture: Option<Gesture>,
    selection: Selection,
    undo: UndoManager<Drawing>,
    zoom: f32,
    offset: (f32, f32),
    eraser_radius: f32,
}

impl Default for CanvasView {
    fn default() -> Self {
        CanvasView::new()
    }
}

impl CanvasView {
    /// An empty canvas for this machine's form factor, with the default tool picker (the pen selected).
    pub fn new() -> Self {
        CanvasView::with_form_factor(FormFactor::current())
    }

    /// An empty canvas for `form`; tests and previews pass the form instead of reading the machine's.
    pub fn with_form_factor(form: FormFactor) -> Self {
        CanvasView {
            drawing: Drawing::new(),
            picker: ToolPicker::new(),
            policy: DrawingPolicy::Default,
            form,
            pencil_seen: false,
            gesture: None,
            selection: Selection::default(),
            undo: UndoManager::new(UNDO_LEVELS),
            zoom: 1.0,
            offset: (0.0, 0.0),
            eraser_radius: DEFAULT_ERASER_RADIUS,
        }
    }

    // --- content ---------------------------------------------------------------------------------------------------

    pub fn drawing(&self) -> &Drawing {
        &self.drawing
    }

    /// Replaces the drawing. The history, the selection and any gesture in progress are dropped.
    pub fn set_drawing(&mut self, drawing: Drawing) {
        self.drawing = drawing;
        self.gesture = None;
        self.selection = Selection::default();
        self.undo.clear();
    }

    /// The saved form of the drawing (`PKDrawing.dataRepresentation`): its text format as bytes.
    pub fn data_representation(&self) -> Vec<u8> {
        self.drawing.to_text().into_bytes()
    }

    /// Reads saved drawing bytes without touching any canvas (`PKDrawing(data:)`).
    pub fn drawing_from_data(data: &[u8]) -> Result<Drawing, DataError> {
        if data.len() > MAX_DATA_BYTES {
            return Err(DataError::TooLarge);
        }
        let text = std::str::from_utf8(data).map_err(|_| DataError::NotText)?;
        Drawing::from_text(text).map_err(DataError::Format)
    }

    /// Loads saved drawing bytes into the canvas. On an error the canvas is unchanged.
    pub fn set_drawing_data(&mut self, data: &[u8]) -> Result<(), DataError> {
        let drawing = CanvasView::drawing_from_data(data)?;
        self.set_drawing(drawing);
        Ok(())
    }

    // --- tools and policy ------------------------------------------------------------------------------------------

    pub fn tool_picker(&self) -> &ToolPicker {
        &self.picker
    }

    /// The picker the canvas draws with: select a tool or change an ink's colour and width through it.
    pub fn tool_picker_mut(&mut self) -> &mut ToolPicker {
        &mut self.picker
    }

    pub fn set_tool_picker(&mut self, picker: ToolPicker) {
        self.picker = picker;
    }

    pub fn drawing_policy(&self) -> DrawingPolicy {
        self.policy
    }

    pub fn set_drawing_policy(&mut self, policy: DrawingPolicy) {
        self.policy = policy;
    }

    /// Whether a touch of `kind` draws under the policy. A mouse or trackpad (indirect) is always accepted.
    pub fn accepts(&self, kind: TouchType) -> bool {
        let pencil_only = match self.policy {
            DrawingPolicy::PencilOnly => true,
            DrawingPolicy::AnyInput => false,
            DrawingPolicy::Default => self.form.is_tablet() && self.pencil_seen,
        };
        !pencil_only || kind != TouchType::Direct
    }

    pub fn eraser_radius(&self) -> f32 {
        self.eraser_radius
    }

    /// Sets the eraser radius in drawing points, kept between 0.5 and 100. A value that is not a number is ignored.
    pub fn set_eraser_radius(&mut self, radius: f32) {
        if radius.is_finite() {
            self.eraser_radius = radius.clamp(0.5, 100.0);
        }
    }

    // --- view ------------------------------------------------------------------------------------------------------

    pub fn zoom_scale(&self) -> f32 {
        self.zoom
    }

    /// Sets the zoom, kept inside [`ZOOM_RANGE`]; a value that is not a number is ignored.
    pub fn set_zoom_scale(&mut self, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        }
    }

    pub fn content_offset(&self) -> (f32, f32) {
        self.offset
    }

    /// The drawing point at the top left of the view.
    pub fn set_content_offset(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.offset = (x, y);
        }
    }

    /// A point of the view (points from its top left) as a point of the drawing.
    pub fn to_drawing(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.zoom + self.offset.0, y / self.zoom + self.offset.1)
    }

    /// A point of the drawing as a point of the view.
    pub fn to_view(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.offset.0) * self.zoom, (y - self.offset.1) * self.zoom)
    }

    /// The part of the drawing the view shows, for a view of `width` by `height` points.
    pub fn visible_rect(&self, width: f32, height: f32) -> Rect {
        Rect { x: self.offset.0, y: self.offset.1, w: width / self.zoom, h: height / self.zoom }
    }

    // --- rendering -------------------------------------------------------------------------------------------------

    /// The picture of `rect` (drawing coordinates) at `scale` pixels per point: the strokes and the stroke being drawn.
    pub fn render(&self, rect: Rect, scale: f32) -> Raster {
        let live = match &self.gesture {
            Some(Gesture::Ink { stroke, .. }) => std::slice::from_ref(stroke),
            _ => &[],
        };
        self.drawing.image_with(live, rect, scale)
    }

    /// The picture of what a view of `width` by `height` points shows, at `scale` device pixels per view point.
    pub fn render_view(&self, width: f32, height: f32, scale: f32) -> Raster {
        self.render(self.visible_rect(width, height), scale * self.zoom)
    }

    /// The stroke being drawn, for an app that draws it itself.
    pub fn active_stroke(&self) -> Option<&Stroke> {
        match &self.gesture {
            Some(Gesture::Ink { stroke, .. }) => Some(stroke),
            _ => None,
        }
    }

    /// The loop being drawn with the lasso, in drawing coordinates.
    pub fn lasso_path(&self) -> Option<&[(f32, f32)]> {
        match &self.gesture {
            Some(Gesture::Lasso { lasso, .. }) => Some(lasso.points()),
            _ => None,
        }
    }

    /// True while a stroke, an erase, a lasso or a move is in progress.
    pub fn is_drawing(&self) -> bool {
        self.gesture.is_some()
    }

    // --- input -----------------------------------------------------------------------------------------------------

    /// One frame of touches, in the view's points (see `TouchEvent::in_view`). True when the canvas needs a new frame.
    /// Only the first accepted touch draws; the others are ignored until it ends.
    pub fn touches(&mut self, ev: &TouchEvent) -> bool {
        let mut changed = false;
        for t in &ev.touches {
            changed |= self.touch(t, ev.timestamp);
        }
        changed
    }

    fn touch(&mut self, t: &Touch, now_ms: f64) -> bool {
        if t.kind == TouchType::Pencil {
            self.pencil_seen = true;
        }
        if !(t.x.is_finite() && t.y.is_finite()) {
            return false;
        }
        let (x, y) = self.to_drawing(t.x, t.y);
        let s = Sample { x, y, force: t.normalized_force().unwrap_or(0.0), azimuth: 0.0, altitude: 0.0, at_ms: now_ms };
        match t.phase {
            TouchPhase::Began => {
                if self.gesture.is_some() || !self.accepts(t.kind) {
                    return false;
                }
                self.begin(t.id, None, s)
            }
            TouchPhase::Moved => self.extend(t.id, s),
            TouchPhase::Ended => self.finish(t.id, Some(s)),
            TouchPhase::Cancelled => self.cancel(t.id),
            TouchPhase::Stationary => false,
        }
    }

    /// One report of a pen digitizer (`profile` says how to read its raw values): `x` and `y` are view points. A report
    /// with the tip down starts or extends a stroke, one without (hover, or a lift) ends it. The eraser end of the pen
    /// erases whole strokes whatever tool is selected. True when the canvas needs a new frame.
    pub fn stylus(&mut self, report: &StylusReport, profile: &StylusProfile, now_ms: f64) -> bool {
        self.pencil_seen = true;
        if !(report.x.is_finite() && report.y.is_finite()) {
            return false;
        }
        let Some(point) = report.to_point(profile) else {
            return self.finish(STYLUS_ID, None);
        };
        let (x, y) = self.to_drawing(report.x, report.y);
        let s = Sample { x, y, force: point.force, azimuth: point.azimuth, altitude: point.altitude, at_ms: now_ms };
        if self.gesture.as_ref().is_some_and(|g| g.id() == STYLUS_ID) {
            return self.extend(STYLUS_ID, s);
        }
        if self.gesture.is_some() {
            return false;
        }
        let tool = report.eraser.then_some(Tool::Eraser(EraserKind::Vector));
        self.begin(STYLUS_ID, tool, s)
    }

    /// Abandons the gesture in progress, as when the window loses the pointer: a stroke is dropped, an erase or a move
    /// is put back.
    pub fn cancel_all(&mut self) -> bool {
        match self.gesture.as_ref().map(Gesture::id) {
            Some(id) => self.cancel(id),
            None => false,
        }
    }

    fn begin(&mut self, id: u32, tool: Option<Tool>, s: Sample) -> bool {
        match tool.unwrap_or_else(|| self.picker.selected()) {
            Tool::Ink(_) => {
                let Some(ink) = self.picker.current_ink() else { return false };
                self.selection = Selection::default();
                self.gesture = Some(Gesture::Ink { id, stroke: Stroke { ink, points: vec![s.point(s.at_ms)] }, start_ms: s.at_ms });
                true
            }
            Tool::Eraser(kind) => {
                let before = Rc::new(self.drawing.clone());
                let changed = self.erase_at(kind, s.x, s.y);
                self.gesture = Some(Gesture::Erase { id, kind, last: (s.x, s.y), before, changed });
                true
            }
            Tool::Lasso => {
                if self.in_selection(s.x, s.y) {
                    self.gesture = Some(Gesture::Move { id, last: (s.x, s.y), before: None });
                } else {
                    self.selection = Selection::default();
                    let mut lasso = Lasso::new();
                    lasso.add(s.x, s.y);
                    self.gesture = Some(Gesture::Lasso { id, lasso });
                }
                true
            }
        }
    }

    fn extend(&mut self, id: u32, s: Sample) -> bool {
        let Some(mut g) = self.gesture.take() else { return false };
        if g.id() != id {
            self.gesture = Some(g);
            return false;
        }
        let changed = match &mut g {
            Gesture::Ink { stroke, start_ms, .. } => {
                let p = s.point(*start_ms);
                let same = stroke.points.last().is_some_and(|q| q.x == p.x && q.y == p.y);
                if same || stroke.points.len() >= MAX_STROKE_POINTS {
                    false
                } else {
                    stroke.points.push(p);
                    true
                }
            }
            Gesture::Erase { kind, last, changed, .. } => {
                let mut any = false;
                for (x, y) in erase_steps(*last, (s.x, s.y), self.eraser_radius * 0.5) {
                    any |= self.erase_at(*kind, x, y);
                }
                *last = (s.x, s.y);
                *changed |= any;
                any
            }
            Gesture::Lasso { lasso, .. } => {
                lasso.add(s.x, s.y);
                true
            }
            Gesture::Move { last, before, .. } => {
                let (dx, dy) = (s.x - last.0, s.y - last.1);
                if dx == 0.0 && dy == 0.0 {
                    false
                } else {
                    if before.is_none() {
                        *before = Some(Rc::new(self.drawing.clone()));
                    }
                    self.selection.transform(&mut self.drawing, Affine::translate(dx, dy));
                    *last = (s.x, s.y);
                    true
                }
            }
        };
        self.gesture = Some(g);
        changed
    }

    fn finish(&mut self, id: u32, last: Option<Sample>) -> bool {
        if let Some(s) = last {
            self.extend(id, s);
        }
        let Some(g) = self.gesture.take() else { return false };
        if g.id() != id {
            self.gesture = Some(g);
            return false;
        }
        match g {
            Gesture::Ink { stroke, .. } => {
                let kept = stroke.clone();
                self.undo.register("Draw", |d: &mut Drawing| drop(d.pop()), move |d: &mut Drawing| d.push(kept.clone()));
                self.drawing.push(stroke);
            }
            Gesture::Erase { before, changed, .. } => {
                if changed {
                    self.selection = Selection::default();
                    self.record("Erase", before);
                }
            }
            Gesture::Lasso { lasso, .. } => self.selection = lasso.select(&self.drawing),
            Gesture::Move { before, .. } => {
                if let Some(before) = before {
                    self.record("Move", before);
                }
            }
        }
        true
    }

    fn cancel(&mut self, id: u32) -> bool {
        let Some(g) = self.gesture.take() else { return false };
        if g.id() != id {
            self.gesture = Some(g);
            return false;
        }
        match g {
            Gesture::Erase { before, changed: true, .. } => self.drawing = (*before).clone(),
            Gesture::Move { before: Some(before), .. } => self.drawing = (*before).clone(),
            _ => {}
        }
        true
    }

    fn erase_at(&mut self, kind: EraserKind, x: f32, y: f32) -> bool {
        let radius = self.eraser_radius;
        let n = match kind {
            EraserKind::Bitmap => self.drawing.erase_points_at(x, y, radius),
            EraserKind::Vector => self.drawing.erase_strokes_at(x, y, radius),
        };
        n > 0
    }

    /// Registers an undo step that swaps the drawing between `before` and what it is now.
    fn record(&mut self, name: &str, before: Rc<Drawing>) {
        let after = Rc::new(self.drawing.clone());
        self.undo.register(name, move |d: &mut Drawing| *d = (*before).clone(), move |d: &mut Drawing| *d = (*after).clone());
    }

    fn in_selection(&self, x: f32, y: f32) -> bool {
        let slop = GRAB_SLOP / self.zoom;
        self.selection.bounds(&self.drawing).is_some_and(|b| x >= b.x - slop && x <= b.x + b.w + slop && y >= b.y - slop && y <= b.y + b.h + slop)
    }

    // --- selection -------------------------------------------------------------------------------------------------

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The box around the selected strokes, in drawing coordinates.
    pub fn selection_bounds(&self) -> Option<Rect> {
        self.selection.bounds(&self.drawing)
    }

    pub fn clear_selection(&mut self) {
        self.selection = Selection::default();
    }

    pub fn select_all(&mut self) {
        self.selection = Selection::from_indices((0..self.drawing.strokes().len()).collect());
    }

    /// Deletes the selected strokes as one undo step. False when nothing is selected or a gesture is running.
    pub fn delete_selection(&mut self) -> bool {
        if self.selection.is_empty() || self.gesture.is_some() {
            return false;
        }
        let before = Rc::new(self.drawing.clone());
        let kept: Vec<Stroke> = self.drawing.strokes().iter().enumerate().filter(|(i, _)| !self.selection.contains_stroke(*i)).map(|(_, s)| s.clone()).collect();
        self.drawing = Drawing::from_strokes(kept);
        self.selection = Selection::default();
        self.record("Delete", before);
        true
    }

    /// Moves the selected strokes by (`dx`, `dy`) drawing points as one undo step.
    pub fn move_selection(&mut self, dx: f32, dy: f32) -> bool {
        if !(dx.is_finite() && dy.is_finite()) {
            return false;
        }
        self.transform_selection("Move", Affine::translate(dx, dy))
    }

    /// Scales the selected strokes by `factor` about the centre of the selection, as one undo step. A factor that is
    /// not a positive number is refused.
    pub fn scale_selection(&mut self, factor: f32) -> bool {
        let (true, Some(b)) = (factor.is_finite() && factor > 0.0, self.selection_bounds()) else { return false };
        let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        self.transform_selection("Scale", Affine::translate(-cx, -cy).then(Affine::scale(factor, factor)).then(Affine::translate(cx, cy)))
    }

    fn transform_selection(&mut self, name: &str, t: Affine) -> bool {
        if self.selection.is_empty() || self.gesture.is_some() {
            return false;
        }
        let before = Rc::new(self.drawing.clone());
        self.selection.transform(&mut self.drawing, t);
        self.record(name, before);
        true
    }

    // --- undo ------------------------------------------------------------------------------------------------------

    pub fn can_undo(&self) -> bool {
        self.gesture.is_none() && self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.gesture.is_none() && self.undo.can_redo()
    }

    /// Undoes the latest change. The selection is dropped, as the strokes it named may be gone.
    pub fn undo(&mut self) -> bool {
        if self.gesture.is_some() {
            return false;
        }
        let done = self.undo.undo(&mut self.drawing).is_some();
        if done {
            self.selection = Selection::default();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        if self.gesture.is_some() {
            return false;
        }
        let done = self.undo.redo(&mut self.drawing).is_some();
        if done {
            self.selection = Selection::default();
        }
        done
    }

    /// The Edit menu titles: ("Undo Draw", "Redo Erase"), or plain "Undo" and "Redo" when there is nothing to name.
    pub fn undo_titles(&self) -> (String, String) {
        (self.undo.undo_menu_item_title(), self.undo.redo_menu_item_title())
    }

    /// Where the drawing is in its edit history: unequal values mean unsaved changes.
    pub fn change_count(&self) -> bifrost_ui::undo::ChangeCount {
        self.undo.change_count()
    }
}

/// The points an eraser visits going from `from` to `to`: every `step` along the segment (at most
/// [`MAX_ERASE_STEPS`]), ending at `to`. The start is not repeated, as the previous move erased it.
fn erase_steps(from: (f32, f32), to: (f32, f32), step: f32) -> Vec<(f32, f32)> {
    let len = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
    let n = if step > 0.0 && len.is_finite() { ((len / step).ceil() as usize).clamp(1, MAX_ERASE_STEPS) } else { 1 };
    (1..=n).map(|i| (from.0 + (to.0 - from.0) * i as f32 / n as f32, from.1 + (to.1 - from.1) * i as f32 / n as f32)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::stylus::LogicalRange;
    use bifrost_ui::tool_picker::EraserKind::{Bitmap, Vector};

    fn desktop() -> CanvasView {
        CanvasView::with_form_factor(FormFactor::Desktop)
    }

    fn touch(id: u32, phase: TouchPhase, kind: TouchType, x: f32, y: f32) -> Touch {
        Touch { id, phase, kind, x, y, prev_x: x, prev_y: y, timestamp: 0.0, tap_count: 1, force: None, max_force: None, major_radius: None }
    }

    fn frame(t: Touch, at_ms: f64) -> TouchEvent {
        TouchEvent { timestamp: at_ms, touches: vec![t] }
    }

    /// A whole pointer gesture: press, one move per point and release at the last one.
    fn drag(c: &mut CanvasView, kind: TouchType, points: &[(f32, f32)]) {
        let mut at = 0.0;
        let last = points.len() - 1;
        for (i, &(x, y)) in points.iter().enumerate() {
            let phase = if i == 0 { TouchPhase::Began } else if i == last { TouchPhase::Ended } else { TouchPhase::Moved };
            c.touches(&frame(touch(1, phase, kind, x, y), at));
            at += 16.0;
        }
    }

    fn mouse(c: &mut CanvasView, points: &[(f32, f32)]) {
        drag(c, TouchType::Indirect, points);
    }

    #[test]
    fn a_mouse_drag_draws_one_stroke_with_the_selected_ink() {
        let mut c = desktop();
        c.tool_picker_mut().set_color(Tool::Ink(bifrost_ui::drawing::InkKind::Pen), 0xff12_3456);
        mouse(&mut c, &[(10.0, 10.0), (20.0, 10.0), (30.0, 12.0), (40.0, 12.0)]);
        let strokes = c.drawing().strokes();
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].ink.color, 0xff12_3456);
        assert_eq!(strokes[0].points.len(), 4);
        let times: Vec<f32> = strokes[0].points.iter().map(|p| p.time_offset).collect();
        assert_eq!(times[0], 0.0);
        assert!(times.windows(2).all(|w| w[1] > w[0]), "each point is later than the one before: {times:?}");
        assert!(!c.is_drawing());
    }

    #[test]
    fn a_tap_leaves_a_dot_and_a_still_press_adds_no_duplicate_points() {
        let mut c = desktop();
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 5.0, 5.0), 0.0));
        c.touches(&frame(touch(1, TouchPhase::Moved, TouchType::Indirect, 5.0, 5.0), 10.0));
        c.touches(&frame(touch(1, TouchPhase::Ended, TouchType::Indirect, 5.0, 5.0), 20.0));
        assert_eq!(c.drawing().strokes().len(), 1);
        assert_eq!(c.drawing().strokes()[0].points.len(), 1);
    }

    #[test]
    fn the_stroke_in_progress_is_rendered_before_it_is_committed() {
        let mut c = desktop();
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 4.0, 10.0), 0.0));
        c.touches(&frame(touch(1, TouchPhase::Moved, TouchType::Indirect, 36.0, 10.0), 16.0));
        assert!(c.active_stroke().is_some());
        assert!(c.drawing().is_empty());
        let live = c.render_view(40.0, 20.0, 1.0);
        assert!(live.pixels.iter().any(|&p| p >> 24 != 0), "the live stroke is painted");
        c.touches(&frame(touch(1, TouchPhase::Ended, TouchType::Indirect, 36.0, 10.0), 32.0));
        assert!(c.active_stroke().is_none());
        assert_eq!(c.render_view(40.0, 20.0, 1.0), live, "committing changes nothing on screen");
    }

    #[test]
    fn pencil_pressure_and_tilt_are_recorded_and_a_mouse_has_none() {
        let mut c = desktop();
        let mut t = touch(1, TouchPhase::Began, TouchType::Pencil, 3.0, 3.0);
        t.force = Some(2.0);
        t.max_force = Some(4.0);
        c.touches(&frame(t, 0.0));
        t.phase = TouchPhase::Ended;
        c.touches(&frame(t, 5.0));
        assert_eq!(c.drawing().strokes()[0].points[0].force, 0.5);
        mouse(&mut c, &[(1.0, 1.0), (9.0, 9.0)]);
        assert_eq!(c.drawing().strokes()[1].points[0].force, 0.0, "no reading: the full width");
    }

    #[test]
    fn the_policy_decides_which_inputs_draw() {
        let mut desk = desktop();
        assert!(desk.accepts(TouchType::Direct) && desk.accepts(TouchType::Indirect) && desk.accepts(TouchType::Pencil));
        desk.set_drawing_policy(DrawingPolicy::PencilOnly);
        assert!(!desk.accepts(TouchType::Direct));
        assert!(desk.accepts(TouchType::Indirect), "a mouse or trackpad is never a finger");
        assert!(desk.accepts(TouchType::Pencil));
        desk.set_drawing_policy(DrawingPolicy::AnyInput);
        assert!(desk.accepts(TouchType::Direct));
    }

    #[test]
    fn on_a_tablet_the_default_policy_turns_fingers_off_once_a_pencil_is_seen() {
        let mut tab = CanvasView::with_form_factor(FormFactor::Tablet);
        assert!(tab.accepts(TouchType::Direct), "no pencil yet: the finger draws");
        drag(&mut tab, TouchType::Pencil, &[(0.0, 0.0), (5.0, 5.0)]);
        assert!(!tab.accepts(TouchType::Direct));
        let before = tab.drawing().strokes().len();
        drag(&mut tab, TouchType::Direct, &[(10.0, 10.0), (20.0, 20.0)]);
        assert_eq!(tab.drawing().strokes().len(), before, "the finger drew nothing");
        // the desktop never switches
        let mut desk = desktop();
        drag(&mut desk, TouchType::Pencil, &[(0.0, 0.0), (5.0, 5.0)]);
        assert!(desk.accepts(TouchType::Direct));
    }

    #[test]
    fn a_second_touch_is_ignored_while_the_first_draws() {
        let mut c = desktop();
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Direct, 0.0, 0.0), 0.0));
        c.touches(&frame(touch(2, TouchPhase::Began, TouchType::Direct, 50.0, 50.0), 1.0));
        c.touches(&frame(touch(2, TouchPhase::Moved, TouchType::Direct, 60.0, 60.0), 2.0));
        c.touches(&frame(touch(2, TouchPhase::Ended, TouchType::Direct, 60.0, 60.0), 3.0));
        assert!(c.is_drawing(), "the first touch still draws");
        c.touches(&frame(touch(1, TouchPhase::Ended, TouchType::Direct, 5.0, 5.0), 4.0));
        assert_eq!(c.drawing().strokes().len(), 1);
        assert!(c.drawing().strokes()[0].points.iter().all(|p| p.x <= 5.0));
    }

    #[test]
    fn a_hostile_location_or_a_cancel_draws_nothing() {
        let mut c = desktop();
        assert!(!c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, f32::NAN, 0.0), 0.0)));
        assert!(!c.is_drawing());
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 1.0, 1.0), 0.0));
        c.touches(&frame(touch(1, TouchPhase::Moved, TouchType::Indirect, f32::INFINITY, 3.0), 1.0));
        c.touches(&frame(touch(1, TouchPhase::Cancelled, TouchType::Indirect, 2.0, 2.0), 2.0));
        assert!(c.drawing().is_empty(), "a cancelled stroke is dropped");
        assert!(!c.can_undo());
    }

    #[test]
    fn a_stroke_stops_growing_at_the_point_limit() {
        let mut c = desktop();
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 0.0, 0.0), 0.0));
        for i in 1..(MAX_STROKE_POINTS + 50) {
            c.touches(&frame(touch(1, TouchPhase::Moved, TouchType::Indirect, i as f32, 0.0), i as f64));
        }
        c.touches(&frame(touch(1, TouchPhase::Ended, TouchType::Indirect, 0.5, 0.0), 1e6));
        assert_eq!(c.drawing().strokes()[0].points.len(), MAX_STROKE_POINTS);
    }

    #[test]
    fn undo_and_redo_walk_the_strokes_and_a_new_stroke_ends_the_redo_history() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (10.0, 0.0)]);
        mouse(&mut c, &[(0.0, 5.0), (10.0, 5.0)]);
        assert_eq!(c.undo_titles(), ("Undo Draw".to_string(), "Redo".to_string()));
        assert!(c.undo());
        assert_eq!(c.drawing().strokes().len(), 1);
        assert!(c.can_redo());
        assert!(c.redo());
        assert_eq!(c.drawing().strokes().len(), 2);
        assert!(c.undo() && c.undo());
        assert!(c.drawing().is_empty());
        assert!(!c.undo(), "nothing left");
        mouse(&mut c, &[(1.0, 1.0), (2.0, 2.0)]);
        assert!(!c.can_redo());
    }

    #[test]
    fn the_vector_eraser_removes_a_whole_stroke_in_one_undo_step() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (100.0, 0.0)]);
        mouse(&mut c, &[(0.0, 50.0), (100.0, 50.0)]);
        c.tool_picker_mut().select(Tool::Eraser(Vector));
        mouse(&mut c, &[(50.0, 2.0), (50.0, 3.0)]);
        assert_eq!(c.drawing().strokes().len(), 1);
        assert_eq!(c.drawing().strokes()[0].points[0].y, 50.0);
        assert!(c.undo());
        assert_eq!(c.drawing().strokes().len(), 2, "one step brings the stroke back");
        assert!(c.redo());
        assert_eq!(c.drawing().strokes().len(), 1);
    }

    #[test]
    fn an_erase_that_touches_nothing_leaves_no_undo_step() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (100.0, 0.0)]);
        c.tool_picker_mut().select(Tool::Eraser(Vector));
        mouse(&mut c, &[(50.0, 80.0), (60.0, 80.0)]);
        assert_eq!(c.drawing().strokes().len(), 1);
        assert_eq!(c.undo_titles().0, "Undo Draw", "the last step is still the stroke");
    }

    #[test]
    fn the_bitmap_eraser_cuts_a_stroke_in_two() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (100.0, 0.0)]);
        c.tool_picker_mut().select(Tool::Eraser(Bitmap));
        c.set_eraser_radius(6.0);
        mouse(&mut c, &[(50.0, 0.0), (50.0, 1.0)]);
        assert_eq!(c.drawing().strokes().len(), 2);
        assert!(c.undo());
        assert_eq!(c.drawing().strokes().len(), 1);
    }

    #[test]
    fn a_fast_erase_stroke_still_erases_what_it_crossed() {
        let mut c = desktop();
        mouse(&mut c, &[(50.0, -20.0), (50.0, 20.0)]);
        c.tool_picker_mut().select(Tool::Eraser(Vector));
        // two events only: the pointer jumps from far left to far right over the stroke
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 0.0, 0.0), 0.0));
        c.touches(&frame(touch(1, TouchPhase::Ended, TouchType::Indirect, 100.0, 0.0), 16.0));
        assert!(c.drawing().is_empty());
    }

    #[test]
    fn cancelling_an_erase_puts_the_strokes_back_without_a_history_step() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (100.0, 0.0)]);
        c.tool_picker_mut().select(Tool::Eraser(Vector));
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 50.0, 0.0), 0.0));
        assert!(c.drawing().is_empty());
        assert!(c.cancel_all());
        assert_eq!(c.drawing().strokes().len(), 1);
        assert_eq!(c.undo_titles().0, "Undo Draw");
    }

    fn lasso_around(c: &mut CanvasView, x0: f32, y0: f32, x1: f32, y1: f32) {
        c.tool_picker_mut().select(Tool::Lasso);
        mouse(c, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0 + 0.5)]);
    }

    #[test]
    fn the_lasso_selects_the_strokes_it_encloses_and_a_drag_inside_moves_them() {
        let mut c = desktop();
        mouse(&mut c, &[(10.0, 10.0), (20.0, 20.0)]);
        mouse(&mut c, &[(200.0, 200.0), (210.0, 210.0)]);
        lasso_around(&mut c, 0.0, 0.0, 30.0, 30.0);
        assert_eq!(c.selection().indices(), &[0]);
        assert!(c.selection_bounds().is_some());
        mouse(&mut c, &[(15.0, 15.0), (25.0, 20.0), (35.0, 25.0)]);
        let moved = &c.drawing().strokes()[0].points;
        assert_eq!((moved[0].x, moved[0].y), (30.0, 20.0), "moved by the drag's distance (20, 10)");
        assert_eq!(c.drawing().strokes()[1].points[0].x, 200.0, "the other stroke stayed");
        assert_eq!(c.undo_titles().0, "Undo Move");
        assert!(c.undo());
        assert_eq!(c.drawing().strokes()[0].points[0].x, 10.0);
    }

    #[test]
    fn a_lasso_press_outside_the_selection_starts_a_new_loop() {
        let mut c = desktop();
        mouse(&mut c, &[(10.0, 10.0), (20.0, 20.0)]);
        lasso_around(&mut c, 0.0, 0.0, 30.0, 30.0);
        assert!(!c.selection().is_empty());
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 300.0, 300.0), 0.0));
        assert!(c.selection().is_empty());
        assert!(c.lasso_path().is_some());
        c.cancel_all();
    }

    #[test]
    fn selected_strokes_can_be_deleted_moved_and_scaled_with_undo() {
        let mut c = desktop();
        mouse(&mut c, &[(10.0, 10.0), (30.0, 10.0)]);
        mouse(&mut c, &[(100.0, 100.0), (110.0, 100.0)]);
        c.select_all();
        assert!(c.scale_selection(2.0));
        let b = c.drawing().bounds().unwrap();
        assert!((b.w - 200.0).abs() < 1e-3, "twice as wide: {b:?}");
        assert!(c.undo());
        c.select_all();
        assert!(c.move_selection(5.0, 0.0));
        assert_eq!(c.drawing().strokes()[0].points[0].x, 15.0);
        assert!(!c.scale_selection(-1.0) && !c.scale_selection(f32::NAN) && !c.move_selection(f32::NAN, 0.0));
        c.select_all();
        assert!(c.delete_selection());
        assert!(c.drawing().is_empty());
        assert!(c.undo());
        assert_eq!(c.drawing().strokes().len(), 2);
        assert!(!c.delete_selection(), "the selection was dropped by the undo");
    }

    #[test]
    fn zoom_and_offset_map_the_view_to_the_drawing() {
        let mut c = desktop();
        c.set_zoom_scale(2.0);
        c.set_content_offset(10.0, 0.0);
        assert_eq!(c.to_drawing(20.0, 20.0), (20.0, 10.0));
        assert_eq!(c.to_view(20.0, 10.0), (20.0, 20.0));
        mouse(&mut c, &[(20.0, 20.0), (40.0, 20.0)]);
        assert_eq!(c.drawing().strokes()[0].points[0].x, 20.0);
        assert_eq!(c.drawing().strokes()[0].points[1].x, 30.0);
        c.set_zoom_scale(1000.0);
        assert_eq!(c.zoom_scale(), ZOOM_RANGE.1);
        c.set_zoom_scale(f32::NAN);
        assert_eq!(c.zoom_scale(), ZOOM_RANGE.1);
        assert_eq!(c.visible_rect(100.0, 50.0), Rect { x: 10.0, y: 0.0, w: 100.0 / ZOOM_RANGE.1, h: 50.0 / ZOOM_RANGE.1 });
    }

    fn profile() -> StylusProfile {
        StylusProfile { pressure: LogicalRange { min: 0.0, max: 1000.0 }, tilt: LogicalRange { min: -90.0, max: 90.0 } }
    }

    fn report(x: f32, y: f32, pressure: f32, touching: bool, eraser: bool) -> StylusReport {
        StylusReport { x, y, time_offset: 0.0, pressure, tilt_x: 45.0, tilt_y: 0.0, touching, eraser }
    }

    #[test]
    fn a_pen_tablet_stroke_keeps_pressure_and_tilt_and_a_hover_ends_it() {
        let mut c = desktop();
        assert!(c.stylus(&report(10.0, 10.0, 500.0, true, false), &profile(), 0.0));
        c.stylus(&report(20.0, 10.0, 1000.0, true, false), &profile(), 16.0);
        assert!(c.is_drawing());
        c.stylus(&report(20.0, 10.0, 0.0, false, false), &profile(), 32.0);
        assert!(!c.is_drawing());
        let p = &c.drawing().strokes()[0].points;
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].force, 0.5);
        assert_eq!(p[1].force, 1.0);
        assert!(p[0].altitude > 0.0 && p[0].altitude < std::f32::consts::FRAC_PI_2, "the lean was recorded");
        assert!(c.accepts(TouchType::Direct), "a pen alone does not turn fingers off on the desktop");
    }

    #[test]
    fn the_eraser_end_of_the_pen_erases_whatever_tool_is_selected() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (100.0, 0.0)]);
        assert!(matches!(c.tool_picker().selected(), Tool::Ink(_)));
        c.stylus(&report(50.0, 1.0, 400.0, true, true), &profile(), 0.0);
        c.stylus(&report(50.0, 1.0, 0.0, false, true), &profile(), 10.0);
        assert!(c.drawing().is_empty());
        assert!(matches!(c.tool_picker().selected(), Tool::Ink(_)), "the selected tool did not change");
    }

    #[test]
    fn a_pen_report_with_a_bad_position_is_ignored() {
        let mut c = desktop();
        assert!(!c.stylus(&report(f32::NAN, 0.0, 100.0, true, false), &profile(), 0.0));
        assert!(!c.is_drawing());
    }

    #[test]
    fn a_drawing_saves_and_loads_through_its_bytes() {
        let mut c = desktop();
        let mut t = touch(1, TouchPhase::Began, TouchType::Pencil, 3.0, 4.0);
        t.force = Some(1.0);
        t.max_force = Some(4.0);
        c.touches(&frame(t, 0.0));
        t.phase = TouchPhase::Moved;
        t.x = 9.5;
        c.touches(&frame(t, 20.0));
        t.phase = TouchPhase::Ended;
        c.touches(&frame(t, 40.0));
        let bytes = c.data_representation();
        let mut other = desktop();
        other.set_drawing_data(&bytes).unwrap();
        assert_eq!(other.drawing(), c.drawing());
        assert!(!other.can_undo(), "a loaded drawing has no history");
    }

    #[test]
    fn hostile_saved_data_is_refused_and_leaves_the_canvas_alone() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (5.0, 5.0)]);
        let kept = c.drawing().clone();
        assert_eq!(c.set_drawing_data(&[0xff, 0xfe, 0x00]), Err(DataError::NotText));
        assert_eq!(c.set_drawing_data(&vec![b'x'; MAX_DATA_BYTES + 1]), Err(DataError::TooLarge));
        assert!(matches!(c.set_drawing_data(b"hello"), Err(DataError::Format(DrawingError::BadHeader))));
        assert!(matches!(c.set_drawing_data(b"bifrost-drawing 1\npoint 1 2 3 4 5 6 7 8 9\n"), Err(DataError::Format(_))), "a point before any stroke");
        assert!(matches!(c.set_drawing_data(b"bifrost-drawing 1\nstroke pen ff000000 NaN\n"), Err(DataError::Format(_))));
        assert_eq!(c.drawing(), &kept);
        assert!(c.can_undo(), "the history is kept too");
    }

    #[test]
    fn setting_a_drawing_drops_the_history_and_the_gesture() {
        let mut c = desktop();
        mouse(&mut c, &[(0.0, 0.0), (5.0, 5.0)]);
        c.touches(&frame(touch(1, TouchPhase::Began, TouchType::Indirect, 1.0, 1.0), 0.0));
        c.set_drawing(Drawing::new());
        assert!(!c.is_drawing() && !c.can_undo() && c.drawing().is_empty());
    }

    #[test]
    fn the_change_count_moves_with_each_edit() {
        let mut c = desktop();
        let start = c.change_count();
        mouse(&mut c, &[(0.0, 0.0), (5.0, 5.0)]);
        assert_ne!(c.change_count(), start);
        c.undo();
        assert_eq!(c.change_count(), start, "undoing back to the start is the saved state again");
    }

    #[test]
    fn erase_steps_cover_a_jump_and_stay_bounded() {
        let steps = erase_steps((0.0, 0.0), (100.0, 0.0), 5.0);
        assert_eq!(steps.len(), 20);
        assert_eq!(steps.last(), Some(&(100.0, 0.0)));
        assert_eq!(erase_steps((0.0, 0.0), (0.0, 0.0), 5.0), vec![(0.0, 0.0)]);
        assert_eq!(erase_steps((0.0, 0.0), (1.0e9, 0.0), 0.25).len(), MAX_ERASE_STEPS);
        assert_eq!(erase_steps((0.0, 0.0), (10.0, 0.0), 0.0).len(), 1);
    }
}
