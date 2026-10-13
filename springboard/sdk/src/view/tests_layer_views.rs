use super::layer_views::*;
use super::{View, ViewBase, ViewEvent};
use crate::kit::{Event, Font, Surface, Ui};
use crate::layer_kinds::emitter::{EmitterCell, EmitterContent, EmitterShape};
use crate::layer_kinds::gradient::{GradientContent, GradientType};
use crate::layer_kinds::replicator::{translation, Replicator, ReplicatorContent};
use crate::layer_kinds::scroll::ScrollMode;
use crate::layer_kinds::shape::ShapeContent;
use crate::layer_kinds::text::TextContent;
use crate::layer_kinds::tiled::{TileDrawing, TileGrid, TileKey};
use bifrost_render::emitter::Cell;
use bifrost_render::gfx::{rgba, Image};
use bifrost_render::cg::path::{Path, Rect as PathRect};
use bifrost_ui::Rect;

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn surface(px: &mut Vec<u32>, w: usize, h: usize) -> Surface<'_> {
    px.resize(w * h, 0);
    Surface { w, h, stride: w, px, y0: 0 }
}

/// A document view with a fixed intrinsic size.
struct Doc {
    base: ViewBase,
    size: (f32, f32),
}
impl View for Doc {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        self.size
    }
}
fn doc(w: f32, h: f32) -> Box<dyn View> {
    Box::new(Doc { base: ViewBase::default(), size: (w, h) })
}

