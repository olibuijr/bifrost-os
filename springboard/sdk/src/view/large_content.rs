//! Large content viewer (`UILargeContentViewerInteraction`) for a bar item or a control. `LargeContentView` wraps the
//! item's view: a press held still for `HOLD_S` shows the item's large title, and its picture when it has one, in a
//! bubble above the item; lifting the finger hides the bubble. The wrapped view keeps every pointer event, so a button
//! still sends its action; a long press on a button therefore also fires that action on release. The wrapper sees the
//! press through `View::observe_pointer`, which the item does not stop.
use super::{blit, View, ViewBase, ViewEvent};
use crate::kit::gfx::Image;
use crate::kit::ui::{Weight, CARD_BG, LABEL};
use crate::kit::{Event, Ui};
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_ui::large_content::{Change, LargeContentInteraction, LargeContentItem};
use bifrost_ui::stack::Axis;
use bifrost_ui::Rect;

/// Text size of the bubble title, in points.
const TITLE_PT: f32 = 22.0;
/// Inner padding of the bubble, in points.
const PAD_PT: f32 = 12.0;
/// Gap between the item and the bubble, and between the picture and the title, in points.
const GAP_PT: f32 = 8.0;
/// Corner radius of the bubble, in points.
const BUBBLE_RADIUS_PT: f32 = 4.0;
/// Room around the item that the bubble may cover; the window repaints this area while the item is dirty.
const BUBBLE_MARGIN_PT: f32 = 160.0;

/// A view that presents its `item` in enlarged form when its press is held still.
pub struct LargeContentView {
    base: ViewBase,
    kids: Vec<Box<dyn View>>,
    item: LargeContentItem,
    interaction: LargeContentInteraction,
    image: Option<Image>,
    now: f64,
}

impl LargeContentView {
    /// Wraps `child`, the item's own view. `enabled` is `isEnabled`: the app turns the viewer on for large text sizes.
    pub fn new(item: LargeContentItem, child: Box<dyn View>, enabled: bool) -> Self {
        LargeContentView {
            base: ViewBase::default(),
            kids: vec![child],
            item,
            interaction: LargeContentInteraction::new(enabled),
            image: None,
            now: 0.0,
        }
    }

    /// The picture the bubble shows above the title, rendered at the size the app wants.
    pub fn with_image(mut self, image: Image) -> Self {
        self.image = Some(image);
        self
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.interaction.enabled = on;
    }

    pub fn is_showing(&self) -> bool {
        self.interaction.is_showing()
    }

    pub fn item(&self) -> &LargeContentItem {
        &self.item
    }
}

impl View for LargeContentView {
    crate::view_base!();

    fn children(&self) -> &[Box<dyn View>] {
        &self.kids
    }

    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.kids
    }

    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        self.kids[0].intrinsic(ui)
    }

    fn hugging(&self, axis: Axis) -> u16 {
        self.kids[0].hugging(axis)
    }

    fn compression(&self, axis: Axis) -> u16 {
        self.kids[0].compression(axis)
    }

    fn observes_pointer(&self) -> bool {
        self.item.shows
    }

    fn observe_pointer(&mut self, ev: &ViewEvent) {
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => self.interaction.press(self.now, *x, *y),
            ViewEvent::Pointer(Event::Move(x, y)) => self.interaction.moved(*x, *y),
            ViewEvent::Pointer(Event::Up(..)) | ViewEvent::Cancel => {
                self.interaction.end();
            }
            _ => {}
        }
        self.base.dirty = true;
    }

    fn tick(&mut self, t: f64) {
        self.now = t;
        if self.interaction.tick(t) == Some(Change::Present) {
            self.base.dirty = true;
        }
    }

    fn wants_tick(&self) -> bool {
        self.interaction.wants_tick()
    }

    /// The bubble lies above the frame, so the dirty area is the frame with a margin around it.
    fn dirty_rect(&self) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x - BUBBLE_MARGIN_PT, y: f.y - BUBBLE_MARGIN_PT, w: f.w + 2.0 * BUBBLE_MARGIN_PT, h: f.h + 2.0 * BUBBLE_MARGIN_PT }
    }

    fn draw_overlay(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        if !self.interaction.is_showing() {
            return;
        }
        let f = self.base.frame;
        let title = ui.text(&self.item.title, TITLE_PT, Weight::Semibold, LABEL.now());
        let pad = ui.pt(PAD_PT);
        let gap = ui.pt(GAP_PT);
        let (img_w, img_h) = self.image.as_ref().map_or((0.0, 0.0), |i| (i.w as f32, i.h as f32));
        let content_w = (title.w as f32).max(img_w);
        let w = (content_w + 2.0 * pad).max(ui.pt(f.w));
        let h = img_h + if img_h > 0.0 { gap } else { 0.0 } + title.h as f32 + 2.0 * pad;
        let x = (ui.pt(f.x) + ui.pt(f.w) / 2.0 - w / 2.0).max(0.0);
        let above = ui.pt(f.y) - gap - h;
        let y = if above >= 0.0 { above } else { ui.pt(f.y + f.h) + gap };
        s.fill_rrect(RectF::new(x, y, w, h), ui.pt(BUBBLE_RADIUS_PT), CARD_BG.now());
        let mut row_y = y + pad;
        if let Some(img) = &self.image {
            blit(s, img, (x + (w - img_w) / 2.0) as i32, row_y as i32);
            row_y += img_h + gap;
        }
        blit(s, &title, (x + (w - title.w as f32) / 2.0) as i32, row_y as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::controls::Label;
    use bifrost_ui::large_content::HOLD_S;

    #[test]
    fn a_held_press_presents_the_item_and_lifting_the_finger_dismisses_it() {
        let child = Box::new(Label::new("Share"));
        let mut v = LargeContentView::new(LargeContentItem::new("Share"), child, true);
        assert!(v.observes_pointer());
        v.observe_pointer(&ViewEvent::Pointer(Event::Down(1.0, 1.0)));
        assert!(v.wants_tick());
        v.tick(0.0);
        v.tick(HOLD_S);
        assert!(v.is_showing());
        v.observe_pointer(&ViewEvent::Pointer(Event::Up(1.0, 1.0)));
        assert!(!v.is_showing());
    }

    #[test]
    fn an_item_that_does_not_show_in_the_viewer_is_not_observed() {
        let mut item = LargeContentItem::new("Hidden");
        item.shows = false;
        let v = LargeContentView::new(item, Box::new(Label::new("Hidden")), true);
        assert!(!v.observes_pointer());
    }
}
