//! Offscreen rendering: draw a window's view tree into an image instead of the screen.
//!
//! [`render_window`] lays the tree out in the pixel size asked for (divided by the interface scale to get points),
//! paints every view and returns the pixels. Use it for thumbnails, snapshots and export.

use crate::kit::{Image, Surface, Ui};
use crate::view::Window;
use bifrost_ui::responder::Responder;
use bifrost_ui::Rect;

/// Renders `window` into a new `width` x `height` image. The window is laid out at that size and fully repainted.
pub fn render_window<S: Responder>(window: &mut Window<S>, ui: &Ui, width: usize, height: usize) -> Image {
    let mut img = Image::new(width, height);
    let scale = ui.pt(1.0).max(f32::EPSILON);
    window.layout(ui, Rect { x: 0.0, y: 0.0, w: width as f32 / scale, h: height as f32 / scale });
    {
        let mut s = Surface { w: width, h: height, stride: width, px: &mut img.px, y0: 0 };
        window.draw(&mut s, ui);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Font;
    use crate::view::{Container, NoApp};

    #[test]
    fn a_tree_renders_to_an_image_of_the_requested_size() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(Font::parse(data).unwrap(), 1.0);
        let mut w: Window<NoApp> = Window::new(Container::new(vec![]), 1);
        let img = render_window(&mut w, &ui, 64, 32);
        assert_eq!((img.w, img.h, img.px.len()), (64, 32, 64 * 32));
        assert!(img.px.iter().any(|p| *p != 0), "the window background is painted");
    }
}
