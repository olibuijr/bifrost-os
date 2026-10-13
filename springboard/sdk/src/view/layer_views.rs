//! Views that hold one layer content kind (`crate::layer_kinds`) and draw its rendered image: `ShapeView`,
//! `GradientView`, `TextLayerView`, `ReplicatorView`, `EmitterView`, `TiledView` and the two-axis `ScrollLayerView`.
//! Each view re-renders only when its content, its size or the display scale changed.
use super::{image_key, View, ViewBase, ViewEvent};
use crate::kit::ui::LABEL_2;
use crate::kit::{Event, Ui};
use crate::layer_kinds::emitter::EmitterContent;
use crate::layer_kinds::gradient::GradientContent;
use crate::layer_kinds::replicator::ReplicatorContent;
use crate::layer_kinds::scroll::{ScrollContent, ScrollMode};
use crate::layer_kinds::shape::ShapeContent;
use crate::layer_kinds::text::TextContent;
use crate::layer_kinds::tiled::{TileDrawing, TileGrid, TileSource, TiledContent};
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_render::gfx::Image;
use bifrost_ui::scroll::Scroller;
use bifrost_ui::Rect;

/// Device pixel size of a frame, and the origin it is drawn at.
fn device(ui: &Ui, f: Rect) -> (RectF, usize, usize) {
    let (w, h) = (ui.pt(f.w).round().max(0.0), ui.pt(f.h).round().max(0.0));
    (RectF::new(ui.pt(f.x).round(), ui.pt(f.y).round(), w, h), w as usize, h as usize)
}

fn draw_image(s: &mut dyn Canvas, dst: RectF, img: &Image) {
    if img.w > 0 && img.h > 0 {
        s.image(ImageRef { id: image_key(img), image: img }, dst, 1.0);
    }
}

/// A rendered image and what it was rendered for.
#[derive(Default)]
struct Rendered {
    key: Option<(usize, usize, u64)>,
    img: Option<Image>,
}

impl Rendered {
    fn get(&mut self, key: (usize, usize, u64), make: impl FnOnce() -> Image) -> &Image {
        if self.key != Some(key) || self.img.is_none() {
            self.img = Some(make());
            self.key = Some(key);
        }
        self.img.as_ref().expect("rendered above")
    }
    fn clear(&mut self) {
        self.key = None;
        self.img = None;
    }
}

macro_rules! content_accessors {
    ($ty:ty) => {
        /// The content to read.
        pub fn content(&self) -> &$ty {
            &self.content
        }
        /// The content to change; the view re-renders on the next frame.
        pub fn content_mut(&mut self) -> &mut $ty {
            self.cache.clear();
            self.base.dirty = true;
            &mut self.content
        }
    };
}

/// A path with a fill and a stroke.
pub struct ShapeView {
    base: ViewBase,
    content: ShapeContent,
    cache: Rendered,
}

impl ShapeView {
    pub fn new(content: ShapeContent) -> Self {
        ShapeView { base: ViewBase::default(), content, cache: Rendered::default() }
    }
    content_accessors!(ShapeContent);
}

impl View for ShapeView {
    crate::view_base!();
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (dst, w, h) = device(ui, self.base.frame);
        let scale = ui.pt(1.0);
        let content = &self.content;
        let img = self.cache.get((w, h, scale.to_bits() as u64), || content.render(w, h, scale));
        draw_image(s, dst, img);
    }
}

/// A colour ramp, axial, radial or conic.
pub struct GradientView {
    base: ViewBase,
    content: GradientContent,
    cache: Rendered,
}

impl GradientView {
    pub fn new(content: GradientContent) -> Self {
        GradientView { base: ViewBase::default(), content, cache: Rendered::default() }
    }
    content_accessors!(GradientContent);
}

impl View for GradientView {
    crate::view_base!();
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (dst, w, h) = device(ui, self.base.frame);
        let content = &self.content;
        let img = self.cache.get((w, h, 0), || content.render(w, h));
        draw_image(s, dst, img);
    }
}

/// Text in a box with wrapping, alignment and truncation modes.
pub struct TextLayerView {
    base: ViewBase,
    content: TextContent,
    cache: Rendered,
}

