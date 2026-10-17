//! Embedded 2D scene view (the SwiftUI `SpriteView` content): a [`View`] that hosts a node tree
//! (`bifrost_app::framework_kits::spritekit::sk_node::NodeTree`) with the scene loop that drives it, and draws the tree's
//! shapes, labels and images into the app window every frame.
//!
//! The view owns the scene: its logical size and how it is scaled and placed in the view (`Scene2d`, with the four
//! scale modes and the anchor), the node tree, the frame clock (`SceneLoop`) and the node actions. Coordinates follow
//! the reference scene: the origin is the middle of the scene, x grows to the right and y grows up, and the scene's
//! bottom-left corner sits where the scale mode and anchor put it in the view. Nodes have no colour of their own in the
//! model, so the view keeps one per node ([`SpriteWidget::set_color`]); the default is the accent colour for shapes and
//! the label colour for text. Images are registered by name ([`SpriteWidget::add_image`]) and a node's
//! `Content::Image { name, .. }` draws the registered pixels in the node's box.
//!
//! Each frame the window calls [`View::tick`] with the time in seconds; the loop turns that into a clamped step and the
//! ordered phases of one frame (update, actions, physics, constraints, finish). The view runs the actions itself and
//! hands every phase to the closure given to [`SpriteWidget::on_step`], which is where the app moves its nodes and runs
//! its physics. Reports that actions fired are read with [`SpriteWidget::take_reports`]. A paused scene runs no phases
//! and asks for no more frames ([`View::wants_tick`]).
//!
//! A click that lands on a node sends [`ACTION_TOUCH`]: [`SpriteWidget::touched`] is the topmost node under the pointer
//! (a rotated node is hit by its rotated box, a circle by its disc) and [`SpriteWidget::touch_point`] the point in scene
//! coordinates. Rectangles and images are drawn rotated and scaled as the node tree says; a label is drawn upright and
//! centred in its box, and shader nodes are not supported.
use super::controls::node as ax_node;
use super::{View, ViewBase, ViewEvent};
use crate::kit::gfx::Image;
use crate::kit::ui::{Weight, DETAIL_BG, LABEL, TINT};
use crate::kit::{Event, Ui};
use bifrost_render::canvas::{Canvas, ImageRef, RectF, Transform2D};
use bifrost_ui::scene2d::{Scene2d, ScaleMode};
use bifrost_ui::scene_loop::{Frame, SceneLoop, Step};
use bifrost_ui::sk_node::{Content, DrawQuad, NodeId, NodeTree, Point, Size};
use bifrost_ui::{Node, Role};
use std::collections::HashMap;

/// The action sent when a click lands on a node; read the node with [`SpriteWidget::touched`].
pub const ACTION_TOUCH: &str = "sprite.touch";

/// A label's text is this share of its box height.
const LABEL_TEXT_SHARE: f32 = 0.7;

/// What the app does in one phase of a frame: the phase, the node tree, and the frame (its step and the clock).
pub type StepFn = Box<dyn FnMut(Step, &mut NodeTree, &Frame)>;

/// A node's box placed in the window, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub node: NodeId,
    pub content: Content,
    pub alpha: f32,
    /// The box corners in the node's axes order, bottom-left first when y points up: window points.
    pub corners: [(f32, f32); 4],
}

impl Placed {
    /// The box's centre.
    pub fn centre(&self) -> (f32, f32) {
        let c = &self.corners;
        ((c[0].0 + c[2].0) / 2.0, (c[0].1 + c[2].1) / 2.0)
    }

