//! A drawing context for a view's own rectangle: `with_context` hands `View::draw` a `DrawContext` whose user space
//! is the view's frame in points (origin at the frame's top-left), clipped to the frame. The drawing goes to an
//! offscreen bitmap of the frame's device pixels and is composited onto the canvas, so it works on every canvas.
use super::{image_key, px_rect};
use crate::kit::{Image, Ui};
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_render::cg::context::{Bitmap, DrawContext};
use bifrost_ui::Rect;

/// Runs `f` with a context for `frame` (window points) and draws the result onto `c`. Inside `f` user space is in
/// points with (0, 0) at the frame's top-left corner, so a view draws the same at any scale; nothing outside the frame
/// is touched. Returns `f`'s value, or `None` when the frame has no pixels.
pub fn with_context<R>(c: &mut dyn Canvas, ui: &Ui, frame: Rect, f: impl FnOnce(&mut DrawContext<'_>) -> R) -> Option<R> {
    let (x, y, w, h) = px_rect(ui, frame);
    if w == 0 || h == 0 {
        return None;
    }
    let mut bm = Bitmap::new(w, h);
    let out = {
        let mut cx = bm.context();
        // the frame's exact device origin can sit between pixels; `px_rect` snaps outwards, so shift by the remainder
        cx.translate_ctm(ui.pt(frame.x) - x as f32, ui.pt(frame.y) - y as f32);
        cx.scale_ctm(ui.s, ui.s);
        cx.clip_to_rect(bifrost_render::cg::context::Rect::new(0.0, 0.0, frame.w, frame.h));
        f(&mut cx)
    };
    let img: Image = bm.make_image();
    c.image(ImageRef { id: image_key(&img), image: &img }, RectF::new(x as f32, y as f32, w as f32, h as f32), 1.0);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Font, Surface};
    use bifrost_render::cg::color::Color;
    use bifrost_render::cg::context::Rect as CgRect;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 2.0)
    }

    #[test]
    fn the_context_draws_in_points_inside_the_frame_only() {
        let ui = ui();
        let mut px = vec![0u32; 40 * 40];
        let mut s = Surface { w: 40, h: 40, stride: 40, px: &mut px, y0: 0 };
        let frame = Rect { x: 5.0, y: 5.0, w: 10.0, h: 10.0 }; // device 10..30 at scale 2
        let r = with_context(&mut s, &ui, frame, |cx| {
            cx.set_fill_color(Color::rgba(1.0, 0.0, 0.0, 1.0));
            // larger than the frame: the clip stops it at the frame
            cx.fill_rect(CgRect::new(-100.0, -100.0, 300.0, 300.0));
            7
        });
        assert_eq!(r, Some(7));
        assert_eq!(px[20 * 40 + 20], 0xff00_00ff);
        assert_eq!(px[5 * 40 + 5], 0, "outside the frame");
        assert_eq!(px[31 * 40 + 20], 0);
    }

    #[test]
    fn an_empty_frame_draws_nothing() {
        let ui = ui();
        let mut px = vec![0u32; 16];
        let mut s = Surface { w: 4, h: 4, stride: 4, px: &mut px, y0: 0 };
        assert!(with_context(&mut s, &ui, Rect { x: 0.0, y: 0.0, w: 0.0, h: 5.0 }, |_| ()).is_none());
    }
}
