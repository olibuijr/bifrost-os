//! Embedded map view (the SwiftUI `Map` content that the MapKit overlay module adds): a [`View`] that shows the map
//! tiles around a camera inside an app window, with annotation pins, drag to pan and the keyboard to zoom.
//!
//! The camera, the gestures and the annotation model are `bifrost_app::maps::MapView` and [`MapItem`]; the tiles are
//! the Maps app's own (`springboard::maptiles`), so the view draws the same map. Drawing never waits for the network:
//! [`MapWidget::refresh`] renders the tiles for the current camera (the app calls it from its tick, with the
//! snapshotter it owns), and `draw` paints the last picture. While no picture exists, or after the camera moved and
//! before the next refresh, the old picture stays on screen and the view reports [`MapWidget::needs_refresh`].
//!
//! ```ignore
//! let mut map = MapWidget::new(camera);
//! map.set_annotations(vec![MapItem::new(place)]);
//! // in the app tick:
//! if map.needs_refresh() { map.refresh(&mut snapshotter, Duration::from_millis(200)); }
//! ```
use super::{View, ViewBase, ViewEvent};
use crate::kit::gfx::Image;
use crate::kit::ui::{DETAIL_BG, DESTRUCTIVE};
use crate::kit::{Event, Ui};
use crate::maps::{Coordinate, MapCamera, MapItem, MapView, Picture, Snapshotter};
use crate::proto::KeyInput;
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_ui::{Node, Role};
use std::time::Duration;

/// Packs a rendered map picture (RGB bytes) into the opaque pixels the canvas draws.
pub fn picture_to_image(picture: &Picture) -> Image {
    let px = picture
        .rgb
        .chunks_exact(3)
        .map(|c| 0xFF00_0000 | (c[2] as u32) << 16 | (c[1] as u32) << 8 | c[0] as u32)
        .collect();
    Image { w: picture.width, h: picture.height, px }
}

/// What a refresh rendered: the camera it was made for, so a later change of centre, zoom or size is noticed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rendered {
    lat: f64,
    lon: f64,
    zoom: f64,
    w: f64,
    h: f64,
    scale: f64,
}

/// A map shown in a window.
pub struct MapWidget {
    base: ViewBase,
    map: MapView,
    annotations: Vec<MapItem>,
    image: Option<Image>,
    image_id: u64,
    rendered: Option<Rendered>,
    drag_from: Option<(f32, f32)>,
}

impl MapWidget {
    /// A map showing `camera`, with no annotations.
    pub fn new(camera: MapCamera) -> Self {
        MapWidget {
            base: ViewBase::default(),
            map: MapView::new(camera),
            annotations: Vec::new(),
            image: None,
            image_id: 0,
            rendered: None,
            drag_from: None,
        }
    }

    /// The camera and gesture model.
    pub fn map(&self) -> &MapView {
        &self.map
    }

    /// The model for changing the region, the centre or the gestures that are on.
    pub fn map_mut(&mut self) -> &mut MapView {
        self.base.dirty = true;
        &mut self.map
    }

    /// Replaces the annotations (pins) the view draws.
    pub fn set_annotations(&mut self, items: Vec<MapItem>) {
        self.annotations = items;
        self.base.dirty = true;
    }

    pub fn annotations(&self) -> &[MapItem] {
        &self.annotations
    }

    /// Where an annotation's pin stands in view points (from the view's top left), or `None` when it is outside the
    /// view.
    pub fn pin_position(&self, item: &MapItem) -> Option<(f32, f32)> {
        let c = item.placemark.coordinate;
        let (x, y) = self.map.camera().screen_point(c);
        let cam = self.map.camera();
        (x >= 0.0 && y >= 0.0 && x <= cam.viewport_w && y <= cam.viewport_h).then_some((x as f32, y as f32))
    }

    fn wanted(&self, scale: f64) -> Rendered {
        let cam = self.map.camera();
        Rendered {
            lat: cam.center.latitude,
            lon: cam.center.longitude,
            zoom: cam.zoom,
            w: cam.viewport_w,
            h: cam.viewport_h,
            scale,
        }
    }

    /// Whether the picture on screen is out of date: none yet, or the camera or the size changed since.
    pub fn needs_refresh(&self) -> bool {
        self.rendered != Some(self.wanted(self.rendered.map_or(1.0, |r| r.scale)))
    }

    /// Renders the tiles for the current camera at `scale` pixels a point. Waits for the tiles for at most
    /// `timeout`; a tile that has not arrived stays white and is filled in by a later refresh from the shared cache.
    pub fn refresh(&mut self, snapshotter: &mut Snapshotter, scale: f64, timeout: Duration) {
        let want = self.wanted(scale);
        if want.w < 1.0 || want.h < 1.0 {
            return;
        }
        let centre = self.map.camera().center;
        let picture = snapshotter.render(centre, want.zoom, want.w, want.h, scale, timeout);
        self.image = Some(picture_to_image(&picture));
        self.image_id += 1;
        self.rendered = Some(want);
        self.base.dirty = true;
    }

    /// Draws a picture that the caller rendered itself (same effect as [`MapWidget::refresh`]).
    pub fn set_picture(&mut self, picture: &Picture, scale: f64) {
        let want = self.wanted(scale);
        self.image = Some(picture_to_image(picture));
        self.image_id += 1;
        self.rendered = Some(want);
        self.base.dirty = true;
    }

