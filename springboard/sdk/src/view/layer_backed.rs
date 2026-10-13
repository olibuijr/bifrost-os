//! Layer-backed views: a view whose `ViewBase::layer` is set draws that layer tree after its own content.
use super::{px_rf, View};
use crate::kit::Ui;
use bifrost_render::canvas::{Canvas, ImageRef, RectF, Transform2D};
use bifrost_render::gfx::Image;
use bifrost_ui::layer_tree::{Layer, LayerCmd};
use bifrost_ui::Rect;

/// Keeps the layer's frame equal to the view's frame (points, window space).
fn sync(layer: &Layer, f: Rect) {
    if layer.frame() != f {
        let b = layer.bounds();
        layer.set_bounds(Rect { x: b.x, y: b.y, w: f.w, h: f.h });
        let (ax, ay) = layer.anchor_point();
        layer.set_position(f.x + ax * f.w, f.y + ay * f.h);
    }
}

pub(super) fn draw(v: &mut dyn View, c: &mut dyn Canvas, ui: &Ui) {
    let Some(layer) = v.base().layer.clone() else { return };
    let f = v.frame();
    sync(&layer, f);
    let cmds = layer.render_list();
    layer.clear_dirty_tree();
    if cmds.is_empty() {
        return;
    }
    c.push_clip(px_rf(ui, f), 0.0);
    let s = ui.pt(1.0);
    c.push_transform(Transform2D::scale(s, s));
    replay(&cmds, c);
    c.pop_transform();
    c.pop_clip();
}

fn rf(r: Rect) -> RectF {
    RectF::new(r.x, r.y, r.w, r.h)
}

/// Plays render commands onto a canvas whose current space is the layer tree's root space.
pub(super) fn replay(cmds: &[LayerCmd], c: &mut dyn Canvas) {
    let mut faded: Vec<bool> = Vec::new();
    for cmd in cmds {
        match cmd {
            LayerCmd::Push { transform: t, opacity } => {
                c.push_transform(Transform2D { a: t.a, b: t.b, c: t.c, d: t.d, tx: t.tx, ty: t.ty });
                let fade = *opacity < 1.0;
                if fade {
                    c.push_opacity(*opacity);
                }
                faded.push(fade);
            }
            LayerCmd::Pop => {
                if faded.pop().unwrap_or(false) {
                    c.pop_opacity();
                }
                c.pop_transform();
            }
            LayerCmd::PushClip(r) => c.push_clip(rf(*r), 0.0),
            LayerCmd::PopClip => c.pop_clip(),
            LayerCmd::Fill { rect, color } => c.fill_rect(rf(*rect), *color),
            LayerCmd::Stroke { rect, width, color } => c.stroke_rrect(rf(*rect), 0.0, *width, *color),
            LayerCmd::Image { image, src, dst, alpha } => {
                let full = Image { w: image.w, h: image.h, px: image.px.as_ref().clone() };
                let (x, y) = ((src.x * image.w as f32).round().max(0.0) as usize, (src.y * image.h as f32).round().max(0.0) as usize);
                let (w, h) = ((src.w * image.w as f32).round() as usize, (src.h * image.h as f32).round() as usize);
                if w == 0 || h == 0 || x >= image.w || y >= image.h {
                    continue;
                }
                if (x, y, w, h) == (0, 0, image.w, image.h) {
                    c.image(ImageRef { id: image.id, image: &full }, rf(*dst), *alpha);
                } else {
                    let part = full.crop(x, y, w, h);
                    let id = image.id ^ ((x as u64) << 48 ^ (y as u64) << 32 ^ (w as u64) << 16 ^ h as u64);
                    c.image(ImageRef { id, image: &part }, rf(*dst), *alpha);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::layer_tree::LayerImage;

    #[test]
    fn sync_follows_the_view_frame() {
        let l = Layer::new();
        let f = Rect { x: 5.0, y: 6.0, w: 30.0, h: 20.0 };
        sync(&l, f);
        assert_eq!(l.frame(), f);
        l.set_anchor_point(0.0, 0.0);
        sync(&l, f);
        assert_eq!(l.frame(), f);
    }

    #[test]
    fn a_layer_tree_renders_to_commands_that_replay() {
        let l = Layer::with_frame(Rect { x: 0.0, y: 0.0, w: 4.0, h: 4.0 });
        l.set_background_color(Some(0xff00_00ff));
        l.set_contents(Some(LayerImage::new(1, 1, vec![0xffff_ffff])));
        let cmds = l.render_list();
        assert!(cmds.len() >= 4);
    }
}
