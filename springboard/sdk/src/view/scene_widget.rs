//! Embedded 3D scene view (the `SceneView` content of a declarative UI): a [`View`] that renders a
//! `bifrost_app::framework_kits::scenekit::scene3d::Scene` into the app window with the CPU triangle rasteriser
//! (`raster3d`), lets the person orbit the camera by dragging and zoom by scrolling, and finds the object under a click.
//!
//! The scene tree carries poses and one camera but no geometry, so the widget keeps a list of [`SceneDrawable`]s: each
//! ties a scene node to the triangles and the material that are drawn at that node's world pose. The widget owns the
//! scene (`scene_mut` hands it out for animation) and the camera control: an orbit around a target point, set from the
//! camera node's own pose at the start. The camera node should be a child of the scene root, because the widget writes
//! its pose in the world.
//!
//! Drawing happens in [`View::draw`] and only when something changed (the scene, a drawable, the light, the camera or the
//! size), so an idle scene costs one image blit a frame. The picture is rendered at the view's size in device pixels,
//! at most [`MAX_PIXELS`] of them, and scaled to the frame. Rendering is flat shaded per triangle with a depth buffer;
//! triangles that cross the camera's near plane are dropped rather than clipped, and a single-sided drawable shows
//! only the faces that look at the camera. Where nothing is drawn the view's own background shows through, unless
//! [`SceneWidget::set_background`] gives an opaque colour.
//!
//! Input: dragging orbits (left and right turn around the target, up and down tilt, never past the poles), a click that
//! barely moves picks (see [`SceneWidget::picked`], the action [`ACTION_PICK`] is sent), `+` and `-` zoom, the arrow keys
//! orbit. Scrolling zooms through [`SceneWidget::scroll`]; the SDK delivers no wheel events to views yet, so an app that
//! has a wheel or pinch source calls it. Picking tests a bounding sphere around each drawable's origin, because the
//! `SceneView` model hit-tests spheres; a flat plane is picked by its sphere too.
//!
//! ```ignore
//! let mut view = SceneWidget::new(scene, camera_node).unwrap();
//! view.add_drawable(SceneDrawable::new(ball, sphere_geometry(1.0, 32, 16).unwrap()));
//! ```
use super::controls::{focus_ring, node};
use super::{View, ViewBase, ViewEvent};
use crate::kit::gfx::{pack, Image};
use crate::kit::ui::DETAIL_BG;
use crate::kit::{Event, Ui};
use crate::proto::KeyInput;
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_ui::linalg::{Matrix4, Quaternion, Vector3};
use bifrost_ui::primitives3d::Geometry3;
use bifrost_ui::raster3d::{self, Target, Triangle};
use bifrost_ui::scene3d::{look_at_orientation, Camera, NodeId, Pose, Scene};
use bifrost_ui::scene_view::SceneView;
use bifrost_ui::shading::{Light, LightSource, Material, Rgb};
use bifrost_ui::{Node, Role};

/// The action sent when a click picked (or missed) an object; read the result with [`SceneWidget::picked`].
pub const ACTION_PICK: &str = "scene.pick";

/// The most device pixels the rasteriser fills in one picture. A larger view is rendered smaller and scaled up.
pub const MAX_PIXELS: usize = 640 * 480;

/// Radians of orbit for each point the pointer is dragged.
const ORBIT_RAD_PER_PT: f32 = 0.008;
/// Radians of orbit for an arrow key press.
const ORBIT_KEY_RAD: f32 = 0.1;
/// The camera never tilts closer to a pole than this, so the look-at up vector stays defined.
const MAX_PITCH_RAD: f32 = 1.5;
/// A pointer that moves farther than this many points between press and release dragged; it did not click.
const CLICK_SLOP_PT: f32 = 4.0;
/// The distance is multiplied by this for each unit of scroll.
const ZOOM_PER_UNIT: f32 = 0.9;
/// Scroll units for one `+` or `-` key press.
const ZOOM_KEY_UNITS: f32 = 1.0;

