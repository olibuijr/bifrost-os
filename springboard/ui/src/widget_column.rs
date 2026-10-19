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

/// One row of a flowed column: the index of the row in the list given to [`flow`], its top edge and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub index: usize,
    pub y: f32,
    pub height: f32,
}

/// Stacks rows of the given `heights` downwards from `top`, `spacing` apart, and keeps the leading rows that end at or
/// above `bottom`; the first row that would cross it ends the column (a later, shorter row does not jump the queue).
/// The Notification Center column uses it for widget rows whose height follows their family. A negative height or
/// spacing counts as zero.
pub fn flow(heights: &[f32], top: f32, spacing: f32, bottom: f32) -> Vec<Slot> {
    let mut slots = Vec::new();
    let mut y = top;
    for (index, h) in heights.iter().enumerate() {
        let height = h.max(0.0);
        if y + height > bottom {
            break;
        }
        slots.push(Slot { index, y, height });
        y += height + spacing.max(0.0);
    }
    slots
}

/// The shown block under the vertical point `y`: its top edge is included and its bottom edge is not. A point in a
/// gap between blocks, above the column or below it, or a NaN point, gives None.
pub fn block_at(frames: &[Frame], y: f32) -> Option<&Frame> {
    frames.iter().find(|f| y >= f.y && y < f.y + f.height)
}

/// The height the column takes: from the top of the first shown block to the bottom of the last one, gaps included.
pub fn content_height(frames: &[Frame]) -> f32 {
    frames.last().map_or(0.0, |f| f.y + f.height)
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

    #[test]
    fn flowed_rows_stack_from_the_top_with_their_own_heights() {
        let slots = flow(&[58.0, 96.0, 77.0], 100.0, 10.0, 1000.0);
        assert_eq!(
            slots,
            vec![Slot { index: 0, y: 100.0, height: 58.0 }, Slot { index: 1, y: 168.0, height: 96.0 }, Slot { index: 2, y: 274.0, height: 77.0 }]
        );
    }

    #[test]
    fn the_first_row_that_crosses_the_bottom_ends_the_column() {
        // 58 fits (ends 158); the 96 row would end at 264 > 200; the later 20 row would fit but is not shown
        let slots = flow(&[58.0, 96.0, 20.0], 100.0, 10.0, 200.0);
        assert_eq!(slots.iter().map(|s| s.index).collect::<Vec<_>>(), vec![0]);
        assert!(flow(&[58.0], 100.0, 10.0, 157.0).is_empty(), "a row ending past the bottom is left out");
        assert_eq!(flow(&[58.0], 100.0, 10.0, 158.0).len(), 1, "ending exactly at the bottom fits");
    }

    #[test]
    fn flowed_rows_treat_negative_heights_and_spacing_as_zero() {
        let slots = flow(&[-4.0, 10.0], 0.0, -3.0, 50.0);
        assert_eq!(slots, vec![Slot { index: 0, y: 0.0, height: 0.0 }, Slot { index: 1, y: 0.0, height: 10.0 }]);
        assert!(flow(&[], 0.0, 8.0, 50.0).is_empty());
    }
}
