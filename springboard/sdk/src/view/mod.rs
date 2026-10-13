//! Retained views (`NativeView`, `NativeResponder`, `NativeWindow`, `NativeViewController`) on top of the immediate-mode `App` trait.
//!
//! A [`Window`] owns a tree of [`View`]s. Frames are in window points (no per-view bounds or transform), a view's
//! `draw` paints only itself (the window walks the tree, parents first), and pointer events go to the deepest view under
//! the pointer and bubble to its ancestors until one handles them. Controls send *action strings* (target-nil
//! `sendAction:`): the action travels sender, ancestors, the window (Undo and Redo), then the app model `S`.
//! Key commands and menu items use the same walk from the first responder (see [`Window::send`], [`Window::is_enabled`]).
//! `ViewApp` backs an `App` with a window in a few lines. Gaps (no Auto Layout, see animation.rs for the animation subset) are listed in
//! docs/bifrost-ui.md "Retained views".
use crate::kit::ui::DETAIL_BG;
use crate::kit::{Damage, Event, Ui};
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use crate::kit::gfx::Image;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use crate::proto::{KeyInput, CHORD_SHIFT};
use bifrost_ui::focus::{self, Focusable};
use bifrost_ui::responder::Responder;
use bifrost_ui::undo::UndoManager;
use bifrost_ui::{Command, Commands, Node, Rect};
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