impl TextLayerView {
    pub fn new(content: TextContent) -> Self {
        TextLayerView { base: ViewBase::default(), content, cache: Rendered::default() }
    }
    content_accessors!(TextContent);
}

impl View for TextLayerView {
    crate::view_base!();
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (dst, w, h) = device(ui, self.base.frame);
        let content = &self.content;
        let img = self.cache.get((w, h, ui.text_epoch()), || content.render(ui, w, h));
        draw_image(s, dst, img);
    }
}

/// What a replicator view copies.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplicaSource {
    Shape(ShapeContent),
    Gradient(GradientContent),
    /// A premultiplied image drawn at its own pixel size at the view's top left.
    Image(Image),
}

/// A source drawn several times, each copy one more step of a transform and colour offset along.
pub struct ReplicatorView {
    base: ViewBase,
    content: ReplicatorContent,
    source: ReplicaSource,
    cache: Rendered,
}

impl ReplicatorView {
    pub fn new(content: ReplicatorContent, source: ReplicaSource) -> Self {
        ReplicatorView { base: ViewBase::default(), content, source, cache: Rendered::default() }
    }
    content_accessors!(ReplicatorContent);
    pub fn source(&self) -> &ReplicaSource {
        &self.source
    }
    pub fn set_source(&mut self, source: ReplicaSource) {
        self.source = source;
        self.cache.clear();
        self.base.dirty = true;
    }
}

impl View for ReplicatorView {
    crate::view_base!();
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (dst, w, h) = device(ui, self.base.frame);
        let scale = ui.pt(1.0);
        let (content, source) = (&self.content, &self.source);
        let img = self.cache.get((w, h, scale.to_bits() as u64), || {
            let src = match source {
                ReplicaSource::Shape(sh) => sh.render(w, h, scale),
                ReplicaSource::Gradient(g) => g.render(w, h),
                ReplicaSource::Image(i) => i.clone(),
            };
            content.render(w, h, scale, &src)
        });
        draw_image(s, dst, img);
    }
}

/// Particles. The view advances the simulation by the frame time and redraws every frame.
pub struct EmitterView {
    base: ViewBase,
    content: EmitterContent,
    last: Option<f64>,
    /// While false the simulation stands still and the view stops asking for frames.
    pub running: bool,
}

/// The longest step the simulation takes in one frame, in seconds, so a stalled window does not burst.
const MAX_STEP: f64 = 0.1;

impl EmitterView {
    pub fn new(content: EmitterContent) -> Self {
        EmitterView { base: ViewBase::default(), content, last: None, running: true }
    }
    pub fn content(&self) -> &EmitterContent {
        &self.content
    }
    pub fn content_mut(&mut self) -> &mut EmitterContent {
        self.base.dirty = true;
        &mut self.content
    }
}

impl View for EmitterView {
    crate::view_base!();
    fn wants_tick(&self) -> bool {
        self.running
    }
    fn tick(&mut self, t: f64) {
        let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, MAX_STEP));
        self.last = Some(t);
        if self.running {
            self.content.step(dt as f32);
            self.base.dirty = true;
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let (dst, w, h) = device(ui, self.base.frame);
        let img = self.content.render(w, h, ui.pt(1.0));
        draw_image(s, dst, &img);
    }
}

/// Large content drawn in tiles, at the level of detail the view's size calls for. The frame is the content scaled:
/// a frame as wide as the content shows it at natural size. Inside a `ScrollLayerView` its `intrinsic` size is the
/// content size, so it scrolls like any document view.
pub struct TiledView {
    base: ViewBase,
    content: TiledContent,
}

impl TiledView {
    pub fn new(grid: TileGrid, source: impl TileSource, drawing: TileDrawing) -> Self {
        TiledView { base: ViewBase::default(), content: TiledContent::new(grid, source, drawing) }
    }
    pub fn content(&self) -> &TiledContent {
        &self.content
    }
    /// Drops every drawn tile and draws them again (the underlying content changed).
    pub fn invalidate(&mut self) {
        self.content.invalidate();
        self.base.dirty = true;
    }
}

