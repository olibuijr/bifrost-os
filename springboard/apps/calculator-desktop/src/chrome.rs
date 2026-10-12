//! The calculator's drawing helpers on the `Canvas` vocabulary (the `draw_canvas` path, docs/graphics-architecture.md).
//!
//! One routine runs on both targets: `Surface` implements `Canvas` for the shared-memory path, and the direct path
//! hands the app a GPU canvas. Everything here takes `&mut dyn Canvas`. The SDK's `widgets` helpers (the toolbar and
//! its band) and `glyphs` take a `Surface`, so this module draws the same pixels with `Canvas` calls: the toolbar
//! mirrors `widgets::toolbar` and `widgets::toolbar_band`, and a symbol is rasterised by `glyphs::glyph` onto a
//! scratch surface and placed as an image.

use bifrost_app::controls::nav::{Toolbar, ToolbarKind, ToolbarLayout};
use bifrost_app::controls::{ax, Rect, Role};
use bifrost_app::glyphs;
use bifrost_app::kit::canvas::{Canvas, ImageRef, RectF};
use bifrost_app::kit::ui::{
    Ui, Weight, CAPSULE_BG, CAPSULE_EDGE, CAPSULE_SHADOW, DETAIL_BG, HAIRLINE_C, LABEL, LABEL_3, MAC_BODY_PT,
    ROW_PRESSED, SEGMENT_ON, SEPARATOR_C, SPACE_S,
};
use bifrost_app::kit::{appearance, Image, Surface};
use bifrost_app::widgets;

/// A rectangle cut to whole pixels the way `Surface` cuts its `usize` arguments: floored, and never negative.
pub fn whole(x: f32, y: f32, w: f32, h: f32) -> RectF {
    RectF::new(x.max(0.0).floor(), y.max(0.0).floor(), w.max(0.0).floor(), h.max(0.0).floor())
}

/// A content hash of a rendered image (FNV-1a over its size and pixels). The GPU side caches images by this id, and
/// it changes whenever the pixels change, which the contract allows (`ImageRef::id`).
pub fn image_id(img: &Image) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |word: u64| {
        h ^= word;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(img.w as u64);
    feed(img.h as u64);
    for &p in &img.px {
        feed(p as u64);
    }
    h
}

/// Draws `img` with its top-left at `(x, y)` in device pixels, at full opacity: what `Surface::blit` drew with a
/// tint and alpha of 255.
pub fn blit(c: &mut dyn Canvas, img: &Image, x: i32, y: i32) {
    c.image(
        ImageRef { id: image_id(img), image: img },
        RectF::new(x as f32, y as f32, img.w as f32, img.h as f32),
        1.0,
    );
}

/// `widgets::capsule`: the glass capsule (ring, plate and edge) in points.
pub fn capsule(c: &mut dyn Canvas, ui: &Ui, b: Rect) {
    let Rect { x, y, w, h } = b;
    let r = h / 2.0;
    let o = ui.pt(1.0).max(1.0);
    c.fill_rrect(
        RectF::new(ui.pt(x) - o, ui.pt(y) - o + o * 0.5, ui.pt(w) + 2.0 * o, ui.pt(h) + 2.0 * o),
        ui.pt(r) + o,
        CAPSULE_SHADOW.now(),
    );
    c.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(r), CAPSULE_BG.now());
    c.stroke_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(r), o, CAPSULE_EDGE.now());
}

/// `widgets::outline_if_borders`: a hairline outline around a button when the appearance asks for button shapes.
fn outline_if_borders(c: &mut dyn Canvas, ui: &Ui, b: Rect, radius: f32) {
    if !appearance::button_shapes() {
        return;
    }
    c.stroke_rrect(
        RectF::new(ui.pt(b.x), ui.pt(b.y), ui.pt(b.w), ui.pt(b.h)),
        ui.pt(radius),
        ui.pt(1.0).max(1.0),
        HAIRLINE_C.now(),
    );
}

/// `widgets::toolbar_band` in its opaque form: the content colour with a hairline at the bottom. The translucent
/// titlebar material is not drawn here yet, because the SDK registers it with the shell from `Surface` only (see
/// docs/graphics/g2-app-canvas-pilot.md, "What is left").
pub fn toolbar_band(c: &mut dyn Canvas, ui: &Ui, b: Rect) {
    let Rect { x, y, w, h } = b;
    c.fill_rect(
        whole(ui.pt(x), ui.pt(y), ui.pt(w).ceil(), ui.pt(h).ceil()),
        DETAIL_BG.now(),
    );
    let sep_y = (ui.pt(y + h) as usize).saturating_sub(1) as f32;
    c.fill_rect(
        RectF::new(ui.pt(x) as usize as f32, sep_y, ui.pt(w).ceil() as usize as f32, 1.0),
        SEPARATOR_C.now(),
    );
}