/// Implements the four plumbing methods of [`View`] for a type with a `base: ViewBase` field.
#[macro_export]
macro_rules! view_base {
    () => {
        fn base(&self) -> &$crate::view::ViewBase {
            &self.base
        }
        fn base_mut(&mut self) -> &mut $crate::view::ViewBase {
            &mut self.base
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

mod animation;
mod app;
mod autolayout;
mod controls;
mod forms;
mod scroll;
pub use animation::{mix_color, Prop, Timing, Transaction, DEFAULT_DURATION, REDUCED_DURATION};
pub use app::{action_for, ControllerHost, MenuEntry, ViewApp, ViewController};
pub use forms::{Checkbox, Form, PopUpButton, ProgressIndicator, ProgressStyle, Section, Stepper, TextField};
pub use autolayout::{anchors, solve as solve_constraints, Anchor, Anchors, Attr, Constraints, Item, Solution, Spec, ViewInfo};
pub use bifrost_ui::constraint::Rel;
pub use bifrost_ui::stack::Distribution;
pub use scroll::ScrollView;
pub use controls::{Align, Button, Label, Segmented, Slider, Stack, Switch};

/// What a text-editing view tells the shell about itself while it is first responder (`NativeTextInputClient`,
/// `NativeTextInput`): the text around the caret, the caret rectangle in window points `(x, y, w, h)` and the traits.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInputInfo {
    pub state: crate::proto::TextState,
    pub caret: (f32, f32, f32, f32),
    pub traits: crate::proto::InputTraits,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// State every view carries (`NativeView`'s frame, `needsDisplay`, `isHidden`, first-responder flag, queued actions).
#[derive(Debug)]
pub struct ViewBase {
    id: u64,
    pub frame: Rect,
    /// `needsDisplay`
    pub dirty: bool,
    pub hidden: bool,
    pub enabled: bool,
    /// set by the window while this view is the first responder (draw a focus ring when true)
    pub focused: bool,
    /// `alphaValue` (0..=1), animatable with [`Window::animate`]; below 1 the view and its subtree are faded as a group
    pub opacity: f32,
    /// Premultiplied fill the window paints under the view (`layer.backgroundColor`), animatable; `None` paints nothing
    pub background: Option<u32>,
    /// actions queued by `send_action`, dispatched by the window after the event
    pub pending: Vec<String>,
}

impl Default for ViewBase {
    fn default() -> Self {
        ViewBase { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), frame: Rect::default(), dirty: true, hidden: false, enabled: true, focused: false, opacity: 1.0, background: None, pending: Vec::new() }
    }
}

impl ViewBase {
    /// `NativeControl.sendAction(_:to:)` with a nil target: queues `action`; the window sends it up the chain.
    pub fn send_action(&mut self, action: &str) {
        self.pending.push(action.into());
    }
}

/// What a view receives (`NativeEvent` reduced to what the SDK delivers). Pointer positions are window points.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewEvent {
    Pointer(Event),
    Key(KeyInput),
    /// the system took the pointer away (`touchesCancelled`): drop pressed state, do not act
    Cancel,
}

impl ViewEvent {
    /// Converts an SDK input; touch positions are divided by `scale` (px per point). Other inputs yield `None`.
    pub fn from_input(input: &crate::Input, scale: f32) -> Option<ViewEvent> {
        use crate::proto::TouchKind;
        match input {
            crate::Input::Touch { kind, x, y, .. } => {
                let (x, y) = (x / scale, y / scale);
                Some(match kind {
                    TouchKind::Down => ViewEvent::Pointer(Event::Down(x, y)),
                    TouchKind::Move => ViewEvent::Pointer(Event::Move(x, y)),
                    TouchKind::Up => ViewEvent::Pointer(Event::Up(x, y)),
                    TouchKind::Cancel => ViewEvent::Cancel,
                })
            }
            crate::Input::Key(k) => Some(ViewEvent::Key(k.clone())),
            _ => None,
        }
    }
}

/// A node of the view tree (`NativeView` + `NativeResponder`). Implement `base`, `base_mut`, `as_any`, `as_any_mut` with
/// [`view_base!`](crate::view_base); every other method has a default.
pub trait View: 'static {
    fn base(&self) -> &ViewBase;
    fn base_mut(&mut self) -> &mut ViewBase;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn id(&self) -> u64 {
        self.base().id
    }
    /// Window points.
    fn frame(&self) -> Rect {
        self.base().frame
    }
    /// Moves or resizes; marks the view dirty when the frame changes.
    fn set_frame(&mut self, frame: Rect) {
        if self.base().frame != frame {
            self.base_mut().frame = frame;
            self.base_mut().dirty = true;
        }
    }
    /// The size the view prefers in points (`intrinsicContentSize`); 0 on an axis means "no preference".
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `contentHuggingPriority(for:)`, 1 to 1000: how strongly the view resists growing past its `intrinsic` size.
    fn hugging(&self, _axis: bifrost_ui::stack::Axis) -> u16 {
        250
    }
    /// `contentCompressionResistancePriority(for:)`: how strongly the view resists shrinking below its `intrinsic` size.
    fn compression(&self, _axis: bifrost_ui::stack::Axis) -> u16 {
        750
    }
    /// `layoutSubtreeIfNeeded`: given the rectangle assigned by the parent, set the own frame and place children.
    /// The default fills `bounds` and gives every child the same rectangle.
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        for c in self.children_mut() {
            c.layout(ui, bounds);
        }
    }
    /// Paints this view only (the window draws the children after it).
    fn draw(&mut self, _s: &mut dyn Canvas, _ui: &Ui) {}
    /// Whether `(x, y)` (window points) is inside the view (`hitTest:` without recursion).
    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.frame().contains(x, y)
    }
    /// `acceptsFirstResponder`: true for views that take keys and join the Tab loop.
    fn accepts_first_responder(&self) -> bool {
        false
    }
    /// `resignFirstResponder`: asked before the window moves the focus elsewhere; false refuses the change.
    fn resign_first_responder(&mut self) -> bool {
        true
    }
    /// Called after this view lost the first-responder status (a pop-up closes its menu here).
    fn did_resign_first_responder(&mut self) {}
    /// `NativeClipView.bounds.origin`: how far the children's content is scrolled. A child frame is in content
    /// coordinates; it is drawn and hit at `frame - offset`. Pointer events reach the children in content coordinates.
    fn child_offset(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// True when children are clipped to this view's frame (rows only, see `ScrollView`).
    fn clips_children(&self) -> bool {
        false
    }
    /// Pointer or key event; true when handled (bubbling stops). Pointer events arrive at the deepest view first.
    fn handle(&mut self, _ev: &ViewEvent) -> bool {
        false
    }
    /// Responder chain: performs `action` when this view implements it.
    fn perform(&mut self, _action: &str) -> bool {
        false
    }
    /// Responder chain: `Some(enabled)` when this view knows `action`, `None` to defer outwards.
    fn validate(&self, _action: &str) -> Option<bool> {
        None
    }
    /// The accessibility element for this view, if it is one. Ids should be `self.id()`.
    fn ax_node(&self) -> Option<Node> {
        None
    }
    fn children(&self) -> &[Box<dyn View>] {
        &[]
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut []
    }
    /// `NativeTextInputClient` for the first responder: `Some` when this view edits text. Reported to the shell by
    /// `ViewApp` after every event and frame.
    fn text_input_info(&self) -> Option<TextInputInfo> {
        None
    }
    /// An IME or keyboard event (`SetMarkedText`, `UnmarkText`, `DeleteSurrounding`); true when handled.
    fn apply_text_input(&mut self, _ev: &crate::proto::ToApp) -> bool {
        false
    }
    /// Caret, selection and visible range of a text element, as `AXSelectedTextRange` and `AXVisibleCharacterRange` report them.
    fn ax_text(&self) -> Option<bifrost_ui::ax::AxText> {
        None
    }
    /// `(accessibilityTitle, accessibilityHelp)` of this element when it has either.
    fn ax_meta(&self) -> Option<(String, String)> {
        None
    }
    /// `AXValue` of a checkable element (checkbox, switch); `None` when not checkable.
    fn ax_checked(&self) -> Option<bool> {
        None
    }
    /// `AXValue` write from an assistive technology (slider position, text, "on"/"off"); true when accepted. Queue
    /// the control's action with `send_action` as a user change would.
    fn ax_set_value(&mut self, _value: &str) -> bool {
        false
    }
    /// Frame time in seconds (called by `Window::tick` before drawing); mark the view dirty to animate.
    fn tick(&mut self, _t: f64) {}
    /// True while the view advances on `tick` by itself (a fling, a spinner), so the window asks for the next wake.
    fn wants_tick(&self) -> bool {
        false
    }
    /// True when the view paints an opaque backdrop of its own (a grouped card). A dirty descendant then repaints this
    /// view as a whole instead of clearing its own frame to the window background (the nearest-opaque-ancestor rule
    /// that replaces clipping, see docs/bifrost-ui.md).
    fn opaque_backdrop(&self) -> bool {
        false
    }
    /// The area the view paints, when larger than its frame (an open pop-up menu). A view whose area differs from its
    /// frame while dirty makes the window repaint everything.
    fn dirty_rect(&self) -> Rect {
        self.base().frame
    }
    /// Paints above the whole tree after the normal pass (menus, tooltips).
    fn draw_overlay(&mut self, _s: &mut dyn Canvas, _ui: &Ui) {}
    fn needs_display(&self) -> bool {
        self.base().dirty
    }
    fn set_needs_display(&mut self) {
        self.base_mut().dirty = true;
    }
}

/// A plain container or spacer (`NativeView`).
#[derive(Default)]
pub struct Container {
    base: ViewBase,
    children: Vec<Box<dyn View>>,
}

impl Container {
    pub fn new(children: Vec<Box<dyn View>>) -> Self {
        Container { base: ViewBase::default(), children }
    }
}

impl View for Container {
    view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.children
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.children
    }
}

/// The app model when it has no responder of its own.
#[derive(Default)]
pub struct NoApp;
impl Responder for NoApp {}

/// Who handled an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handler {
    View(u64),
    Window,
    App,
}