/// A scene node together with what is drawn at it.
#[derive(Clone, Debug)]
pub struct SceneDrawable {
    pub node: NodeId,
    pub geometry: Geometry3,
    pub material: Material,
    /// Draw the back of the faces too (a flat plane, a sheet). Off by default.
    pub double_sided: bool,
    radius: f32,
}

impl SceneDrawable {
    /// `geometry` drawn at `node` with a default blue material. The pick radius is the farthest vertex from the origin.
    pub fn new(node: NodeId, geometry: Geometry3) -> Self {
        let radius = geometry.positions.iter().map(|p| p.length()).fold(0.0f32, f32::max);
        SceneDrawable {
            node,
            geometry,
            material: Material {
                ambient: Rgb::new(0.12, 0.14, 0.2),
                diffuse: Rgb::new(0.3, 0.55, 0.95),
                specular: Rgb::new(0.35, 0.35, 0.35),
                shininess: 24.0,
            },
            double_sided: false,
            radius,
        }
    }

    pub fn with_material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    pub fn double_sided(mut self, on: bool) -> Self {
        self.double_sided = on;
        self
    }

    /// The radius of the bounding sphere picking tests, in the drawable's own units.
    pub fn pick_radius(&self) -> f32 {
        self.radius
    }
}

/// The camera control: the camera sits `distance` from `target`, at `yaw` around the vertical axis (0 on the +Z side)
/// and `pitch` above the horizontal plane.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Orbit {
    target: Vector3,
    distance: f32,
    yaw: f32,
    pitch: f32,
}

impl Orbit {
    /// The orbit that puts the camera at `eye` looking at `target`.
    fn from_eye(eye: Vector3, target: Vector3) -> Orbit {
        let d = eye - target;
        let distance = d.length().max(1e-3);
        Orbit { target, distance, yaw: d.x.atan2(d.z), pitch: (d.y / distance).clamp(-1.0, 1.0).asin().clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD) }
    }

    fn eye(&self) -> Vector3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vector3::new(cp * sy, sp, cp * cy) * self.distance
    }
}

/// A 3D scene shown in a window.
pub struct SceneWidget {
    base: ViewBase,
    scene: Scene,
    camera: NodeId,
    drawables: Vec<SceneDrawable>,
    light: Light,
    background: Option<Rgb>,
    orbit: Orbit,
    control: bool,
    /// Counts every change that needs a new picture.
    revision: u64,
    /// What the picture on screen was made for: the revision and its size in device pixels.
    rendered: Option<(u64, usize, usize)>,
    image: Option<Image>,
    image_id: u64,
    pictures: u64,
    view: SceneView,
    /// Where the pointer went down and whether it has dragged since.
    drag: Option<(f32, f32, bool)>,
    last: (f32, f32),
    picked: Option<NodeId>,
}