    fn zoom(&mut self, delta: f64) {
        if self.map.pinch(delta) {
            self.base.dirty = true;
        }
    }
}

impl View for MapWidget {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn layout(&mut self, ui: &Ui, bounds: bifrost_ui::Rect) {
        let _ = ui;
        self.set_frame(bounds);
        if self.map.set_viewport(bounds.w as f64, bounds.h as f64) {
            self.base.dirty = true;
        }
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let area = RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h));
        match &self.image {
            Some(img) => s.image(ImageRef { id: self.image_id, image: img }, area, 1.0),
            None => s.fill_rect(area, DETAIL_BG.now()),
        }
        let pins: Vec<(f32, f32)> = self.annotations.iter().filter_map(|a| self.pin_position(a)).collect();
        for (x, y) in pins {
            let r = ui.pt(5.0);
            s.fill_rrect(RectF::new(area.x + ui.pt(x) - r, area.y + ui.pt(y) - r, 2.0 * r, 2.0 * r), r, DESTRUCTIVE);
        }
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) if self.base.frame.contains(*x, *y) => {
                self.drag_from = Some((*x, *y));
                true
            }
            ViewEvent::Pointer(Event::Move(x, y)) => match self.drag_from {
                Some((px, py)) => {
                    // Dragging the map right moves the camera left.
                    if self.map.drag(-(*x - px) as f64, -(*y - py) as f64) {
                        self.base.dirty = true;
                    }
                    self.drag_from = Some((*x, *y));
                    true
                }
                None => false,
            },
            ViewEvent::Pointer(Event::Up(..)) | ViewEvent::Cancel => {
                self.drag_from.take().is_some()
            }
            ViewEvent::Key(KeyInput::Text(t)) if t == "+" || t == "=" => {
                self.zoom(1.0);
                true
            }
            ViewEvent::Key(KeyInput::Text(t)) if t == "-" => {
                self.zoom(-1.0);
                true
            }
            _ => false,
        }
    }

    fn ax_node(&self) -> Option<Node> {
        super::controls::node(self, Role::StaticText, "Map", "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> MapWidget {
        let c = Coordinate::new(64.1466, -21.9426).unwrap();
        MapWidget::new(MapCamera::new(c, 12.0, 400.0, 300.0).unwrap())
    }

    #[test]
    fn a_picture_becomes_opaque_pixels_in_the_canvas_order() {
        let p = Picture { width: 2, height: 1, rgb: vec![1, 2, 3, 255, 255, 255] };
        let img = picture_to_image(&p);
        assert_eq!((img.w, img.h), (2, 1));
        assert_eq!(img.px[0], 0xFF03_0201);
        assert_eq!(img.px[1], 0xFFFF_FFFF);
    }

    #[test]
    fn a_new_view_needs_a_refresh_and_a_picture_satisfies_it_until_the_camera_moves() {
        let mut w = widget();
        assert!(w.needs_refresh());
        w.set_picture(&Picture { width: 4, height: 3, rgb: vec![255; 36] }, 1.0);
        assert!(!w.needs_refresh());
        w.map_mut().pinch(1.0);
        assert!(w.needs_refresh());
    }

    #[test]
    fn dragging_pans_the_camera_the_opposite_way() {
        let mut w = widget();
        w.set_frame(bifrost_ui::Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 });
        let before = w.map().camera().center;
        assert!(w.handle(&ViewEvent::Pointer(Event::Down(100.0, 100.0))));
        assert!(w.handle(&ViewEvent::Pointer(Event::Move(160.0, 100.0))));
        assert!(w.handle(&ViewEvent::Pointer(Event::Up(160.0, 100.0))));
        let after = w.map().camera().center;
        assert!(after.longitude < before.longitude, "dragging right shows more of the west");
        assert!(!w.handle(&ViewEvent::Pointer(Event::Move(10.0, 10.0))), "no drag is in progress");
    }

    #[test]
    fn keys_zoom_and_gestures_can_be_switched_off() {
        let mut w = widget();
        let z = w.map().camera().zoom;
        assert!(w.handle(&ViewEvent::Key(KeyInput::Text("+".into()))));
        assert!(w.map().camera().zoom > z);
        w.map_mut().interaction.zoom = false;
        let z = w.map().camera().zoom;
        w.handle(&ViewEvent::Key(KeyInput::Text("-".into())));
        assert_eq!(w.map().camera().zoom, z);
    }

    #[test]
    fn a_pin_inside_the_view_has_a_position_and_one_outside_has_none() {
        let mut w = widget();
        let here = MapItem::new(bifrost_ui::reverse_lookup::Place { name: "here".into(), region: String::new(), coordinate: Coordinate::new(64.1466, -21.9426).unwrap() });
        let (x, y) = w.pin_position(&here).expect("the centre is inside the view");
        assert!((x - 200.0).abs() < 2.0 && (y - 150.0).abs() < 2.0);
        let far = MapItem::new(bifrost_ui::reverse_lookup::Place { name: "far".into(), region: String::new(), coordinate: Coordinate::new(-30.0, 100.0).unwrap() });
        assert!(w.pin_position(&far).is_none());
        w.set_annotations(vec![here]);
        assert_eq!(w.annotations().len(), 1);
    }
}