/// A window host (`NativeWindow`): the view tree, first responder, key-view loop, responder chain and undo.
pub struct Window<S: Responder = NoApp> {
    root: Box<dyn View>,
    first: Option<u64>,
    tracking: Option<u64>,
    full: bool,
    anim: animation::Animator,
    sent: Vec<(u64, String)>,
    /// Auto Layout constraints (see [`Constraints`]); views no constraint names keep their manual layout.
    constraints: Constraints,
    /// Required constraints that conflicted in the last layout (indices into the constraint list).
    pub broken_constraints: Vec<usize>,
    /// `NativeWindow.undoManager`: register changes of the model `S`; Edit > Undo and Redo reach it as "undo" and "redo".
    pub undo: UndoManager<S>,
}

/// A window with no app model.
pub type ViewTree = Window<NoApp>;

fn path_to(v: &dyn View, id: u64, path: &mut Vec<usize>) -> bool {
    if v.id() == id {
        return true;
    }
    for (i, c) in v.children().iter().enumerate() {
        path.push(i);
        if path_to(c.as_ref(), id, path) {
            return true;
        }
        path.pop();
    }
    false
}

fn deepest(v: &dyn View, x: f32, y: f32, path: &mut Vec<usize>) -> bool {
    if v.base().hidden || !v.hit_test(x, y) {
        return false;
    }
    let (ox, oy) = v.child_offset();
    for (i, c) in v.children().iter().enumerate().rev() {
        path.push(i);
        if deepest(c.as_ref(), x + ox, y + oy, path) {
            return true;
        }
        path.pop();
    }
    true
}

/// An open overlay (a pop-up menu) outside its view's frame wins over whatever is drawn under it.
fn overlay_at(v: &dyn View, x: f32, y: f32, path: &mut Vec<usize>) -> bool {
    if v.base().hidden {
        return false;
    }
    let (ox, oy) = v.child_offset();
    for (i, c) in v.children().iter().enumerate() {
        path.push(i);
        if overlay_at(c.as_ref(), x + ox, y + oy, path) {
            return true;
        }
        path.pop();
    }
    v.dirty_rect() != v.frame() && !v.frame().contains(x, y) && v.hit_test(x, y)
}

/// The view that receives a pointer at `(x, y)`: an open overlay first, otherwise the deepest view.
fn pick(v: &dyn View, x: f32, y: f32, path: &mut Vec<usize>) -> bool {
    overlay_at(v, x, y, path) || deepest(v, x, y, path)
}

fn at<'a>(root: &'a dyn View, path: &[usize]) -> &'a dyn View {
    path.iter().fold(root, |v, &i| v.children()[i].as_ref())
}

fn at_mut<'a>(root: &'a mut dyn View, path: &[usize]) -> &'a mut dyn View {
    let mut cur = root;
    for &i in path {
        cur = cur.children_mut()[i].as_mut();
    }
    cur
}

fn collect_focus(v: &dyn View, out: &mut Vec<Focusable>, dx: f32, dy: f32) {
    if v.base().hidden {
        return;
    }
    if v.accepts_first_responder() {
        let f = v.frame();
        out.push(Focusable { id: v.id(), x: f.x - dx, y: f.y - dy, w: f.w, h: f.h, enabled: v.base().enabled });
    }
    let (ox, oy) = v.child_offset();
    v.children().iter().for_each(|c| collect_focus(c.as_ref(), out, dx + ox, dy + oy));
}

fn collect_ax(v: &dyn View, depth: usize, parent: Option<u64>, out: &mut Vec<AxEntry>) {
    if v.base().hidden {
        return;
    }
    let mut next = (depth, parent);
    if let Some(node) = v.ax_node() {
        next = (depth + 1, Some(node.id));
        out.push(AxEntry { node, depth, parent });
    }
    v.children().iter().for_each(|c| collect_ax(c.as_ref(), next.0, next.1, out));
}

fn any_dirty(v: &dyn View) -> bool {
    v.needs_display() || v.children().iter().any(|c| any_dirty(c.as_ref()))
}

/// Pixel rectangle `(x, y, w, h)` covering `r` (points) completely.
fn px_rect(ui: &Ui, r: Rect) -> (usize, usize, usize, usize) {
    let (x0, y0) = (ui.pt(r.x).floor().max(0.0), ui.pt(r.y).floor().max(0.0));
    let (x1, y1) = (ui.pt(r.right()).ceil().max(x0), ui.pt(r.bottom()).ceil().max(y0));
    (x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize)
}

fn save_frames(v: &dyn View, out: &mut Vec<Rect>) {
    out.push(v.frame());
    v.children().iter().for_each(|c| save_frames(c.as_ref(), out));
}

fn shift_frames(v: &mut dyn View, dx: f32, dy: f32) {
    let mut f = v.base().frame;
    f.x += dx;
    f.y += dy;
    v.base_mut().frame = f;
    v.children_mut().iter_mut().for_each(|c| shift_frames(c.as_mut(), dx, dy));
}

fn restore_frames(v: &mut dyn View, it: &mut std::vec::IntoIter<Rect>) {
    if let Some(f) = it.next() {
        v.base_mut().frame = f;
    }
    v.children_mut().iter_mut().for_each(|c| restore_frames(c.as_mut(), it));
}

/// Runs `f` with the children's frames moved by `-child_offset()` (content scrolled into place), then restores them
/// exactly. Views without an offset are untouched.
fn scrolled(v: &mut dyn View, f: impl FnOnce(&mut dyn View)) {
    let (ox, oy) = v.child_offset();
    if ox == 0.0 && oy == 0.0 {
        return f(v);
    }
    let mut saved = Vec::new();
    v.children().iter().for_each(|c| save_frames(c.as_ref(), &mut saved));
    v.children_mut().iter_mut().for_each(|c| shift_frames(c.as_mut(), -ox, -oy));
    f(v);
    let mut it = saved.into_iter();
    v.children_mut().iter_mut().for_each(|c| restore_frames(c.as_mut(), &mut it));
}