impl SceneWidget {
    /// A widget over `scene`, seen through `camera`. `None` when `camera` is not a node with a camera attached. The
    /// orbit starts from the camera's pose, around the world origin.
    pub fn new(scene: Scene, camera: NodeId) -> Option<Self> {
        scene.camera(camera)?;
        let eye = scene.world_position(camera)?;
        let mut view = SceneView::new(1.0, 1.0);
        view.set_point_of_view(Some(camera));
        Some(SceneWidget {
            base: ViewBase::default(),
            scene,
            camera,
            drawables: Vec::new(),
            light: Light {
                source: LightSource::Directional(Vector3::new(-0.4, -0.7, -0.6)),
                ambient: Rgb::new(0.35, 0.35, 0.35),
                diffuse: Rgb::new(0.85, 0.85, 0.85),
                specular: Rgb::new(0.3, 0.3, 0.3),
            },
            background: None,
            orbit: Orbit::from_eye(eye, Vector3::ZERO),
            control: true,
            revision: 1,
            rendered: None,
            image: None,
            image_id: 0,
            pictures: 0,
            view,
            drag: None,
            last: (0.0, 0.0),
            picked: None,
        })
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The scene, for the app to move nodes. The view redraws. Moving the camera node by hand does not move the orbit:
    /// call [`SceneWidget::reset_orbit`] afterwards to take the orbit from the camera again.
    pub fn scene_mut(&mut self) -> &mut Scene {
        self.changed();
        &mut self.scene
    }

    pub fn camera(&self) -> NodeId {
        self.camera
    }

    pub fn drawables(&self) -> &[SceneDrawable] {
        &self.drawables
    }

    pub fn add_drawable(&mut self, drawable: SceneDrawable) {
        self.drawables.push(drawable);
        self.changed();
    }

    pub fn clear_drawables(&mut self) {
        self.drawables.clear();
        self.changed();
    }

    pub fn light(&self) -> &Light {
        &self.light
    }

    pub fn set_light(&mut self, light: Light) {
        self.light = light;
        self.changed();
    }

    /// An opaque background colour, or `None` (the default) to leave the view's own background showing.
    pub fn set_background(&mut self, background: Option<Rgb>) {
        self.background = background;
        self.changed();
    }

    /// Whether dragging, scrolling and the keys move the camera. On by default.
    pub fn set_camera_control(&mut self, on: bool) {
        self.control = on;
    }

    pub fn camera_control(&self) -> bool {
        self.control
    }

    /// The object the last click picked; `None` when it hit nothing or nothing was clicked yet.
    pub fn picked(&self) -> Option<NodeId> {
        self.picked
    }

    // ---- camera ----

    /// The point the camera looks at and turns around.
    pub fn orbit_target(&self) -> Vector3 {
        self.orbit.target
    }

    /// Looks at `target` from where the camera is now.
    pub fn set_orbit_target(&mut self, target: Vector3) {
        let eye = self.orbit.eye();
        self.orbit = Orbit::from_eye(eye, target);
        self.apply_orbit();
    }

    pub fn orbit_distance(&self) -> f32 {
        self.orbit.distance
    }

    /// `(yaw, pitch)` in radians: yaw 0 is the +Z side of the target, positive pitch looks down from above.
    pub fn orbit_angles(&self) -> (f32, f32) {
        (self.orbit.yaw, self.orbit.pitch)
    }

    /// Where the camera is in the world.
    pub fn camera_position(&self) -> Vector3 {
        self.scene.world_position(self.camera).unwrap_or(Vector3::ZERO)
    }

    /// Takes the orbit from the camera node's present pose, keeping the target.
    pub fn reset_orbit(&mut self) {
        if let Some(eye) = self.scene.world_position(self.camera) {
            self.orbit = Orbit::from_eye(eye, self.orbit.target);
            self.changed();
        }
    }

    /// Turns the camera around the target by `dyaw` and `dpitch` radians. Does nothing while camera control is off.
    pub fn rotate(&mut self, dyaw: f32, dpitch: f32) -> bool {
        if !self.control {
            return false;
        }
        self.orbit.yaw = (self.orbit.yaw + dyaw) % std::f32::consts::TAU;
        self.orbit.pitch = (self.orbit.pitch + dpitch).clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);
        self.apply_orbit();
        true
    }

    /// Zooms: each unit of positive `delta` moves the camera 10% closer, a negative one farther. The distance stays
    /// between twice the near plane and half the far plane of the camera.
    pub fn scroll(&mut self, delta: f32) -> bool {
        if !self.control || delta == 0.0 {
            return false;
        }
        let (near, far) = self.scene.camera(self.camera).map_or((0.1, 1000.0), |c| (c.z_near, c.z_far));
        let (lo, hi) = (near * 2.0, (far * 0.5).max(near * 4.0));
        let next = (self.orbit.distance * ZOOM_PER_UNIT.powf(delta)).clamp(lo, hi);
        if next == self.orbit.distance {
            return true;
        }
        self.orbit.distance = next;
        self.apply_orbit();
        true
    }