/// A symbol centred in `b` (points), rasterised by the SDK's `glyphs::glyph` onto a scratch surface the size of `b`
/// and placed as an image. False when the name is unknown (nothing drawn).
pub fn glyph(c: &mut dyn Canvas, ui: &Ui, name: &str, b: Rect, color: u32) -> bool {
    let (w, h) = (ui.pt(b.w).ceil().max(1.0) as usize, ui.pt(b.h).ceil().max(1.0) as usize);
    let mut px = vec![0u32; w * h];
    let drew = {
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        glyphs::glyph(&mut s, ui, name, Rect { x: 0.0, y: 0.0, w: b.w, h: b.h }, color)
    };
    if !drew {
        return false;
    }
    let img = Image { w, h, px };
    let (x, y) = (ui.pt(b.x) as i32, ui.pt(b.y) as i32);
    blit(c, &img, x, y);
    true
}

/// A text image centred on the point `(cx, cy)` (points) in the canvas.
fn centred(c: &mut dyn Canvas, ui: &Ui, img: &Image, cx: f32, cy: f32) {
    let x = ui.pt(cx) as i32 - img.w as i32 / 2;
    let y = ui.pt(cy) as i32 - img.h as i32 / 2;
    blit(c, img, x, y);
}

/// `widgets::toolbar`: lays the items out in `b` and draws them with `Canvas` calls. The same layout, glyphs and
/// accessibility records as the SDK helper; `pressed` is the id being pressed.
pub fn toolbar(c: &mut dyn Canvas, ui: &Ui, b: Rect, bar: &Toolbar, pressed: Option<u32>) -> ToolbarLayout {
    let placed = widgets::toolbar_layout(ui, b, bar);
    for (_, r) in &placed.groups {
        capsule(c, ui, *r);
        outline_if_borders(c, ui, *r, r.h / 2.0);
    }
    for (k, (id, r)) in placed.visible.iter().enumerate() {
        let Some(item) = bar.items.iter().find(|i| i.id == *id && i.kind == ToolbarKind::Button) else { continue };
        ax::record(Role::Button, &item.label, if item.selected { "selected" } else { "" }, *r, item.enabled);
        if item.group == 0 {
            capsule(c, ui, *r);
        } else if k > 0 && bar.items.iter().find(|i| i.id == placed.visible[k - 1].0).is_some_and(|p| p.group == item.group) {
            // a hairline between neighbours of one group
            let inset = r.h * 0.25;
            c.fill_rect(
                whole(ui.pt(r.x), ui.pt(r.y + inset), ui.pt(1.0).max(1.0), ui.pt(r.h - 2.0 * inset)),
                SEPARATOR_C.now(),
            );
        }
        let plate = Rect { x: r.x + 3.0, y: r.y + 3.0, w: r.w - 6.0, h: r.h - 6.0 };
        let fill = if pressed == Some(*id) && item.enabled {
            Some(ROW_PRESSED.now())
        } else if item.selected {
            Some(SEGMENT_ON.now())
        } else {
            None
        };
        if let Some(fill) = fill {
            c.fill_rrect(RectF::new(ui.pt(plate.x), ui.pt(plate.y), ui.pt(plate.w), ui.pt(plate.h)), ui.pt(plate.h / 2.0), fill);
        }
        let color = if item.enabled { LABEL.now() } else { LABEL_3.now() };
        let r = *r;
        let r = if item.menu {
            // a pull-down item keeps its chevron in the trailing TOOLBAR_MENU_CHEVRON_PT
            let chevron = 7.0;
            let cx = r.right() - widgets::TOOLBAR_MENU_CHEVRON_PT - 2.0 + (widgets::TOOLBAR_MENU_CHEVRON_PT - chevron) / 2.0;
            glyph(
                c,
                ui,
                "chevron.down",
                Rect { x: cx, y: r.y + (r.h - chevron) / 2.0, w: chevron, h: chevron },
                color,
            );
            Rect { w: r.w - widgets::TOOLBAR_MENU_CHEVRON_PT + 4.0, ..r }
        } else {
            r
        };
        let drew = item.symbol.as_deref().is_some_and(|name| {
            let g = 20.0;
            glyph(c, ui, name, Rect { x: r.x + (r.w - g) / 2.0, y: r.y + (r.h - g) / 2.0, w: g, h: g }, color)
        });
        if !drew {
            let img = ui.text(&ui.fit(&item.label, MAC_BODY_PT, ui.pt(r.w - 2.0 * SPACE_S)), MAC_BODY_PT, Weight::Regular, color);
            centred(c, ui, &img, r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
    }
    if let Some(r) = placed.overflow_button {
        capsule(c, ui, r);
        let img = ui.text("\u{bb}", MAC_BODY_PT, Weight::Regular, LABEL.now());
        centred(c, ui, &img, r.x + r.w / 2.0, r.y + r.h / 2.0);
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::canvas::{MaterialStyle, PathData, PathPaint, Stop, TextRun, Transform2D};

    fn img(w: usize, h: usize, fill: u32) -> Image {
        Image { w, h, px: vec![fill; w * h] }
    }

    #[test]
    fn whole_floors_and_never_goes_negative() {
        assert_eq!(whole(3.7, 2.2, 10.9, 4.0), RectF::new(3.0, 2.0, 10.0, 4.0));
        assert_eq!(whole(-5.0, 1.5, -2.0, 3.9), RectF::new(0.0, 1.0, 0.0, 3.0));
    }

    #[test]
    fn identical_pixels_give_the_same_id() {
        assert_eq!(image_id(&img(3, 2, 0xff00_00ff)), image_id(&img(3, 2, 0xff00_00ff)));
    }

    #[test]
    fn one_changed_pixel_changes_the_id() {
        let a = img(3, 2, 7);
        let mut b = a.clone();
        b.px[4] = 8;
        assert_ne!(image_id(&a), image_id(&b));
    }

    #[test]
    fn the_same_pixels_in_another_shape_change_the_id() {
        // 2x3 and 3x2 hold the same six pixels but are different images.
        assert_ne!(image_id(&img(2, 3, 9)), image_id(&img(3, 2, 9)));
    }

    #[test]
    fn an_empty_image_has_a_stable_id() {
        let e = Image { w: 0, h: 0, px: Vec::new() };
        assert_eq!(image_id(&e), image_id(&e.clone()));
        assert_ne!(image_id(&e), image_id(&img(1, 1, 0)));
    }

    /// Records the image calls, so a test can check where `blit` put an image.
    #[derive(Default)]
    struct Recorder {
        images: Vec<(u64, RectF, f32)>,
    }

    impl Canvas for Recorder {
        fn size(&self) -> (u32, u32) {
            (100, 100)
        }
        fn fill_rect(&mut self, _r: RectF, _color: u32) {}
        fn fill_rrect(&mut self, _r: RectF, _radius: f32, _color: u32) {}
        fn stroke_rrect(&mut self, _r: RectF, _radius: f32, _width: f32, _color: u32) {}
        fn linear_gradient(&mut self, _r: RectF, _radius: f32, _s: (f32, f32), _e: (f32, f32), _stops: &[Stop]) {}
        fn shadow(&mut self, _r: RectF, _radius: f32, _blur: f32, _offset: (f32, f32), _color: u32) {}
        fn image(&mut self, img: ImageRef<'_>, dst: RectF, alpha: f32) {
            self.images.push((img.id, dst, alpha));
        }
        fn image_tinted(&mut self, _img: ImageRef<'_>, _dst: RectF, _color: u32) {}
        fn text(&mut self, _run: &TextRun, _color: u32) {}
        fn path(&mut self, _path: &PathData, _paint: PathPaint, _color: u32) {}
        fn material(&mut self, _r: RectF, _radius: f32, _style: MaterialStyle) {}
        fn push_clip(&mut self, _r: RectF, _radius: f32) {}
        fn pop_clip(&mut self) {}
        fn push_transform(&mut self, _t: Transform2D) {}
        fn pop_transform(&mut self) {}
        fn push_opacity(&mut self, _alpha: f32) {}
        fn pop_opacity(&mut self) {}
    }

    #[test]
    fn blit_places_the_image_at_full_opacity_with_its_content_id() {
        let picture = img(4, 3, 0x1234_5678);
        let mut rec = Recorder::default();
        blit(&mut rec, &picture, 10, 20);
        assert_eq!(rec.images.len(), 1);
        let (id, dst, alpha) = rec.images[0];
        assert_eq!(id, image_id(&picture));
        assert_eq!(dst, RectF::new(10.0, 20.0, 4.0, 3.0));
        assert_eq!(alpha, 1.0);
    }
}