impl View for TiledView {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        self.content.grid.content
    }
    fn wants_tick(&self) -> bool {
        self.content.pending() > 0
    }
    fn tick(&mut self, _t: f64) {
        if self.content.poll() {
            self.base.dirty = true;
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let grid = self.content.grid;
        if f.w <= 0.0 || f.h <= 0.0 || grid.content.0 <= 0.0 || grid.content.1 <= 0.0 {
            return;
        }
        // Device pixels per content point, and the frame's device rectangle.
        let k = ui.pt(f.w) / grid.content.0;
        let (ox, oy) = (ui.pt(f.x), ui.pt(f.y));
        let (cw, ch) = s.size();
        // The part of the content on the canvas, in content points.
        let x0 = ((0.0 - ox) / k).max(0.0);
        let y0 = ((0.0 - oy) / k).max(0.0);
        let x1 = ((cw as f32 - ox) / k).min(grid.content.0);
        let y1 = ((ch as f32 - oy) / k).min(grid.content.1);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let level = grid.level_for_zoom(k);
        let keys = self.content.request_region(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, level);
        for key in keys {
            let Some(tile) = self.content.cached(key) else { continue };
            let r = grid.tile_rect(key);
            draw_image(s, RectF::new(ox + r.x * k, oy + r.y * k, r.w * k, r.h * k), &tile);
        }
    }
}

/// Scroll indicator thickness and inset, in points.
const KNOB_PT: f32 = 3.0;
const KNOB_INSET_PT: f32 = 3.0;
const KNOB_MIN_PT: f32 = 24.0;

/// One document view in a clipping frame that scrolls along the axes of its `ScrollMode`: vertically, horizontally or
/// both. Dragging pans the content; a release flings or springs back (`bifrost_ui::scroll::Scroller` on each axis).
/// `scroll_to_point` and `scroll_to_rect` move it from code. The document is as large as its `intrinsic` size on the
/// scrolling axes and as large as the view on the others.
pub struct ScrollLayerView {
    base: ViewBase,
    kids: Vec<Box<dyn View>>,
    scroll: ScrollContent,
    hor: Scroller,
    ver: Scroller,
    now: f64,
    last: (f32, f32),
    knob: bool,
}

impl ScrollLayerView {
    pub fn new(document: Box<dyn View>, mode: ScrollMode) -> Self {
        ScrollLayerView {
            base: ViewBase::default(),
            kids: vec![document],
            scroll: ScrollContent::new(mode),
            hor: Scroller::new(),
            ver: Scroller::new(),
            now: 0.0,
            last: (0.0, 0.0),
            knob: false,
        }
    }

    pub fn mode(&self) -> ScrollMode {
        self.scroll.mode
    }

    pub fn set_mode(&mut self, mode: ScrollMode) {
        self.scroll.mode = mode;
        self.base.dirty = true;
    }

    /// The content point at the top left of the view (may be past the range mid rubber band).
    pub fn content_offset(&self) -> (f32, f32) {
        (
            if self.scroll.mode.horizontal() { self.hor.offset() } else { 0.0 },
            if self.scroll.mode.vertical() { self.ver.offset() } else { 0.0 },
        )
    }

    pub fn max_offset(&self) -> (f32, f32) {
        self.scroll.max_offset()
    }

    /// Scrolls so the content point `p` is at the top left, as far as the mode and the content allow. Stops a fling.
    pub fn scroll_to_point(&mut self, p: (f32, f32)) {
        self.scroll.scroll_to_point(p);
        self.sync_scrollers();
    }

    /// Scrolls the least that brings `r` (content points) into view.
    pub fn scroll_to_rect(&mut self, r: Rect) {
        self.scroll.scroll_to_rect(r);
        self.sync_scrollers();
    }

    fn sync_scrollers(&mut self) {
        let o = self.scroll.offset();
        self.hor.jump_to(o.0);
        self.ver.jump_to(o.1);
        self.base.dirty = true;
    }

    pub fn document_view<T: View>(&self) -> Option<&T> {
        self.kids[0].as_any().downcast_ref::<T>()
    }