/// Offset of the content space the view at `path` lives in: the sum of its ancestors' `child_offset`.
fn path_offset(root: &dyn View, path: &[usize]) -> (f32, f32) {
    let (mut v, mut sum) = (root, (0.0, 0.0));
    for &i in path {
        let (ox, oy) = v.child_offset();
        sum = (sum.0 + ox, sum.1 + oy);
        v = v.children()[i].as_ref();
    }
    sum
}

fn clear_dirty(v: &mut dyn View) {
    v.base_mut().dirty = false;
    v.children_mut().iter_mut().for_each(|c| clear_dirty(c.as_mut()));
}

/// Content key of a rendered image (`ImageRef::id`): equal pixels give equal ids, so the GPU side can reuse a texture.
pub(super) fn image_key(img: &Image) -> u64 {
    let mut h = DefaultHasher::new();
    img.w.hash(&mut h);
    img.h.hash(&mut h);
    img.px.hash(&mut h);
    h.finish()
}

/// Draws the rendered image `img` with its top-left corner at the device pixel `(x, y)`, source-over (what
/// `Surface::blit` did for text and glyph images).
pub(super) fn blit(c: &mut dyn Canvas, img: &Image, x: i32, y: i32) {
    let dst = RectF::new(x as f32, y as f32, img.w as f32, img.h as f32);
    c.image(ImageRef { id: image_key(img), image: img }, dst, 1.0);
}

/// The device-pixel rectangle covering `r`, as a canvas rectangle.
fn px_rf(ui: &Ui, r: Rect) -> RectF {
    let (x, y, w, h) = px_rect(ui, r);
    RectF::new(x as f32, y as f32, w as f32, h as f32)
}

fn draw_subtree(v: &mut dyn View, c: &mut dyn Canvas, ui: &Ui) {
    if v.base().hidden {
        return;
    }
    let alpha = v.base().opacity;
    if alpha <= 0.0 {
        clear_dirty(v);
        return;
    }
    // Group opacity: the subtree is drawn at `alpha` as one group (`Canvas::push_opacity`), not blended afterwards.
    if alpha < 1.0 {
        c.push_opacity(alpha);
    }
    if let Some(bg) = v.base().background {
        c.fill_rrect(px_rf(ui, v.frame()), 0.0, bg);
    }
    v.draw(c, ui);
    v.base_mut().dirty = false;
    scrolled(v, |v| {
        if v.clips_children() {
            // children are confined vertically to the rows of the frame; widths are the view's own (see ScrollView)
            let f = px_rf(ui, v.frame());
            let full = c.size().0 as f32;
            c.push_clip(RectF::new(0.0, f.y, full, f.h), 0.0);
            v.children_mut().iter_mut().for_each(|ch| draw_subtree(ch.as_mut(), c, ui));
            c.pop_clip();
        } else {
            v.children_mut().iter_mut().for_each(|ch| draw_subtree(ch.as_mut(), c, ui));
        }
    });
    if alpha < 1.0 {
        c.pop_opacity();
    }
}

fn draw_dirty(v: &mut dyn View, c: &mut dyn Canvas, ui: &Ui, d: &mut Damage) {
    if v.base().hidden {
        return;
    }
    // A dirty opaque ancestor repaints as a whole: clearing a child's frame to the window background inside a card
    // would punch a hole in it, and the canvas has no backing store of its own to confine the card's redraw.
    if v.needs_display() || (v.opaque_backdrop() && v.children().iter().any(|ch| any_dirty(ch.as_ref()))) {
        let (x, y, w, h) = px_rect(ui, v.frame());
        c.fill_rect(px_rf(ui, v.frame()), DETAIL_BG.now());
        d.add(x as i32, y as i32, w as i32, h as i32);
        draw_subtree(v, c, ui);
    } else {
        v.children_mut().iter_mut().for_each(|ch| draw_dirty(ch.as_mut(), c, ui, d));
    }
}

/// Some view paints outside its frame and is dirty or open: only a full repaint is safe.
fn wide_dirty(v: &dyn View) -> bool {
    !v.base().hidden && ((v.dirty_rect() != v.frame()) || v.children().iter().any(|c| wide_dirty(c.as_ref())))
}

fn draw_overlays(v: &mut dyn View, c: &mut dyn Canvas, ui: &Ui) {
    if v.base().hidden {
        return;
    }
    v.draw_overlay(c, ui);
    scrolled(v, |v| v.children_mut().iter_mut().for_each(|ch| draw_overlays(ch.as_mut(), c, ui)));
}

/// Whether any visible view in the subtree advances on `tick` (`View::wants_tick`).
pub(super) fn any_wants_tick(v: &dyn View) -> bool {
    !v.base().hidden && (v.wants_tick() || v.children().iter().any(|c| any_wants_tick(c.as_ref())))
}

fn tick_all(v: &mut dyn View, t: f64) {
    if v.base().hidden {
        return;
    }
    v.tick(t);
    v.children_mut().iter_mut().for_each(|c| tick_all(c.as_mut(), t));
}

/// One accessibility element with its place in the tree (`Node` itself is flat).
#[derive(Clone, Debug, PartialEq)]
pub struct AxEntry {
    pub node: Node,
    /// number of ancestors that are accessibility elements
    pub depth: usize,
    /// id of the nearest ancestor element
    pub parent: Option<u64>,
}