    /// Whether the window point `(x, y)` is on the node: inside its rotated box, or inside its disc for a circle.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if let Content::Circle(_) = self.content {
            let (cx, cy) = self.centre();
            let r = ((self.corners[1].0 - self.corners[0].0).hypot(self.corners[1].1 - self.corners[0].1)
                + (self.corners[3].0 - self.corners[0].0).hypot(self.corners[3].1 - self.corners[0].1))
                / 4.0;
            return (x - cx).hypot(y - cy) <= r;
        }
        // Inside a convex quad: the point is on the same side of every edge.
        let mut sign = 0.0f32;
        for i in 0..4 {
            let (a, b) = (self.corners[i], self.corners[(i + 1) % 4]);
            let cross = (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
            if cross == 0.0 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        sign != 0.0
    }
}

/// A 2D scene shown in a window.
pub struct SpriteWidget {
    base: ViewBase,
    tree: NodeTree,
    scene: Scene2d,
    looper: SceneLoop,
    step_fn: Option<StepFn>,
    reports: Vec<(NodeId, u32)>,
    colors: HashMap<NodeId, u32>,
    images: HashMap<String, (u64, Image)>,
    image_count: u64,
    /// The node the pointer went down on, and where.
    pressed: Option<(Option<NodeId>, f32, f32)>,
    touched: Option<NodeId>,
    touch_point: Option<Point>,
}

impl SpriteWidget {
    /// A scene of `width` by `height` scene units, scaled to fit the view with the whole scene visible and centred
    /// (`ScaleMode::AspectFit`, anchor in the middle). The scene is presented at once, so its clock runs.
    pub fn new(width: f32, height: f32) -> Self {
        let mut scene = Scene2d::new(width, height);
        scene.set_scale_mode(ScaleMode::AspectFit);
        scene.set_anchor(0.5, 0.5);
        let mut looper = SceneLoop::new(width, height);
        looper.present();
        SpriteWidget {
            base: ViewBase::default(),
            tree: NodeTree::new(),
            scene,
            looper,
            step_fn: None,
            reports: Vec::new(),
            colors: HashMap::new(),
            images: HashMap::new(),
            image_count: 0,
            pressed: None,
            touched: None,
            touch_point: None,
        }
    }

    pub fn tree(&self) -> &NodeTree {
        &self.tree
    }

    /// The node tree, for the app to add and move nodes. The view redraws.
    pub fn tree_mut(&mut self) -> &mut NodeTree {
        self.base.dirty = true;
        &mut self.tree
    }

    pub fn scene(&self) -> &Scene2d {
        &self.scene
    }

    /// `(width, height)` of the scene in scene units.
    pub fn scene_size(&self) -> (f32, f32) {
        self.scene.size()
    }

    pub fn set_scale_mode(&mut self, mode: ScaleMode) {
        self.scene.set_scale_mode(mode);
        self.base.dirty = true;
    }

    /// Where the scene sits in the free space of the view: `(0, 0)` bottom-left, `(0.5, 0.5)` centred.
    pub fn set_anchor(&mut self, x: f32, y: f32) {
        self.scene.set_anchor(x, y);
        self.base.dirty = true;
    }

    pub fn looper(&self) -> &SceneLoop {
        &self.looper
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.looper.set_paused(paused);
    }

    pub fn is_paused(&self) -> bool {
        self.looper.is_paused()
    }

    /// Sets what the app does in each phase of a frame.
    pub fn on_step(&mut self, f: impl FnMut(Step, &mut NodeTree, &Frame) + 'static) {
        self.step_fn = Some(Box::new(f));
    }

    /// The reports that node actions fired since the last call, as `(node, report id)`.
    pub fn take_reports(&mut self) -> Vec<(NodeId, u32)> {
        std::mem::take(&mut self.reports)
    }

    /// The colour a node's shape or text is drawn in, packed like every kit colour.
    pub fn set_color(&mut self, node: NodeId, color: u32) {
        self.colors.insert(node, color);
        self.base.dirty = true;
    }

    /// Registers the pixels `Content::Image { name, .. }` nodes draw.
    pub fn add_image(&mut self, name: &str, image: Image) {
        self.image_count += 1;
        let id = self.id().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.image_count;
        self.images.insert(name.to_string(), (id, image));
        self.base.dirty = true;
    }