#[test]
fn a_shape_view_draws_its_path_in_the_frame() {
    let ui = ui();
    let mut p = Path::new();
    p.add_rect(PathRect::new(2.0, 2.0, 4.0, 4.0));
    let mut content = ShapeContent::new(p);
    content.fill = Some([255, 0, 0, 255]);
    let mut v = ShapeView::new(content);
    v.layout(&ui, rect(4.0, 4.0, 8.0, 8.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 16, 16);
    v.draw(&mut s, &ui);
    assert_eq!(s.px[7 * 16 + 7], rgba(255, 0, 0, 255), "(3, 3) of the shape sits at (7, 7) in the frame at (4, 4)");
    assert_eq!(s.px[5 * 16 + 5], 0);
    assert_eq!(s.px[0], 0);
}

#[test]
fn changing_the_content_redraws_it() {
    let ui = ui();
    let mut p = Path::new();
    p.add_rect(PathRect::new(0.0, 0.0, 8.0, 8.0));
    let mut v = ShapeView::new(ShapeContent::new(p));
    v.layout(&ui, rect(0.0, 0.0, 8.0, 8.0));
    let mut px = Vec::new();
    {
        let mut s = surface(&mut px, 8, 8);
        v.draw(&mut s, &ui);
        assert_eq!(s.px[0], rgba(0, 0, 0, 255));
    }
    v.content_mut().fill = Some([0, 255, 0, 255]);
    assert!(v.needs_display());
    let mut px = Vec::new();
    let mut s = surface(&mut px, 8, 8);
    v.draw(&mut s, &ui);
    assert_eq!(s.px[0], rgba(0, 255, 0, 255));
}

#[test]
fn a_gradient_view_runs_its_ramp_across_the_frame() {
    let ui = ui();
    let mut v = GradientView::new(GradientContent::new(GradientType::Axial, vec![[0, 0, 0, 255], [255, 255, 255, 255]]));
    v.layout(&ui, rect(0.0, 0.0, 4.0, 16.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 4, 16);
    v.draw(&mut s, &ui);
    assert!((s.px[0] & 255) < 20);
    assert!((s.px[15 * 4] & 255) > 235);
}

#[test]
fn an_emitter_view_steps_on_tick_and_draws_particles() {
    let ui = ui();
    let cell = EmitterCell { base: Cell { birth_rate: 60.0, lifetime: 5.0, ..Default::default() }, size: 6.0, ..Default::default() };
    let mut content = EmitterContent::new(vec![cell], 1);
    content.position = (8.0, 8.0);
    content.shape = EmitterShape::Point;
    let mut v = EmitterView::new(content);
    v.layout(&ui, rect(0.0, 0.0, 16.0, 16.0));
    assert!(v.wants_tick());
    v.tick(0.0);
    v.tick(0.1);
    assert!(!v.content().particles.is_empty());
    let mut px = Vec::new();
    let mut s = surface(&mut px, 16, 16);
    v.draw(&mut s, &ui);
    assert_ne!(s.px[8 * 16 + 8], 0);
    v.running = false;
    assert!(!v.wants_tick());
}

#[test]
fn a_horizontal_scroll_layer_pans_sideways_only() {
    let ui = ui();
    let mut v = ScrollLayerView::new(doc(300.0, 100.0), ScrollMode::Horizontally);
    v.layout(&ui, rect(0.0, 0.0, 100.0, 100.0));
    assert_eq!(v.max_offset(), (200.0, 0.0));
    v.tick(0.0);
    assert!(v.handle(&ViewEvent::Pointer(Event::Down(80.0, 50.0))));
    v.tick(0.02);
    v.handle(&ViewEvent::Pointer(Event::Move(60.0, 20.0)));
    v.tick(0.04);
    v.handle(&ViewEvent::Pointer(Event::Move(20.0, 20.0)));
    let (x, y) = v.child_offset();
    assert!(x > 40.0 && x < 60.0, "dragged left by 60 less the 12 point slop: {x}");
    assert_eq!(y, 0.0, "the vertical axis is locked");
}

#[test]
fn a_vertical_scroll_layer_ignores_sideways_drags() {
    let ui = ui();
    let mut v = ScrollLayerView::new(doc(300.0, 300.0), ScrollMode::Vertically);
    v.layout(&ui, rect(0.0, 0.0, 100.0, 100.0));
    assert_eq!(v.max_offset(), (0.0, 200.0), "the document is as wide as the view");
    v.tick(0.0);
    v.handle(&ViewEvent::Pointer(Event::Down(80.0, 80.0)));
    v.tick(0.02);
    v.handle(&ViewEvent::Pointer(Event::Move(20.0, 40.0)));
    let (x, y) = v.child_offset();
    assert_eq!(x, 0.0);
    assert!(y > 0.0);
}

#[test]
fn both_axes_scroll_to_a_rect() {
    let ui = ui();
    let mut v = ScrollLayerView::new(doc(300.0, 300.0), ScrollMode::Both);
    v.layout(&ui, rect(0.0, 0.0, 100.0, 100.0));
    v.scroll_to_rect(rect(250.0, 10.0, 20.0, 20.0));
    assert_eq!(v.child_offset(), (170.0, 0.0));
    v.scroll_to_point((10.0, 500.0));
    assert_eq!(v.child_offset(), (10.0, 200.0));
    v.set_mode(ScrollMode::None);
    assert_eq!(v.child_offset(), (0.0, 0.0));
}

#[test]
fn a_scroll_layer_clips_in_both_directions() {
    let v = ScrollLayerView::new(doc(10.0, 10.0), ScrollMode::Both);
    assert!(v.clips_children() && v.clips_horizontally());
}

#[test]
fn a_tiled_view_draws_the_tiles_it_needs() {
    let ui = ui();
    let grid = TileGrid { content: (64.0, 64.0), tile: (32, 32), levels: 1, bias: 0 };
    let source = |_k: TileKey, size: (usize, usize), _z: f32| Image { w: size.0, h: size.1, px: vec![rgba(0, 0, 255, 255); size.0 * size.1] };
    let mut v = TiledView::new(grid, source, TileDrawing::Sync);
    v.layout(&ui, rect(0.0, 0.0, 64.0, 64.0));
    assert_eq!(v.intrinsic(&ui), (64.0, 64.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 64, 64);
    v.draw(&mut s, &ui);
    assert_eq!(s.px[40 * 64 + 40], rgba(0, 0, 255, 255));
    assert_eq!(s.px[0], rgba(0, 0, 255, 255));
}

#[test]
fn a_text_layer_view_draws_wrapped_text() {
    let ui = ui();
    let mut t = TextContent::new("Hello world", 13.0, rgba(255, 255, 255, 255));
    t.wrapped = true;
    let mut v = TextLayerView::new(t);
    v.layout(&ui, rect(0.0, 0.0, 60.0, 40.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 60, 40);
    v.draw(&mut s, &ui);
    assert!(s.px.iter().any(|&p| p != 0), "glyphs were drawn");
}

#[test]
fn a_replicator_view_draws_a_source_several_times() {
    let ui = ui();
    let mut p = Path::new();
    p.add_rect(PathRect::new(0.0, 0.0, 2.0, 2.0));
    let mut sh = ShapeContent::new(p);
    sh.fill = Some([255, 255, 255, 255]);
    let r = Replicator { count: 3, transform: translation(5.0, 0.0, 0.0), ..Default::default() };
    let mut v = ReplicatorView::new(ReplicatorContent::new(r, (0.0, 0.0)), ReplicaSource::Shape(sh));
    v.layout(&ui, rect(0.0, 0.0, 16.0, 4.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 16, 4);
    v.draw(&mut s, &ui);
    assert_ne!(s.px[0], 0);
    assert_ne!(s.px[5], 0);
    assert_ne!(s.px[10], 0);
    assert_eq!(s.px[3], 0);
}