impl<S: Responder> Window<S> {
    pub fn new(root: impl View, undo_limit: usize) -> Self {
        Window { root: Box::new(root), first: None, tracking: None, full: true, anim: Default::default(), sent: Vec::new(), constraints: Constraints::new(), broken_constraints: Vec::new(), undo: UndoManager::new(undo_limit) }
    }
    pub fn root(&self) -> &dyn View {
        self.root.as_ref()
    }
    pub fn root_mut(&mut self) -> &mut dyn View {
        self.root.as_mut()
    }
    /// The view with `id` as concrete type `T` (`viewWithTag:` plus a cast).
    pub fn view<T: View>(&self, id: u64) -> Option<&T> {
        let mut p = Vec::new();
        path_to(self.root(), id, &mut p).then(|| at(self.root(), &p).as_any().downcast_ref::<T>()).flatten()
    }
    pub fn view_mut<T: View>(&mut self, id: u64) -> Option<&mut T> {
        let mut p = Vec::new();
        if !path_to(self.root(), id, &mut p) {
            return None;
        }
        at_mut(self.root.as_mut(), &p).as_any_mut().downcast_mut::<T>()
    }

    /// Lays the tree out in `bounds` (window points) and schedules a full repaint.
    pub fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.root.layout(ui, bounds);
        self.apply_constraints(ui, bounds);
        self.full = true;
    }

    /// `NativeWindow.contentView.addConstraints`: adds `cs` to the window's constraints; the next [`Window::layout`] solves
    /// them. Views they do not name keep the frame the top-down layout gave them.
    pub fn constrain(&mut self, cs: Constraints) {
        self.constraints.extend(cs);
        self.full = true;
    }
    /// Removes every constraint (back to purely manual layout).
    pub fn remove_constraints(&mut self) {
        self.constraints = Constraints::new();
        self.broken_constraints.clear();
        self.full = true;
    }
    pub fn constraints(&self) -> &Constraints {
        &self.constraints
    }

    /// Gathers the constrained views, solves, and lays each out in its solved frame (its subtree follows).
    fn apply_constraints(&mut self, ui: &Ui, bounds: Rect) {
        if self.constraints.is_empty() {
            return;
        }
        use bifrost_ui::stack::Axis;
        let mut infos = Vec::new();
        for id in self.constraints.view_ids() {
            let mut p = Vec::new();
            if path_to(self.root(), id, &mut p) {
                let v = at(self.root(), &p);
                infos.push(ViewInfo {
                    id,
                    frame: v.frame(),
                    intrinsic: v.intrinsic(ui),
                    hugging: (v.hugging(Axis::Horizontal), v.hugging(Axis::Vertical)),
                    compression: (v.compression(Axis::Horizontal), v.compression(Axis::Vertical)),
                });
            }
        }
        let margin = bifrost_ui::prefs_layout::m::DETAIL_MARGIN;
        let sol = solve_constraints(&self.constraints, bounds, crate::layout::top_inset(), margin, &infos);
        self.broken_constraints = sol.broken;
        for (id, r) in sol.frames {
            let mut p = Vec::new();
            if path_to(self.root(), id, &mut p) {
                at_mut(self.root.as_mut(), &p).layout(ui, r);
            }
        }
    }

    /// The deepest visible view under `(x, y)`.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<u64> {
        let mut p = Vec::new();
        pick(self.root(), x, y, &mut p).then(|| at(self.root(), &p).id())
    }

    pub fn first_responder(&self) -> Option<u64> {
        self.first
    }
    /// `makeFirstResponder:`; false (and no change) when the view refuses or is unknown.
    pub fn make_first_responder(&mut self, id: Option<u64>) -> bool {
        if let Some(id) = id {
            let mut p = Vec::new();
            if !path_to(self.root(), id, &mut p) {
                return false;
            }
            let v = at(self.root(), &p);
            if !v.accepts_first_responder() || !v.base().enabled {
                return false;
            }
        }
        if id == self.first {
            return true;
        }
        // `resignFirstResponder`: the old responder may refuse the change.
        if let Some(v) = self.first.and_then(|i| self.view_by_id(i)) {
            if !v.resign_first_responder() {
                return false;
            }
        }
        let old = self.first;
        for (old, on) in [(self.first, false), (id, true)] {
            if let Some(v) = old.and_then(|i| self.view_by_id(i)) {
                v.base_mut().focused = on;
                v.base_mut().dirty = true;
            }
        }
        self.first = id;
        if let Some(v) = old.and_then(|i| self.view_by_id(i)) {
            v.did_resign_first_responder();
        }
        true
    }
    fn view_by_id(&mut self, id: u64) -> Option<&mut dyn View> {
        let mut p = Vec::new();
        path_to(self.root(), id, &mut p).then(|| at_mut(self.root.as_mut(), &p))
    }
    /// The key-view loop in reading order (`NativeWindow.keyViewLoop`).
    pub fn key_view_loop(&self) -> Vec<u64> {
        let mut items = Vec::new();
        collect_focus(self.root(), &mut items, 0.0, 0.0);
        let mut order = Vec::new();
        let mut cur = None;
        while let Some(n) = focus::next(&items, cur, true) {
            if order.contains(&n) {
                break;
            }
            order.push(n);
            cur = Some(n);
        }
        order
    }
    /// Tab (`forward`) or Shift-Tab: moves the first responder along the loop; false when nothing can take it.
    pub fn tab(&mut self, forward: bool) -> bool {
        let mut items = Vec::new();
        collect_focus(self.root(), &mut items, 0.0, 0.0);
        match focus::next(&items, self.first, forward) {
            Some(id) => self.make_first_responder(Some(id)),
            None => false,
        }
    }

    /// Delivers `ev`; true when a new frame is needed. Pointer: the deepest view under the pointer (which becomes first
    /// responder if it accepts), bubbling to ancestors; Move and Up go to the view that took the Down. Keys: first
    /// responder and outwards, then Tab and Shift-Tab move the focus. Queued control actions are sent afterwards.
    pub fn event(&mut self, ev: &ViewEvent, state: &mut S) -> bool {
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                let mut p = Vec::new();
                if pick(self.root(), *x, *y, &mut p) {
                    let v = at(self.root(), &p);
                    let (id, accepts) = (v.id(), v.accepts_first_responder());
                    // a click on a view that does not take focus resigns the current responder (empty background)
                    if !self.make_first_responder(Some(id)) && !accepts {
                        self.make_first_responder(None);
                    }
                    self.bubble(&p, ev, state, true);
                } else {
                    self.make_first_responder(None);
                }
            }
            ViewEvent::Pointer(_) | ViewEvent::Cancel => {
                let t = self.tracking;
                let mut p = Vec::new();
                if t.is_some_and(|id| path_to(self.root(), id, &mut p)) {
                    self.dispatch_at(&p, ev, state);
                }
                if matches!(ev, ViewEvent::Pointer(Event::Up(..)) | ViewEvent::Cancel) {
                    self.tracking = None;
                }
            }
            ViewEvent::Key(k) => {
                let mut p = Vec::new();
                if let Some(id) = self.first {
                    path_to(self.root(), id, &mut p);
                }
                if !self.bubble(&p, ev, state, false) {
                    match k {
                        KeyInput::Code(9) => {
                            self.tab(true);
                        }
                        k if k.as_chord() == Some((15, CHORD_SHIFT)) => {
                            self.tab(false);
                        }
                        _ => {}
                    }
                }
            }
        }
        self.needs_display()
    }

    /// Offers `ev` to the view at `path` and then its ancestors; returns whether one handled it.
    fn bubble(&mut self, path: &[usize], ev: &ViewEvent, state: &mut S, track: bool) -> bool {
        for depth in (0..=path.len()).rev() {
            if self.dispatch_at(&path[..depth], ev, state) {
                if track {
                    self.tracking = Some(at(self.root(), &path[..depth]).id());
                }
                return true;
            }
        }
        false
    }

    fn dispatch_at(&mut self, path: &[usize], ev: &ViewEvent, state: &mut S) -> bool {
        let (ox, oy) = path_offset(self.root(), path);
        let local = match ev {
            ViewEvent::Pointer(Event::Down(x, y)) if ox != 0.0 || oy != 0.0 => Some(Event::Down(x + ox, y + oy)),
            ViewEvent::Pointer(Event::Move(x, y)) if ox != 0.0 || oy != 0.0 => Some(Event::Move(x + ox, y + oy)),
            ViewEvent::Pointer(Event::Up(x, y)) if ox != 0.0 || oy != 0.0 => Some(Event::Up(x + ox, y + oy)),
            _ => None,
        };
        let v = at_mut(self.root.as_mut(), path);
        let handled = match local {
            Some(e) => v.handle(&ViewEvent::Pointer(e)),
            None => v.handle(ev),
        };
        self.flush(path, state);
        handled
    }

    /// Actions controls sent since the last call, as `(sender id, action)`; read the sender's value with [`Window::view`].
    pub fn take_sent(&mut self) -> Vec<(u64, String)> {
        std::mem::take(&mut self.sent)
    }

    /// The first responder's `NativeTextInputClient` state, when it edits text.
    pub fn text_input_info(&self) -> Option<TextInputInfo> {
        let id = self.first?;
        let mut p = Vec::new();
        path_to(self.root(), id, &mut p).then(|| at(self.root(), &p).text_input_info()).flatten()
    }
    /// Offers an IME event to the first responder; queued change actions are sent. True when handled.
    pub fn apply_text_input(&mut self, ev: &crate::proto::ToApp, state: &mut S) -> bool {
        let Some(id) = self.first else { return false };
        let mut p = Vec::new();
        if !path_to(self.root(), id, &mut p) {
            return false;
        }
        let handled = at_mut(self.root.as_mut(), &p).apply_text_input(ev);
        self.flush(&p, state);
        handled
    }

    /// Sends `action` from the first responder outwards (menu items, key equivalents).
    pub fn send(&mut self, action: &str, state: &mut S) -> Option<Handler> {
        let mut p = Vec::new();
        if let Some(id) = self.first {
            path_to(self.root(), id, &mut p);
        }
        let r = self.send_from(&p, action, state);
        // a control that performed the action (a text field's "paste") may have queued its own change action
        if let Some(Handler::View(id)) = r {
            let mut q = Vec::new();
            if path_to(self.root(), id, &mut q) {
                self.flush(&q, state);
            }
        }
        r
    }

    fn send_from(&mut self, path: &[usize], action: &str, state: &mut S) -> Option<Handler> {
        for depth in (0..=path.len()).rev() {
            let v = at_mut(self.root.as_mut(), &path[..depth]);
            if v.perform(action) {
                return Some(Handler::View(v.id()));
            }
        }
        let undone = match action {
            "undo" => self.undo.undo(state).is_some(),
            "redo" => self.undo.redo(state).is_some(),
            _ => false,
        };
        if undone {
            self.root.set_needs_display();
            return Some(Handler::Window);
        }
        state.perform(action).then_some(Handler::App)
    }

    /// Whether a menu item for `action` is enabled: the first responder, its ancestors, the window, then the model.
    pub fn is_enabled(&self, action: &str, state: &S) -> bool {
        let mut p = Vec::new();
        if let Some(id) = self.first {
            path_to(self.root(), id, &mut p);
        }
        for depth in (0..=p.len()).rev() {
            if let Some(on) = at(self.root(), &p[..depth]).validate(action) {
                return on;
            }
        }
        match action {
            "undo" => self.undo.can_undo(),
            "redo" => self.undo.can_redo(),
            _ => state.validate(action).unwrap_or(false),
        }
    }
    /// The Edit menu titles from the undo manager: `("Undo Typing", "Redo")`.
    pub fn undo_titles(&self) -> (String, String) {
        (self.undo.undo_menu_item_title(), self.undo.redo_menu_item_title())
    }
    /// The `App::commands` bitmask: every [`Command`] whose action is enabled.
    pub fn commands(&self, state: &S) -> Commands {
        Command::ALL.iter().fold(Commands::default(), |c, &cmd| c.with(cmd, self.is_enabled(app::action_for(cmd), state)))
    }

    /// The accessibility elements in tree order, with depth and parent.
    pub fn ax_tree(&self) -> Vec<AxEntry> {
        let mut out = Vec::new();
        collect_ax(self.root(), 0, None, &mut out);
        out
    }
    /// `App::controls`: the elements in tree order (parents before children). `Node` has no parent field; use
    /// [`Window::ax_tree`] for the nesting.
    pub fn ax_nodes(&self) -> Vec<Node> {
        self.ax_tree().into_iter().map(|e| e.node).collect()
    }

    pub fn needs_display(&self) -> bool {
        self.full || any_dirty(self.root())
    }
    /// Paints what changed and returns the merged damage. After `layout` (or the first call) the whole window is
    /// repainted. Otherwise each dirty view's frame is cleared to the window background and the view and its subtree
    /// are drawn again; sibling views must not overlap.
    pub fn draw(&mut self, c: &mut dyn Canvas, ui: &Ui) -> Damage {
        let (w, h) = c.size();
        let mut d = Damage::new(w as usize, h as usize);
        if self.full || (any_dirty(self.root()) && wide_dirty(self.root())) {
            self.full = false;
            c.fill_rect(RectF::new(0.0, 0.0, w as f32, h as f32), DETAIL_BG.now());
            draw_subtree(self.root.as_mut(), c, ui);
            d.all();
        } else {
            draw_dirty(self.root.as_mut(), c, ui, &mut d);
        }
        draw_overlays(self.root.as_mut(), c, ui);
        self.clear_animation_damage();
        d
    }

    /// Advances animations to frame time `t` (seconds): spinners and indeterminate bars mark themselves dirty. Call
    /// before [`Window::draw`]; true when a frame is needed.
    pub fn tick(&mut self, t: f64) -> bool {
        tick_all(self.root.as_mut(), t);
        self.advance_animations(t);
        self.fire_frame_links(t);
        self.needs_display()
    }

    /// `NativeAccessibility` hierarchy for `App::controls_tree`: parent, focus and checked state of every element.
    pub fn ax_ext(&self) -> Vec<crate::proto::NodeExt> {
        let mut checked = Vec::new();
        collect_checked(self.root(), &mut checked);
        self.ax_tree()
            .into_iter()
            .map(|e| crate::proto::NodeExt {
                id: e.node.id,
                parent: e.parent,
                focused: self.first == Some(e.node.id),
                checked: checked.iter().find(|(i, _)| *i == e.node.id).and_then(|(_, c)| *c),
            })
            .collect()
    }

    /// `App::ax_text`: caret, selection and visible range of every text element.
    pub fn ax_texts(&self) -> Vec<bifrost_ui::ax::AxText> {
        let mut out = Vec::new();
        collect_text(self.root(), &mut out);
        out
    }
    /// `App::ax_meta`: the title and help of every element that has one.
    pub fn ax_metas(&self) -> Vec<bifrost_ui::ax::AxMeta> {
        let mut out = Vec::new();
        collect_meta(self.root(), &mut out);
        out
    }

    /// `AXPress`: activates the element as Return would; true when it exists and handled it.
    pub fn ax_press(&mut self, id: u64, state: &mut S) -> bool {
        let mut p = Vec::new();
        if !path_to(self.root(), id, &mut p) || !at(self.root(), &p).base().enabled {
            return false;
        }
        self.dispatch_at(&p, &ViewEvent::Key(KeyInput::Code(13)), state)
    }
    /// `AXValue` write; queued control actions are sent like a user change.
    pub fn ax_set_value(&mut self, id: u64, value: &str, state: &mut S) -> bool {
        let mut p = Vec::new();
        if !path_to(self.root(), id, &mut p) {
            return false;
        }
        let v = at_mut(self.root.as_mut(), &p);
        if !v.base().enabled || !v.ax_set_value(value) {
            return false;
        }
        v.base_mut().dirty = true;
        self.flush(&p, state);
        true
    }
    fn flush(&mut self, path: &[usize], state: &mut S) {
        let v = at_mut(self.root.as_mut(), path);
        let (id, actions) = (v.id(), std::mem::take(&mut v.base_mut().pending));
        for a in actions {
            self.send_from(path, &a, state);
            self.sent.push((id, a));
        }
    }
}

