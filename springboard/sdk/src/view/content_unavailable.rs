//! Content-unavailable view (the reference platform's content-unavailable view): an empty-state page drawn from a
//! `bifrost_ui::content_unavailable::ContentUnavailable` configuration. The parts stack in the middle of the frame: the
//! picture the app supplies (drawn at its pixel size, centred), the title, the secondary text, then one button per
//! configured button, each sending its own action. The labels and buttons are the accessibility elements.
use super::controls::{Align, Button, Label, Stack};
use super::{blit, View, ViewBase};
use crate::kit::gfx::Image;
use crate::kit::ui::Weight;
use crate::kit::Ui;
use bifrost_render::canvas::Canvas;
use bifrost_render::ui::SPACE_M;
use bifrost_ui::content_unavailable::{Axis, ContentUnavailable};
use bifrost_ui::stack::Axis as StackAxis;
use bifrost_ui::Rect;

/// The picture of the page, drawn at its own pixel size.
struct ImageSlot {
    base: ViewBase,
    image: Image,
}

impl View for ImageSlot {
    crate::view_base!();

    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let scale = ui.pt(1.0);
        (self.image.w as f32 / scale, self.image.h as f32 / scale)
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let x = ui.pt(f.x) + (ui.pt(f.w) - self.image.w as f32) / 2.0;
        let y = ui.pt(f.y) + (ui.pt(f.h) - self.image.h as f32) / 2.0;
        blit(s, &self.image, x as i32, y as i32);
    }
}

/// An empty-state page (the reference platform's content-unavailable view). Build it from a configuration; read the configuration back with
/// [`ContentUnavailableView::config`].
pub struct ContentUnavailableView {
    base: ViewBase,
    config: ContentUnavailable,
    kids: Vec<Box<dyn View>>,
}

impl ContentUnavailableView {
    /// A page for `config` without a picture; the app draws `config.image` itself if it wants one.
    pub fn new(config: ContentUnavailable) -> Self {
        Self::build(config, None)
    }

    /// A page for `config` with `image` drawn above the title.
    pub fn with_image(config: ContentUnavailable, image: Image) -> Self {
        Self::build(config, Some(image))
    }

    fn build(config: ContentUnavailable, image: Option<Image>) -> Self {
        let mut rows: Vec<Box<dyn View>> = Vec::new();
        if let Some(image) = image {
            rows.push(Box::new(ImageSlot { base: ViewBase::default(), image }));
        }
        if !config.text.is_empty() {
            let mut title = Label::new(&config.text);
            title.weight = Weight::Semibold;
            rows.push(Box::new(title));
        }
        if !config.secondary_text.is_empty() {
            let mut text = Label::new(&config.secondary_text);
            text.secondary = true;
            rows.push(Box::new(text));
        }
        for b in &config.buttons {
            rows.push(Box::new(Button::new(&b.title, &b.action)));
        }
        let axis = match config.axis {
            Axis::Vertical => StackAxis::Vertical,
            Axis::Horizontal => StackAxis::Horizontal,
        };
        let stack = Stack::new(axis, SPACE_M, rows).aligned(Align::Center);
        ContentUnavailableView { base: ViewBase::default(), config, kids: vec![Box::new(stack)] }
    }

    pub fn config(&self) -> &ContentUnavailable {
        &self.config
    }
}

impl View for ContentUnavailableView {
    crate::view_base!();

    fn children(&self) -> &[Box<dyn View>] {
        &self.kids
    }

    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.kids
    }

    /// The stack takes its own height and sits in the middle of the page; a page without a height takes all of it.
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let (_, content_h) = self.kids[0].intrinsic(ui);
        let h = if content_h > 0.0 { content_h.min(bounds.h) } else { bounds.h };
        let y = bounds.y + (bounds.h - h) / 2.0;
        self.kids[0].layout(ui, Rect { x: bounds.x, y, w: bounds.w, h });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_stack_in_order_without_a_picture_or_empty_text() {
        let view = ContentUnavailableView::new(ContentUnavailable::search());
        assert_eq!(view.kids[0].children().len(), 2);
        let view = ContentUnavailableView::new(ContentUnavailable::default().with_button("Retry", "retry"));
        assert_eq!(view.kids[0].children().len(), 1);
        assert_eq!(view.config().buttons.len(), 1);
    }
}