    /// The node the last click landed on; `None` when it hit no node.
    pub fn touched(&self) -> Option<NodeId> {
        self.touched
    }

    /// Where the last click was, in scene coordinates (the camera applied).
    pub fn touch_point(&self) -> Option<Point> {
        self.touch_point
    }

    // ---- geometry ----

    /// Tells the scene how big the view is now, and the loop how big the scene is.
    fn sync(&mut self) {
        let f = self.base.frame;
        self.scene.set_view_size(f.w, f.h);
        let (w, h) = self.scene.size();
        self.looper.resize(w, h);
    }

    /// The visible nodes in paint order, with their boxes in window points.
    pub fn placed(&mut self) -> Vec<Placed> {
        self.sync();
        let f = self.base.frame;
        let (w, h) = self.scene.size();
        let to_window = |scene: &Scene2d, p: Point| {
            let (vx, vy) = scene.scene_to_view(p.x as f32, p.y as f32);
            (f.x + vx, f.y + f.h - vy)
        };
        self.tree
            .draw_quads(Size { width: w as f64, height: h as f64 })
            .into_iter()
            .map(|q: DrawQuad| Placed {
                node: q.node,
                content: q.content,
                alpha: q.alpha as f32,
                corners: [to_window(&self.scene, q.corners[0]), to_window(&self.scene, q.corners[1]), to_window(&self.scene, q.corners[2]), to_window(&self.scene, q.corners[3])],
            })
            .collect()
    }

    /// The topmost node at the window point `(x, y)`.
    pub fn node_at(&mut self, x: f32, y: f32) -> Option<NodeId> {
        self.placed().iter().rev().find(|p| p.contains(x, y)).map(|p| p.node)
    }

    /// The window point `(x, y)` in scene coordinates.
    pub fn scene_point(&mut self, x: f32, y: f32) -> Option<Point> {
        self.sync();
        let f = self.base.frame;
        let (sx, sy) = self.scene.view_to_scene(x - f.x, f.y + f.h - y);
        let (w, h) = self.scene.size();
        self.tree.scene_point(Point { x: sx as f64, y: sy as f64 }, Size { width: w as f64, height: h as f64 })
    }

    fn shape_color(&self, node: NodeId, label: bool) -> u32 {
        self.colors.get(&node).copied().unwrap_or(if label { LABEL.now() } else { TINT })
    }
}

/// The transform that maps a box of `w` by `h` device pixels, drawn with its top left at the origin and y growing
/// down, onto the quad whose corners (bottom-left, bottom-right, top-right, top-left) are `px`.
fn quad_transform(px: &[(f32, f32); 4]) -> (Transform2D, f32, f32) {
    let (tl, tr, bl) = (px[3], px[2], px[0]);
    let w = (tr.0 - tl.0).hypot(tr.1 - tl.1).max(1e-3);
    let h = (bl.0 - tl.0).hypot(bl.1 - tl.1).max(1e-3);
    let t = Transform2D { a: (tr.0 - tl.0) / w, b: (tr.1 - tl.1) / w, c: (bl.0 - tl.0) / h, d: (bl.1 - tl.1) / h, tx: tl.0, ty: tl.1 };
    (t, w, h)
}

impl View for SpriteWidget {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn accepts_first_responder(&self) -> bool {
        false
    }

    fn tick(&mut self, t: f64) {
        let Some(frame) = self.looper.frame(t) else { return };
        if frame.steps.is_empty() {
            return;
        }
        for step in frame.steps {
            if let Some(f) = self.step_fn.as_mut() {
                f(*step, &mut self.tree, &frame);
            }
            if *step == Step::EvaluateActions {
                let fired = self.tree.advance_actions(frame.delta);
                self.reports.extend(fired);
            }
        }
        self.base.dirty = true;
    }