    /// Writes the orbit into the camera node's pose.
    fn apply_orbit(&mut self) {
        let eye = self.orbit.eye();
        if let Some(orientation) = look_at_orientation(eye, self.orbit.target, Vector3::new(0.0, 1.0, 0.0)) {
            self.scene.set_pose(self.camera, Pose { position: eye, orientation, scale: Vector3::new(1.0, 1.0, 1.0) });
        }
        self.changed();
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.base.dirty = true;
    }

    // ---- picking ----

    /// The objects under the point `(x, y)` in points from the view's top left, nearest first.
    pub fn hits(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let f = self.base.frame;
        self.view.set_size(f.w, f.h);
        let candidates: Vec<(NodeId, f32)> = self
            .drawables
            .iter()
            .map(|d| {
                let scale = self.scene.world_pose(d.node).map_or(1.0, |p| p.scale.x.abs().max(p.scale.y.abs()).max(p.scale.z.abs()));
                (d.node, d.radius * scale)
            })
            .collect();
        self.view.hit_test(&self.scene, x, y, &candidates).into_iter().map(|h| h.node).collect()
    }

    /// The nearest object under `(x, y)` in points from the view's top left.
    pub fn pick(&mut self, x: f32, y: f32) -> Option<NodeId> {
        self.hits(x, y).first().copied()
    }

    // ---- rendering ----

    /// Renders the scene from the camera into a new colour and depth buffer of `width` by `height` pixels.
    pub fn render(&self, width: usize, height: usize) -> Target {
        let mut target = Target::new(width, height, self.background.unwrap_or(Rgb::BLACK));
        if width == 0 || height == 0 {
            return target;
        }
        let Some(camera) = self.scene.camera(self.camera) else { return target };
        let Some(view) = self.scene.camera_view(self.camera) else { return target };
        let fitted = Camera { aspect: width as f32 / height as f32, ..camera };
        let view_projection = fitted.projection() * view;
        let eye = self.scene.world_position(self.camera).unwrap_or(Vector3::ZERO);
        for d in &self.drawables {
            let Some(pose) = self.scene.world_pose(d.node) else { continue };
            let triangles = world_triangles(d, pose.matrix(), pose.orientation, eye);
            raster3d::draw(&mut target, &triangles, view_projection, eye, &self.light, &d.material);
        }
        target
    }

    /// [`SceneWidget::render`] as pixels the canvas draws. Where nothing was drawn a pixel is transparent, unless the
    /// widget has an opaque background.
    pub fn render_image(&self, width: usize, height: usize) -> Image {
        let target = self.render(width, height);
        let opaque_background = self.background.is_some();
        let mut px = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let covered = target.depth_at(x, y).is_some_and(f32::is_finite);
                px.push(match target.color_at(x, y) {
                    Some(c) if covered || opaque_background => pack(encode(c.r), encode(c.g), encode(c.b), 255),
                    _ => 0,
                });
            }
        }
        Image { w: width, h: height, px }
    }

    /// The size of the picture for the frame, in device pixels: the frame's size, shrunk to [`MAX_PIXELS`].
    fn picture_size(&self, ui: &Ui) -> (usize, usize) {
        let f = self.base.frame;
        let (w, h) = (ui.pt(f.w).round().max(0.0), ui.pt(f.h).round().max(0.0));
        let area = w * h;
        if area <= 0.0 {
            return (0, 0);
        }
        let shrink = if area > MAX_PIXELS as f32 { (MAX_PIXELS as f32 / area).sqrt() } else { 1.0 };
        (((w * shrink) as usize).max(1), ((h * shrink) as usize).max(1))
    }

    /// Whether the picture is missing or out of date for a picture of `width` by `height`.
    pub fn needs_render(&self, width: usize, height: usize) -> bool {
        self.rendered != Some((self.revision, width, height))
    }

    fn zoom_key(&mut self, units: f32) -> bool {
        self.scroll(units);
        true
    }
}

