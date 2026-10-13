//! The column of the Notification Center: the notification list and the Today widgets, stacked from the top. Pure
//! layout model. It takes the blocks in their order, leaves out the widgets whose app reported no content (the
//! `Controller`), gives each widget the height its display mode allows (`Limits`), and returns the frame of every
//! block that is shown. Nothing here draws; the shell uses the frames when it places the cards.

use std::collections::HashMap;

use crate::widget::{Controller, DisplayMode, Limits};

/// One block of the column: a widget by its bundle identifier, or the notification list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Widget(String),
    Notifications,
}

/// Where a shown block sits: its top edge and its height, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub block: Block,
    pub y: f32,
    pub height: f32,
}

/// Lays out the column. `modes` holds the display mode each widget is in (a widget not listed is compact).
/// `notifications_height` is the height the notification list asks for. A block whose height is zero is left out,
/// and `spacing` separates neighbouring shown blocks.
pub fn layout(
    blocks: &[Block],
    controller: &Controller,
    modes: &HashMap<String, DisplayMode>,
    limits: &Limits,
    notifications_height: f32,
    spacing: f32,
) -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut y = 0.0_f32;
    for block in blocks {
        let height = match block {
            Block::Widget(id) => {
                if !controller.has_content(id) {
                    continue;
                }
                let mode = modes.get(id).copied().unwrap_or_default();
                limits.max_size(mode).1
            }
            Block::Notifications => notifications_height.max(0.0),
        };
        if height <= 0.0 {
            continue;
        }
        if !frames.is_empty() {
            y += spacing.max(0.0);
        }
        frames.push(Frame { block: block.clone(), y, height });
        y += height;
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(id: &str) -> Block {
        Block::Widget(id.to_string())
    }

    fn limits() -> Limits {
        Limits::new(342.0, 110.0, 340.0)
    }

    #[test]
    fn blocks_stack_from_the_top_with_spacing_between_them_only() {
        let blocks = vec![Block::Notifications, widget("is.example.a")];
        let frames = layout(&blocks, &Controller::default(), &HashMap::new(), &limits(), 200.0, 8.0);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Frame { block: Block::Notifications, y: 0.0, height: 200.0 });
        assert_eq!(frames[1], Frame { block: widget("is.example.a"), y: 208.0, height: 110.0 });
    }

    #[test]
    fn a_widget_with_no_content_is_left_out_and_takes_no_room() {
        let mut controller = Controller::default();
        controller.set_has_content("is.example.a", false);
        let blocks = vec![widget("is.example.a"), widget("is.example.b")];
        let frames = layout(&blocks, &controller, &HashMap::new(), &limits(), 0.0, 8.0);
        assert_eq!(frames, vec![Frame { block: widget("is.example.b"), y: 0.0, height: 110.0 }]);
    }

    #[test]
    fn an_expanded_widget_takes_its_expanded_height() {
        let mut modes = HashMap::new();
        modes.insert("is.example.a".to_string(), DisplayMode::Expanded);
        let frames = layout(&[widget("is.example.a")], &Controller::default(), &modes, &limits(), 0.0, 0.0);
        assert_eq!(frames[0].height, 340.0);
    }

    #[test]
    fn a_compact_widget_is_the_default_mode() {
        let frames = layout(&[widget("is.example.a")], &Controller::default(), &HashMap::new(), &limits(), 0.0, 0.0);
        assert_eq!(frames[0].height, 110.0);
    }

    #[test]
    fn an_empty_notification_list_is_left_out_without_a_gap() {
        let blocks = vec![Block::Notifications, widget("is.example.a")];
        let frames = layout(&blocks, &Controller::default(), &HashMap::new(), &limits(), 0.0, 8.0);
        assert_eq!(frames, vec![Frame { block: widget("is.example.a"), y: 0.0, height: 110.0 }]);
    }

    #[test]
    fn negative_heights_and_spacing_are_treated_as_zero() {
        let blocks = vec![Block::Notifications, widget("is.example.a")];
        let frames = layout(&blocks, &Controller::default(), &HashMap::new(), &limits(), -5.0, -8.0);
        assert_eq!(frames, vec![Frame { block: widget("is.example.a"), y: 0.0, height: 110.0 }]);
    }
}