    fn animating(&self) -> bool {
        self.hor.animating() || self.ver.animating() || self.hor.dragging() || self.ver.dragging()
    }
}

impl View for ScrollLayerView {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.kids
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.kids
    }
    fn child_offset(&self) -> (f32, f32) {
        self.content_offset()
    }
    fn clips_children(&self) -> bool {
        true
    }
    fn clips_horizontally(&self) -> bool {
        true
    }
    fn opaque_backdrop(&self) -> bool {
        true
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let (iw, ih) = self.kids[0].intrinsic(ui);
        let mode = self.scroll.mode;
        let w = if mode.horizontal() && iw > 0.0 { iw } else { bounds.w };
        let h = if mode.vertical() && ih > 0.0 { ih } else { bounds.h };
        self.kids[0].layout(ui, Rect { x: bounds.x, y: bounds.y, w, h });
        let doc = self.kids[0].frame();
        self.scroll.set_sizes((doc.w, doc.h), (bounds.w, bounds.h));
        let max = self.scroll.max_offset();
        self.hor.set_max(max.0);
        self.ver.set_max(max.1);
        self.base.dirty = true;
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        let mode = self.scroll.mode;
        let t = self.now * 1000.0;
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                self.last = (*x, *y);
                if mode.horizontal() {
                    self.hor.down(*x, t);
                }
                if mode.vertical() {
                    self.ver.down(*y, t);
                }
            }
            ViewEvent::Pointer(Event::Move(x, y)) => {
                self.last = (*x, *y);
                if mode.horizontal() {
                    self.hor.moved(*x, t);
                }
                if mode.vertical() {
                    self.ver.moved(*y, t);
                }
            }
            ViewEvent::Pointer(Event::Up(x, y)) => {
                if mode.horizontal() {
                    self.hor.up(*x, t);
                }
                if mode.vertical() {
                    self.ver.up(*y, t);
                }
            }
            ViewEvent::Cancel => {
                let (x, y) = self.last;
                if mode.horizontal() {
                    self.hor.up(x, t);
                }
                if mode.vertical() {
                    self.ver.up(y, t);
                }
            }
            _ => return false,
        }
        self.base.dirty = true;
        true
    }
    fn wants_tick(&self) -> bool {
        self.animating() || self.knob
    }
    fn tick(&mut self, t: f64) {
        self.now = t;
        let ms = t * 1000.0;
        self.hor.tick(ms);
        self.ver.tick(ms);
        let moving = self.animating();
        if moving || self.knob {
            self.base.dirty = true;
        }
        self.knob = moving;
        if moving {
            // Keep the model's offset in step with the physics.
            let o = self.content_offset();
            self.scroll.scroll_to_point(o);
        }
    }
    fn draw_overlay(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        if !self.knob {
            return;
        }
        let f = self.base.frame;
        let max = self.scroll.max_offset();
        let off = self.content_offset();
        let doc = self.scroll.content_size();
        let colour = LABEL_2.now();
        if self.scroll.mode.vertical() && max.1 > 0.0 {
            let len = (f.h * f.h / doc.1).max(KNOB_MIN_PT).min(f.h);
            let y = f.y + (off.1 / max.1).clamp(0.0, 1.0) * (f.h - len);
            s.fill_rrect(RectF::new(ui.pt(f.right() - KNOB_INSET_PT - KNOB_PT), ui.pt(y), ui.pt(KNOB_PT), ui.pt(len)), ui.pt(KNOB_PT / 2.0), colour);
        }
        if self.scroll.mode.horizontal() && max.0 > 0.0 {
            let len = (f.w * f.w / doc.0).max(KNOB_MIN_PT).min(f.w);
            let x = f.x + (off.0 / max.0).clamp(0.0, 1.0) * (f.w - len);
            s.fill_rrect(RectF::new(ui.pt(x), ui.pt(f.bottom() - KNOB_INSET_PT - KNOB_PT), ui.pt(len), ui.pt(KNOB_PT)), ui.pt(KNOB_PT / 2.0), colour);
        }
    }
}