/// One linear colour component as an 8-bit display value (gamma 2.2).
fn encode(c: f32) -> u8 {
    (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8
}

/// The triangles of `d` in world space. A triangle is wound to agree with its vertex normals, then dropped when it
/// faces away from `eye` (a double-sided drawable is turned toward the eye instead). Degenerate triangles are dropped.
fn world_triangles(d: &SceneDrawable, matrix: Matrix4, rotation: Quaternion, eye: Vector3) -> Vec<Triangle> {
    let world: Vec<Vector3> = d.geometry.positions.iter().map(|p| matrix.transform_point(*p).unwrap_or(*p)).collect();
    let normals: Vec<Vector3> = d.geometry.normals.iter().map(|n| rotation.rotate(*n)).collect();
    let mut out = Vec::with_capacity(d.geometry.indices.len() / 3);
    for t in d.geometry.indices.chunks_exact(3) {
        let (ia, ib, ic) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (Some(&a), Some(&b), Some(&c)) = (world.get(ia), world.get(ib), world.get(ic)) else { continue };
        let (mut b, mut c) = (b, c);
        let mut g = (b - a).cross(c - a);
        if g.length() <= f32::EPSILON {
            continue;
        }
        if let (Some(&na), Some(&nb), Some(&nc)) = (normals.get(ia), normals.get(ib), normals.get(ic)) {
            if g.dot(na + nb + nc) < 0.0 {
                std::mem::swap(&mut b, &mut c);
                g = -g;
            }
        }
        let centre = (a + b + c) * (1.0 / 3.0);
        if g.dot(eye - centre) < 0.0 {
            if !d.double_sided {
                continue;
            }
            std::mem::swap(&mut b, &mut c);
        }
        out.push(Triangle { a, b, c });
    }
    out
}

impl View for SceneWidget {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let area = RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h));
        s.fill_rect(area, DETAIL_BG.now());
        let (w, h) = self.picture_size(ui);
        if w > 0 && h > 0 {
            if self.needs_render(w, h) {
                self.image = Some(self.render_image(w, h));
                self.pictures += 1;
                self.image_id = self.id().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.pictures;
                self.rendered = Some((self.revision, w, h));
            }
            if let Some(img) = &self.image {
                s.image(ImageRef { id: self.image_id, image: img }, area, 1.0);
            }
        }
        if self.base.focused {
            focus_ring(s, ui, f, 0.0);
        }
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) if self.base.frame.contains(*x, *y) => {
                self.drag = Some((*x, *y, false));
                self.last = (*x, *y);
                true
            }
            ViewEvent::Pointer(Event::Move(x, y)) => {
                let Some((sx, sy, dragged)) = self.drag else { return false };
                let moved = dragged || (x - sx).hypot(y - sy) > CLICK_SLOP_PT;
                if moved {
                    let (dx, dy) = (x - self.last.0, y - self.last.1);
                    self.rotate(-dx * ORBIT_RAD_PER_PT, dy * ORBIT_RAD_PER_PT);
                }
                self.drag = Some((sx, sy, moved));
                self.last = (*x, *y);
                true
            }
            ViewEvent::Pointer(Event::Up(x, y)) => {
                let Some((_, _, dragged)) = self.drag.take() else { return false };
                if !dragged {
                    let f = self.base.frame;
                    self.picked = self.pick(x - f.x, y - f.y);
                    self.base.send_action(ACTION_PICK);
                }
                true
            }
            ViewEvent::Cancel => self.drag.take().is_some(),
            ViewEvent::Key(KeyInput::Text(t)) if t == "+" || t == "=" => self.zoom_key(ZOOM_KEY_UNITS),
            ViewEvent::Key(KeyInput::Text(t)) if t == "-" => self.zoom_key(-ZOOM_KEY_UNITS),
            ViewEvent::Key(KeyInput::Code(37)) => {
                self.rotate(ORBIT_KEY_RAD, 0.0);
                true
            }
            ViewEvent::Key(KeyInput::Code(39)) => {
                self.rotate(-ORBIT_KEY_RAD, 0.0);
                true
            }
            ViewEvent::Key(KeyInput::Code(38)) => {
                self.rotate(0.0, ORBIT_KEY_RAD);
                true
            }
            ViewEvent::Key(KeyInput::Code(40)) => {
                self.rotate(0.0, -ORBIT_KEY_RAD);
                true
            }
            _ => false,
        }
    }

    fn ax_node(&self) -> Option<Node> {
        node(self, Role::StaticText, "3D scene", &format!("{} objects", self.drawables.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Font, Surface};
    use bifrost_ui::primitives3d::{plane_geometry, sphere_geometry};
    use bifrost_ui::responder::Responder;
    use bifrost_ui::Rect;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn red() -> Material {
        Material { ambient: Rgb::new(1.0, 0.0, 0.0), diffuse: Rgb::BLACK, specular: Rgb::BLACK, shininess: 1.0 }
    }

    fn flat_light() -> Light {
        Light { source: LightSource::Directional(Vector3::new(0.0, 0.0, -1.0)), ambient: Rgb::WHITE, diffuse: Rgb::BLACK, specular: Rgb::BLACK }
    }

    /// A camera 5 units out on +Z looking at a red ball of radius 1 at the origin, in a 320 by 240 frame.
    fn demo() -> (SceneWidget, NodeId) {
        let mut scene = Scene::new();
        let root = scene.root();
        let cam = scene.add_child(root, "camera").unwrap();
        scene.set_camera(cam, Some(Camera::new(60.0, 4.0 / 3.0, 0.1, 100.0)));
        let mut pose = Pose::identity();
        pose.position = Vector3::new(0.0, 0.0, 5.0);
        scene.set_pose(cam, pose);
        let ball = scene.add_child(root, "ball").unwrap();
        let mut w = SceneWidget::new(scene, cam).unwrap();
        w.add_drawable(SceneDrawable::new(ball, sphere_geometry(1.0, 24, 12).unwrap()).with_material(red()));
        w.set_light(flat_light());
        w.set_frame(Rect { x: 0.0, y: 0.0, w: 320.0, h: 240.0 });
        (w, ball)
    }

    fn covered(t: &Target) -> usize {
        (0..t.height()).flat_map(|y| (0..t.width()).map(move |x| (x, y))).filter(|&(x, y)| t.depth_at(x, y).is_some_and(f32::is_finite)).count()
    }

    fn pointer(w: &mut SceneWidget, down: (f32, f32), up: (f32, f32)) {
        w.handle(&ViewEvent::Pointer(Event::Down(down.0, down.1)));
        w.handle(&ViewEvent::Pointer(Event::Move(up.0, up.1)));
        w.handle(&ViewEvent::Pointer(Event::Up(up.0, up.1)));
    }

    #[test]
    fn a_node_without_a_camera_is_not_a_point_of_view() {
        let scene = Scene::new();
        let root = scene.root();
        assert!(SceneWidget::new(scene, root).is_none());
    }

    #[test]
    fn the_ball_is_drawn_in_its_colour_at_the_centre_and_the_corners_stay_empty() {
        let (w, _) = demo();
        let t = w.render(64, 48);
        assert_eq!(t.color_at(32, 24), Some(Rgb::new(1.0, 0.0, 0.0)));
        assert!(t.depth_at(32, 24).unwrap().is_finite());
        assert!(t.depth_at(0, 0).unwrap().is_infinite());
        assert!(t.depth_at(63, 47).unwrap().is_infinite());
        let img = w.render_image(64, 48);
        assert_eq!(img.px[24 * 64 + 32], pack(255, 0, 0, 255));
        assert_eq!(img.px[0], 0, "an empty pixel is transparent so the view's background shows");
    }

    #[test]
    fn an_opaque_background_fills_the_empty_pixels() {
        let (mut w, _) = demo();
        w.set_background(Some(Rgb::new(0.0, 0.0, 1.0)));
        let img = w.render_image(64, 48);
        assert_eq!(img.px[0], pack(0, 0, 255, 255));
    }

    #[test]
    fn faces_that_look_away_are_not_drawn_unless_the_drawable_is_double_sided() {
        let (mut w, _) = demo();
        let plane = w.scene.add_child(w.scene.root(), "plane").unwrap();
        // A plane facing +Y seen from the side at the camera's height is edge on, so tilt the camera above it.
        w.rotate(0.0, 0.6);
        w.clear_drawables();
        let mut d = SceneDrawable::new(plane, plane_geometry(2.0, 2.0).unwrap()).with_material(red());
        w.add_drawable(d.clone());
        let from_above = covered(&w.render(64, 48));
        assert!(from_above > 100, "{from_above}");
        w.rotate(0.0, -1.2);
        assert_eq!(covered(&w.render(64, 48)), 0, "the underside of a single-sided plane is culled");
        d.double_sided = true;
        w.clear_drawables();
        w.add_drawable(d);
        assert!(covered(&w.render(64, 48)) > 100, "a double-sided plane shows its underside");
    }

    #[test]
    fn dragging_orbits_the_camera_around_the_target_at_the_same_distance() {
        let (mut w, _) = demo();
        assert!((w.orbit_distance() - 5.0).abs() < 1e-3);
        pointer(&mut w, (100.0, 100.0), (140.0, 100.0));
        let p = w.camera_position();
        assert!(p.x < -0.5, "dragging right brings the left side into view: {p:?}");
        assert!(((p - w.orbit_target()).length() - 5.0).abs() < 1e-3);
        assert_eq!(w.picked(), None, "a drag is not a click");
        pointer(&mut w, (100.0, 100.0), (100.0, 160.0));
        assert!(w.camera_position().y > 0.5, "dragging down tilts the camera up");
    }

    #[test]
    fn the_camera_never_tilts_past_the_poles() {
        let (mut w, _) = demo();
        w.rotate(0.0, 10.0);
        assert!(w.orbit_angles().1 <= MAX_PITCH_RAD);
        assert!(w.camera_position().y > 4.9 && w.camera_position().y < 5.0);
        w.rotate(0.0, -20.0);
        assert!(w.orbit_angles().1 >= -MAX_PITCH_RAD);
        assert!(w.render(32, 24).width() == 32, "still renders");
    }

    #[test]
    fn scrolling_zooms_within_the_cameras_clip_range_and_the_ball_grows() {
        let (mut w, _) = demo();
        let before = covered(&w.render(64, 48));
        assert!(w.scroll(3.0));
        assert!(w.orbit_distance() < 5.0 * 0.9 * 0.9);
        assert!(covered(&w.render(64, 48)) > before);
        w.scroll(1000.0);
        assert!((w.orbit_distance() - 0.2).abs() < 1e-4, "stops at twice the near plane");
        w.scroll(-1000.0);
        assert!((w.orbit_distance() - 50.0).abs() < 1e-3, "stops at half the far plane");
    }

    #[test]
    fn keys_zoom_and_orbit_and_camera_control_can_be_switched_off() {
        let (mut w, _) = demo();
        w.handle(&ViewEvent::Key(KeyInput::Text("+".into())));
        assert!(w.orbit_distance() < 5.0);
        w.handle(&ViewEvent::Key(KeyInput::Code(37)));
        assert!(w.orbit_angles().0 > 0.0);
        w.set_camera_control(false);
        let before = w.orbit_angles();
        let distance = w.orbit_distance();
        w.handle(&ViewEvent::Key(KeyInput::Code(39)));
        w.handle(&ViewEvent::Key(KeyInput::Text("-".into())));
        pointer(&mut w, (100.0, 100.0), (180.0, 140.0));
        assert_eq!((w.orbit_angles(), w.orbit_distance()), (before, distance));
    }

    #[test]
    fn a_click_picks_the_object_under_it_and_a_click_on_empty_space_picks_nothing() {
        let (mut w, ball) = demo();
        pointer(&mut w, (160.0, 120.0), (160.0, 120.0));
        assert_eq!(w.picked(), Some(ball));
        pointer(&mut w, (6.0, 6.0), (6.0, 6.0));
        assert_eq!(w.picked(), None);
        assert_eq!(w.hits(160.0, 120.0), vec![ball]);
    }

    #[test]
    fn the_nearer_of_two_objects_is_picked_first() {
        let (mut w, ball) = demo();
        let near = w.scene.add_child(w.scene.root(), "near").unwrap();
        let mut pose = Pose::identity();
        pose.position = Vector3::new(0.0, 0.0, 2.0);
        w.scene.set_pose(near, pose);
        w.add_drawable(SceneDrawable::new(near, sphere_geometry(0.5, 12, 6).unwrap()));
        assert_eq!(w.hits(160.0, 120.0), vec![near, ball]);
    }

    #[test]
    fn a_pick_sends_one_action_through_the_window_and_the_view_draws_the_scene() {
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
        let (w, ball) = demo();
        let id = w.id();
        let mut win: super::super::Window<Model> = super::super::Window::new(w, 1);
        let ui = ui();
        win.layout(&ui, Rect { x: 0.0, y: 0.0, w: 320.0, h: 240.0 });
        let mut m = Model::default();
        win.event(&ViewEvent::Pointer(Event::Down(160.0, 120.0)), &mut m);
        win.event(&ViewEvent::Pointer(Event::Up(160.0, 120.0)), &mut m);
        assert_eq!(m.log, [ACTION_PICK]);
        assert_eq!(win.view::<SceneWidget>(id).unwrap().picked(), Some(ball));
        let mut px = vec![0u32; 320 * 240];
        let mut s = Surface { w: 320, h: 240, stride: 320, px: &mut px, y0: 0 };
        win.draw(&mut s, &ui);
        let centre = s.px[120 * 320 + 160];
        assert!(centre & 0xFF > 200 && (centre >> 8) & 0xFF < 60 && (centre >> 16) & 0xFF < 60, "the ball is at the middle of the window: {centre:#x}");
        assert_ne!(s.px[2 * 320 + 2], centre);
    }

    #[test]
    fn the_picture_is_rendered_again_only_when_something_changed() {
        let (mut w, _) = demo();
        assert!(w.needs_render(64, 48));
        w.rendered = Some((w.revision, 64, 48));
        assert!(!w.needs_render(64, 48));
        assert!(w.needs_render(65, 48), "a new size needs a new picture");
        w.rotate(0.1, 0.0);
        assert!(w.needs_render(64, 48), "a camera move needs a new picture");
        w.rendered = Some((w.revision, 64, 48));
        w.scene_mut();
        assert!(w.needs_render(64, 48), "handing out the scene counts as a change");
    }

    #[test]
    fn a_large_view_is_rendered_at_the_pixel_cap() {
        let (mut w, _) = demo();
        let ui = ui();
        w.set_frame(Rect { x: 0.0, y: 0.0, w: 1600.0, h: 1200.0 });
        let (pw, ph) = w.picture_size(&ui);
        assert!(pw * ph <= MAX_PIXELS && pw * ph > MAX_PIXELS * 9 / 10, "{pw}x{ph}");
        assert!((pw as f32 / ph as f32 - 4.0 / 3.0).abs() < 0.02);
        w.set_frame(Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 });
        assert_eq!(w.picture_size(&ui), (200, 100));
    }
}