fn collect_text(v: &dyn View, out: &mut Vec<bifrost_ui::ax::AxText>) {
    if v.base().hidden {
        return;
    }
    if v.ax_node().is_some() {
        out.extend(v.ax_text());
    }
    v.children().iter().for_each(|c| collect_text(c.as_ref(), out));
}

fn collect_meta(v: &dyn View, out: &mut Vec<bifrost_ui::ax::AxMeta>) {
    if v.base().hidden {
        return;
    }
    if v.ax_node().is_some() {
        if let Some((title, help)) = v.ax_meta() {
            out.push(bifrost_ui::ax::AxMeta { id: v.id(), title, help });
        }
    }
    v.children().iter().for_each(|c| collect_meta(c.as_ref(), out));
}

fn collect_checked(v: &dyn View, out: &mut Vec<(u64, Option<bool>)>) {
    if v.base().hidden {
        return;
    }
    if v.ax_node().is_some() {
        out.push((v.id(), v.ax_checked()));
    }
    v.children().iter().for_each(|c| collect_checked(c.as_ref(), out));
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_app;

#[cfg(test)]
mod tests_controls;

/// Canvas drawing of the view tree: the image key, the image blit, backgrounds, group opacity and the display-list replay.
#[cfg(test)]
mod canvas_tests {
    use super::*;
    use crate::kit::gfx::{Image, Surface};
    use crate::kit::Font;
    use bifrost_render::display_list::DisplayList;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// A view that paints only its background (`ViewBase::background`) and nothing else.
    struct Plain {
        base: ViewBase,
    }
    impl View for Plain {
        crate::view_base!();
    }
    fn plain(bg: Option<u32>) -> Plain {
        let mut p = Plain { base: ViewBase::default() };
        p.base.background = bg;
        p
    }

    fn surface(px: &mut Vec<u32>, w: usize, h: usize, fill: u32) -> Surface<'_> {
        px.clear();
        px.resize(w * h, fill);
        Surface { w, h, stride: w, px, y0: 0 }
    }

    #[test]
    fn image_key_is_a_hash_of_the_size_and_the_pixels() {
        let a = Image { w: 2, h: 1, px: vec![1, 2] };
        let same = Image { w: 2, h: 1, px: vec![1, 2] };
        let other_pixel = Image { w: 2, h: 1, px: vec![1, 3] };
        let other_shape = Image { w: 1, h: 2, px: vec![1, 2] };
        assert_eq!(image_key(&a), image_key(&same));
        assert_ne!(image_key(&a), image_key(&other_pixel));
        assert_ne!(image_key(&a), image_key(&other_shape));
    }

    #[test]
    fn blit_draws_source_over_at_the_top_left_corner() {
        let mut px = Vec::new();
        let mut s = surface(&mut px, 4, 4, 0xff00_0000);
        let white = Image { w: 2, h: 2, px: vec![0xffff_ffff; 4] };
        blit(&mut s, &white, 1, 1);
        // columns 1..3 and rows 1..3 are white; the rest keeps its black
        assert_eq!(s.px[5], 0xffff_ffff);
        assert_eq!(s.px[6], 0xffff_ffff);
        assert_eq!(s.px[10], 0xffff_ffff);
        assert_eq!(s.px[0], 0xff00_0000);
        assert_eq!(s.px[3], 0xff00_0000);
        assert_eq!(s.px[12], 0xff00_0000);
        assert_eq!(s.px[15], 0xff00_0000);
    }

    #[test]
    fn blit_past_the_edge_is_clipped_not_wrapped() {
        let mut px = Vec::new();
        let mut s = surface(&mut px, 4, 4, 0xff00_0000);
        let white = Image { w: 2, h: 2, px: vec![0xffff_ffff; 4] };
        blit(&mut s, &white, 3, 3);
        assert_eq!(s.px[15], 0xffff_ffff, "only the corner pixel lies inside the surface");
        assert_eq!(s.px[14], 0xff00_0000, "the row is not wrapped to the next line");
        assert_eq!(s.px[0], 0xff00_0000);
    }

    #[test]
    fn a_background_fills_the_frame_opaque() {
        let ui = ui();
        let mut w: ViewTree = Window::new(plain(Some(0xff00_ff00)), 1);
        w.layout(&ui, rect(0.0, 0.0, 4.0, 4.0));
        let mut px = Vec::new();
        let mut s = surface(&mut px, 4, 4, 0xff00_0000);
        w.draw(&mut s, &ui);
        assert!(s.px.iter().all(|&p| p == 0xff00_ff00), "the whole frame is the background colour");
    }

    #[test]
    fn a_hidden_or_fully_transparent_view_paints_nothing() {
        let ui = ui();
        let mut hidden = plain(Some(0xffff_ffff));
        hidden.base.hidden = true;
        let mut faded = plain(Some(0xffff_ffff));
        faded.base.opacity = 0.0;
        for view in [hidden, faded] {
            let mut w: ViewTree = Window::new(view, 1);
            w.layout(&ui, rect(0.0, 0.0, 4.0, 4.0));
            let mut px = Vec::new();
            let mut s = surface(&mut px, 4, 4, 0xff00_0000);
            w.draw(&mut s, &ui);
            assert!(s.px.iter().all(|&p| p == DETAIL_BG.now()), "only the window background is left");
        }
    }

    #[test]
    fn group_opacity_mixes_the_subtree_with_what_is_behind_it() {
        let ui = ui();
        let mut child = plain(Some(0xffff_ffff));
        child.base.opacity = 0.5;
        let mut w: ViewTree = Window::new(Container::new(vec![Box::new(child)]), 1);
        w.layout(&ui, rect(0.0, 0.0, 4.0, 4.0));
        let mut px = Vec::new();
        let mut s = surface(&mut px, 4, 4, 0xff00_0000);
        w.draw(&mut s, &ui);
        let bg = DETAIL_BG.now();
        // the child is a 4x4 white frame at half opacity over the opaque window background: each channel is
        // (255 + bg) / 2 within rounding
        let inside = s.px[0];
        for shift in [0, 8, 16] {
            let want = (255 + ((bg >> shift) & 0xff)) / 2;
            let got = (inside >> shift) & 0xff;
            assert!(got.abs_diff(want) <= 2, "channel at shift {shift}: {got} vs {want}");
        }
    }

    #[test]
    fn replaying_the_recorded_tree_matches_drawing_it_directly() {
        let ui = ui();
        let build = || -> ViewTree {
            let kids: Vec<Box<dyn View>> = vec![Box::new(plain(Some(0xff00_ff00))), Box::new(Label::new("Hi"))];
            let mut w: ViewTree = Window::new(Container::new(kids), 1);
            w.layout(&ui, rect(0.0, 0.0, 60.0, 30.0));
            w
        };
        let (width, height) = (60usize, 30usize);
        let mut direct_px = Vec::new();
        let mut direct = surface(&mut direct_px, width, height, 0xff00_0000);
        build().draw(&mut direct, &ui);

        let mut list = DisplayList::new(width as u32, height as u32);
        build().draw(&mut list, &ui);
        let mut replayed_px = Vec::new();
        let mut replayed = surface(&mut replayed_px, width, height, 0xff00_0000);
        list.replay(&mut replayed);
        assert_eq!(direct.px, replayed.px);
    }
}