    fn wants_tick(&self) -> bool {
        self.looper.is_presented() && !self.looper.is_paused()
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let area = RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h));
        s.fill_rect(area, DETAIL_BG.now());
        s.push_clip(area, 0.0);
        for p in self.placed() {
            let px: [(f32, f32); 4] = [
                (ui.pt(p.corners[0].0), ui.pt(p.corners[0].1)),
                (ui.pt(p.corners[1].0), ui.pt(p.corners[1].1)),
                (ui.pt(p.corners[2].0), ui.pt(p.corners[2].1)),
                (ui.pt(p.corners[3].0), ui.pt(p.corners[3].1)),
            ];
            let (t, w, h) = quad_transform(&px);
            s.push_opacity(p.alpha);
            match &p.content {
                Content::Empty => {}
                Content::Rect(_) => {
                    s.push_transform(t);
                    s.fill_rect(RectF::new(0.0, 0.0, w, h), self.shape_color(p.node, false));
                    s.pop_transform();
                }
                Content::Circle(_) => {
                    let (cx, cy) = ((px[0].0 + px[2].0) / 2.0, (px[0].1 + px[2].1) / 2.0);
                    let r = (w + h) / 4.0;
                    s.fill_rrect(RectF::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, self.shape_color(p.node, false));
                }
                Content::Label { text, .. } => {
                    let pt = (h * LABEL_TEXT_SHARE / ui.pt(1.0)).max(1.0);
                    let img = ui.text(&ui.fit(text, pt, w), pt, Weight::Regular, self.shape_color(p.node, true));
                    let (cx, cy) = ((px[0].0 + px[2].0) / 2.0, (px[0].1 + px[2].1) / 2.0);
                    let dst = RectF::new(cx - img.w as f32 / 2.0, cy - img.h as f32 / 2.0, img.w as f32, img.h as f32);
                    s.image(ImageRef { id: super::image_key(&img), image: &img }, dst, 1.0);
                }
                Content::Image { name, .. } => {
                    if let Some((id, img)) = self.images.get(name) {
                        s.push_transform(t);
                        s.image(ImageRef { id: *id, image: img }, RectF::new(0.0, 0.0, w, h), 1.0);
                        s.pop_transform();
                    }
                }
            }
            s.pop_opacity();
        }
        s.pop_clip();
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) if self.base.frame.contains(*x, *y) => {
                let node = self.node_at(*x, *y);
                self.pressed = Some((node, *x, *y));
                true
            }
            ViewEvent::Pointer(Event::Move(..)) => self.pressed.is_some(),
            ViewEvent::Pointer(Event::Up(x, y)) => {
                let Some((down_on, ..)) = self.pressed.take() else { return false };
                if self.base.frame.contains(*x, *y) {
                    let up_on = self.node_at(*x, *y);
                    // A click ends on the node it began on. A press that slid off one node onto another is not one.
                    if up_on == down_on {
                        self.touched = up_on;
                        self.touch_point = self.scene_point(*x, *y);
                        self.base.send_action(ACTION_TOUCH);
                    }
                }
                true
            }
            ViewEvent::Cancel => self.pressed.take().is_some(),
            _ => false,
        }
    }

    fn ax_node(&self) -> Option<Node> {
        let shown = self.tree.draw_list(Size { width: self.scene.size().0 as f64, height: self.scene.size().1 as f64 }).len();
        ax_node(self, Role::StaticText, "Game scene", &format!("{shown} objects"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::gfx::pack;
    use crate::kit::{Font, Surface};
    use bifrost_ui::responder::Responder;
    use bifrost_ui::sk_action::Action;
    use bifrost_ui::Rect;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    const RED: u32 = pack(255, 0, 0, 255);
    const GREEN: u32 = pack(0, 255, 0, 255);

    fn size(w: f64, h: f64) -> Size {
        Size { width: w, height: h }
    }

    /// A 200 by 100 scene shown 1:1 in a 200 by 100 frame.
    fn widget() -> SpriteWidget {
        let mut w = SpriteWidget::new(200.0, 100.0);
        w.set_frame(Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 });
        w
    }

    fn paint(w: &mut SpriteWidget, width: usize, height: usize) -> Vec<u32> {
        let ui = ui();
        let mut px = vec![0u32; width * height];
        {
            let mut s = Surface { w: width, h: height, stride: width, px: &mut px, y0: 0 };
            w.draw(&mut s, &ui);
        }
        px
    }

    fn click(w: &mut SpriteWidget, x: f32, y: f32) {
        w.handle(&ViewEvent::Pointer(Event::Down(x, y)));
        w.handle(&ViewEvent::Pointer(Event::Up(x, y)));
    }

    #[test]
    fn a_rectangle_is_drawn_at_the_scene_origin_in_the_middle_of_the_view_in_its_colour() {
        let mut w = widget();
        let root = w.tree().root();
        let r = w.tree_mut().add(root, Content::Rect(size(40.0, 20.0))).unwrap();
        w.set_color(r, RED);
        let px = paint(&mut w, 200, 100);
        assert_eq!(px[50 * 200 + 100], RED, "the middle");
        assert_eq!(px[50 * 200 + 85], RED);
        assert_ne!(px[50 * 200 + 70], RED, "left of the 40 point wide box");
        assert_ne!(px[5 * 200 + 5], RED);
    }

    #[test]
    fn scene_y_points_up_so_a_node_above_the_origin_is_drawn_higher() {
        let mut w = widget();
        let root = w.tree().root();
        let up = w.tree_mut().add(root, Content::Rect(size(20.0, 20.0))).unwrap();
        w.tree_mut().set_position(up, Point { x: 0.0, y: 30.0 });
        w.set_color(up, RED);
        let px = paint(&mut w, 200, 100);
        assert_eq!(px[20 * 200 + 100], RED, "30 above the middle row (50)");
        assert_ne!(px[80 * 200 + 100], RED, "nothing 30 below it");
    }

    #[test]
    fn a_circle_is_a_disc() {
        let mut w = widget();
        let root = w.tree().root();
        let c = w.tree_mut().add(root, Content::Circle(20.0)).unwrap();
        w.set_color(c, GREEN);
        let px = paint(&mut w, 200, 100);
        assert_eq!(px[50 * 200 + 100], GREEN);
        assert_eq!(px[50 * 200 + 115], GREEN);
        assert_ne!(px[32 * 200 + 86], GREEN, "the corner of the bounding square is outside the disc");
    }

    #[test]
    fn a_rotated_rectangle_is_drawn_rotated() {
        let mut w = widget();
        let root = w.tree().root();
        let r = w.tree_mut().add(root, Content::Rect(size(60.0, 10.0))).unwrap();
        w.tree_mut().set_rotation(r, std::f64::consts::FRAC_PI_2);
        w.set_color(r, RED);
        let px = paint(&mut w, 200, 100);
        assert_eq!(px[25 * 200 + 100], RED, "a quarter turn makes the long side vertical");
        assert_ne!(px[50 * 200 + 75], RED, "and leaves the old horizontal reach empty");
    }

    #[test]
    fn aspect_fit_centres_a_scene_that_is_narrower_than_the_view() {
        let mut w = SpriteWidget::new(100.0, 100.0);
        w.set_frame(Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 });
        let root = w.tree().root();
        let r = w.tree_mut().add(root, Content::Rect(size(100.0, 100.0))).unwrap();
        w.set_color(r, RED);
        let px = paint(&mut w, 300, 100);
        assert_eq!(px[50 * 300 + 150], RED, "the middle of the view");
        assert_eq!(px[50 * 300 + 105], RED, "the scene's left edge is at 100");
        assert_ne!(px[50 * 300 + 90], RED, "bars either side of the scene");
        assert_ne!(px[50 * 300 + 210], RED);
        w.set_scale_mode(ScaleMode::Fill);
        let px = paint(&mut w, 300, 100);
        assert_eq!(px[50 * 300 + 10], RED, "Fill stretches the scene over the view");
    }

    #[test]
    fn a_click_picks_the_topmost_node_and_reports_the_scene_point() {
        let mut w = widget();
        let root = w.tree().root();
        let back = w.tree_mut().add(root, Content::Rect(size(80.0, 80.0))).unwrap();
        let front = w.tree_mut().add(root, Content::Rect(size(40.0, 40.0))).unwrap();
        w.tree_mut().set_z_position(front, 1.0);
        click(&mut w, 100.0, 50.0);
        assert_eq!(w.touched(), Some(front));
        click(&mut w, 100.0 + 30.0, 50.0);
        assert_eq!(w.touched(), Some(back), "outside the front box, inside the back one");
        let p = w.touch_point().unwrap();
        assert!((p.x - 30.0).abs() < 1e-3 && p.y.abs() < 1e-3, "{p:?}");
        click(&mut w, 100.0, 5.0);
        assert_eq!(w.touched(), None);
        click(&mut w, 100.0, 50.0 - 10.0);
        assert!((w.touch_point().unwrap().y - 10.0).abs() < 1e-3, "above the middle is positive y");
    }

    #[test]
    fn a_rotated_box_and_a_circle_are_hit_by_their_own_shape() {
        let mut w = widget();
        let root = w.tree().root();
        let bar = w.tree_mut().add(root, Content::Rect(size(60.0, 6.0))).unwrap();
        w.tree_mut().set_rotation(bar, std::f64::consts::FRAC_PI_2);
        assert_eq!(w.node_at(100.0, 25.0), Some(bar), "the turned bar reaches 30 above the middle");
        assert_eq!(w.node_at(75.0, 50.0), None, "and no longer 25 to the left");
        let disc = w.tree_mut().add(root, Content::Circle(10.0)).unwrap();
        w.tree_mut().set_position(disc, Point { x: -60.0, y: 0.0 });
        assert_eq!(w.node_at(40.0, 50.0), Some(disc));
        assert_eq!(w.node_at(40.0 + 9.0, 50.0 - 9.0), None, "inside the bounding square, outside the disc");
    }

    #[test]
    fn a_click_sends_one_action_and_a_press_that_slides_to_another_node_is_not_a_click() {
        #[derive(Default)]
        struct Model {
            log: Vec<String>,
        }
        impl Responder for Model {
            fn perform(&mut self, action: &str) -> bool {
                self.log.push(action.into());
                true
            }
        }
        let mut w = widget();
        let root = w.tree().root();
        let a = w.tree_mut().add(root, Content::Rect(size(40.0, 40.0))).unwrap();
        let b = w.tree_mut().add(root, Content::Rect(size(20.0, 20.0))).unwrap();
        w.tree_mut().set_position(b, Point { x: 60.0, y: 0.0 });
        let id = w.id();
        let mut win: super::super::Window<Model> = super::super::Window::new(w, 1);
        win.layout(&ui(), Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 });
        let mut m = Model::default();
        win.event(&ViewEvent::Pointer(Event::Down(100.0, 50.0)), &mut m);
        win.event(&ViewEvent::Pointer(Event::Up(100.0, 50.0)), &mut m);
        assert_eq!(m.log, [ACTION_TOUCH]);
        assert_eq!(win.view::<SpriteWidget>(id).unwrap().touched(), Some(a));
        win.event(&ViewEvent::Pointer(Event::Down(100.0, 50.0)), &mut m);
        win.event(&ViewEvent::Pointer(Event::Up(160.0, 50.0)), &mut m);
        assert_eq!(m.log.len(), 1, "down on one node and up on another is not a click on either");
        let _ = b;
    }

    #[test]
    fn the_loop_runs_the_phases_in_order_with_the_clamped_step_and_the_app_moves_a_node() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut w = widget();
        let root = w.tree().root();
        let ball = w.tree_mut().add(root, Content::Circle(5.0)).unwrap();
        let seen: Rc<RefCell<Vec<(Step, f64)>>> = Rc::default();
        let log = seen.clone();
        w.on_step(move |step, tree, frame| {
            log.borrow_mut().push((step, frame.delta));
            if step == Step::Update {
                tree.set_position(ball, Point { x: 10.0 * frame.current_time, y: 0.0 });
            }
        });
        w.tick(1.0);
        assert_eq!(seen.borrow().iter().map(|s| s.0).collect::<Vec<_>>(), bifrost_ui::scene_loop::FRAME_STEPS, "the first frame runs every phase");
        assert_eq!(seen.borrow()[0].1, 0.0, "with no time elapsed");
        seen.borrow_mut().clear();
        w.tick(1.05);
        assert!((seen.borrow()[0].1 - 0.05).abs() < 1e-9);
        w.tick(9.0);
        assert!((seen.borrow().iter().map(|s| s.1).fold(0.0, f64::max) - 0.1).abs() < 1e-9, "a stall is clamped to the step limit");
        let at = w.tree().world_position(ball).unwrap();
        assert!((at.x - 10.0 * 0.15).abs() < 1e-9, "{at:?}");
        assert!(w.wants_tick());
    }

    #[test]
    fn a_paused_scene_runs_nothing_and_stops_asking_for_frames() {
        let mut w = widget();
        w.on_step(|_, _, _| panic!("a paused scene runs no phase"));
        w.set_paused(true);
        assert!(w.is_paused() && !w.wants_tick());
        w.tick(1.0);
        w.tick(2.0);
    }

    #[test]
    fn node_actions_advance_with_the_loop_and_their_reports_reach_the_app() {
        let mut w = widget();
        let root = w.tree().root();
        let n = w.tree_mut().add(root, Content::Empty).unwrap();
        w.tree_mut().run_action(n, &Action::Sequence(vec![Action::Wait(0.15), Action::Report(7)])).unwrap();
        w.tick(0.0);
        w.tick(0.1);
        assert!(w.take_reports().is_empty(), "0.1 s of a 0.15 s wait");
        w.tick(0.2);
        assert_eq!(w.take_reports(), [(n, 7)]);
        assert!(w.take_reports().is_empty(), "read once");
    }

    #[test]
    fn labels_and_images_are_drawn_and_hidden_or_transparent_nodes_are_not() {
        let mut w = widget();
        let root = w.tree().root();
        let label = w.tree_mut().add(root, Content::Label { text: "Score".into(), size: size(80.0, 20.0) }).unwrap();
        w.tree_mut().set_position(label, Point { x: 0.0, y: 30.0 });
        w.set_color(label, RED);
        let pic = w.tree_mut().add(root, Content::Image { name: "dot".into(), size: size(20.0, 20.0) }).unwrap();
        w.tree_mut().set_position(pic, Point { x: -60.0, y: 0.0 });
        w.add_image("dot", Image { w: 2, h: 2, px: vec![GREEN; 4] });
        let ghost = w.tree_mut().add(root, Content::Rect(size(20.0, 20.0))).unwrap();
        w.tree_mut().set_position(ghost, Point { x: 60.0, y: 0.0 });
        w.set_color(ghost, RED);
        w.tree_mut().set_alpha(ghost, 0.0);
        let px = paint(&mut w, 200, 100);
        let reddish = |p: u32| (p & 0xFF) > 100 && ((p >> 8) & 0xFF) < 60 && ((p >> 16) & 0xFF) < 60;
        assert!(px[10 * 200..30 * 200].iter().any(|&p| reddish(p)), "the label's glyphs are red");
        assert_eq!(px[50 * 200 + 40], GREEN, "the image fills its box");
        assert_ne!(px[50 * 200 + 160], RED, "a node with alpha 0 does not draw");
        assert!(!w.placed().iter().any(|p| p.node == ghost));
    }
}
