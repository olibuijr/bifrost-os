//! Tool picker (the reference SDK tool picker): the row of tools a drawing canvas offers (inking tools, erasers and the lasso), which
//! one is selected, the colour and width of each inking tool, and whether the picker is shown. This is state only. The
//! shell or an app draws the picker, sends the user's choices here, and takes the ink for a new stroke from
//! `current_ink`. Observers are the caller's concern: read `selected` after each event.
//!
//! Rule: a picker holds each tool at most once. `ToolPicker::new` gives the default set and `with_items` gives a custom
//! set (the reference initialiser that takes tool items). Widths are clamped to the range of the tool's ink kind (`width_range`).
use crate::drawing::{Ink, InkKind};

/// A tool on the picker. Inking tools carry their `InkKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Ink(InkKind),
    Eraser(EraserKind),
    Lasso,
}

/// How an eraser works (the reference eraser type): `Bitmap` removes pixels, `Vector` removes whole strokes (see
/// `Drawing::erase_strokes_at`). The picker only records the choice; the caller does the erasing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraserKind {
    Bitmap,
    Vector,
}

/// One entry on the picker (the reference picker item type). `color` is packed 0xAARRGGBB and `width` is in points. Both matter for
/// inking tools only; erasers and the lasso keep 0 for both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolItem {
    pub tool: Tool,
    pub color: u32,
    pub width: f32,
}

/// The width range of an inking kind, in points, as `(min, max)`.
pub fn width_range(kind: InkKind) -> (f32, f32) {
    match kind {
        InkKind::Pen => (1.0, 20.0),
        InkKind::Marker => (4.0, 40.0),
        InkKind::Pencil => (1.0, 12.0),
    }
}

/// The picker state: its items, the selected tool and whether it is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPicker {
    items: Vec<ToolItem>,
    selected: Tool,
    visible: bool,
}

impl Default for ToolPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolPicker {
    /// The default picker: pen, marker, pencil, both erasers and the lasso. The pen is selected and the picker is hidden.
    pub fn new() -> Self {
        let items = vec![
            ToolItem { tool: Tool::Ink(InkKind::Pen), color: 0xff00_0000, width: 3.0 },
            ToolItem { tool: Tool::Ink(InkKind::Marker), color: 0xccff_d60a, width: 16.0 },
            ToolItem { tool: Tool::Ink(InkKind::Pencil), color: 0xff55_5555, width: 2.0 },
            ToolItem { tool: Tool::Eraser(EraserKind::Bitmap), color: 0, width: 0.0 },
            ToolItem { tool: Tool::Eraser(EraserKind::Vector), color: 0, width: 0.0 },
            ToolItem { tool: Tool::Lasso, color: 0, width: 0.0 },
        ];
        ToolPicker { items, selected: Tool::Ink(InkKind::Pen), visible: false }
    }

    /// A picker with the given items, in order. The first item is selected. Returns `None` when there are no items, when
    /// a tool appears twice, or when an inking item has a non-finite width. Inking widths are clamped to their range.
    pub fn with_items(items: Vec<ToolItem>) -> Option<Self> {
        let first = items.first()?.tool;
        for (i, item) in items.iter().enumerate() {
            if items[..i].iter().any(|other| other.tool == item.tool) {
                return None;
            }
        }
        let mut items = items;
        for item in &mut items {
            if let Tool::Ink(kind) = item.tool {
                if !item.width.is_finite() {
                    return None;
                }
                let (lo, hi) = width_range(kind);
                item.width = item.width.clamp(lo, hi);
            }
        }
        Some(ToolPicker { items, selected: first, visible: false })
    }

    pub fn items(&self) -> &[ToolItem] {
        &self.items
    }

    pub fn selected(&self) -> Tool {
        self.selected
    }

    /// Selects `tool`. Returns false, and keeps the selection, when the picker does not have that tool.
    pub fn select(&mut self, tool: Tool) -> bool {
        if self.index_of(tool).is_some() {
            self.selected = tool;
            true
        } else {
            false
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Sets the colour of an inking tool. Returns false for an eraser, the lasso, or a tool not on the picker.
    pub fn set_color(&mut self, tool: Tool, color: u32) -> bool {
        match (self.index_of(tool), tool) {
            (Some(i), Tool::Ink(_)) => {
                self.items[i].color = color;
                true
            }
            _ => false,
        }
    }

    /// Sets the width of an inking tool, clamped to its range, and returns the width set. Returns `None` for a non-inking
    /// tool, a tool not on the picker, or a non-finite width.
    pub fn set_width(&mut self, tool: Tool, width: f32) -> Option<f32> {
        let kind = match tool {
            Tool::Ink(kind) => kind,
            _ => return None,
        };
        if !width.is_finite() {
            return None;
        }
        let i = self.index_of(tool)?;
        let (lo, hi) = width_range(kind);
        let v = width.clamp(lo, hi);
        self.items[i].width = v;
        Some(v)
    }

    /// The ink a new stroke uses: the selected inking tool's kind, colour and width. `None` when an eraser or the lasso
    /// is selected.
    pub fn current_ink(&self) -> Option<Ink> {
        let kind = match self.selected {
            Tool::Ink(kind) => kind,
            _ => return None,
        };
        let item = self.items[self.index_of(self.selected)?];
        Some(Ink { kind, color: item.color, width: item.width })
    }

    fn index_of(&self, tool: Tool) -> Option<usize> {
        self.items.iter().position(|i| i.tool == tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_picker_starts_hidden_with_the_pen_selected() {
        let p = ToolPicker::new();
        assert!(!p.is_visible());
        assert_eq!(p.selected(), Tool::Ink(InkKind::Pen));
        assert_eq!(p.items().len(), 6);
        assert_eq!(p.current_ink(), Some(Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 3.0 }));
    }

    #[test]
    fn selecting_an_eraser_or_the_lasso_gives_no_ink() {
        let mut p = ToolPicker::new();
        assert!(p.select(Tool::Lasso));
        assert_eq!(p.selected(), Tool::Lasso);
        assert_eq!(p.current_ink(), None);
        assert!(p.select(Tool::Eraser(EraserKind::Vector)));
        assert_eq!(p.current_ink(), None);
    }

    #[test]
    fn select_refuses_a_tool_the_picker_does_not_have() {
        let only_pen = ToolItem { tool: Tool::Ink(InkKind::Pen), color: 0xff00_0000, width: 2.0 };
        let mut p = ToolPicker::with_items(vec![only_pen]).expect("one item is valid");
        assert!(!p.select(Tool::Lasso));
        assert_eq!(p.selected(), Tool::Ink(InkKind::Pen), "the selection is kept");
    }

    #[test]
    fn with_items_rejects_an_empty_set_duplicates_and_bad_widths() {
        assert!(ToolPicker::with_items(Vec::new()).is_none());
        let pen = ToolItem { tool: Tool::Ink(InkKind::Pen), color: 0, width: 2.0 };
        assert!(ToolPicker::with_items(vec![pen, pen]).is_none(), "a tool may appear once");
        let nan = ToolItem { tool: Tool::Ink(InkKind::Marker), color: 0, width: f32::NAN };
        assert!(ToolPicker::with_items(vec![nan]).is_none());
    }

    #[test]
    fn with_items_selects_the_first_item_and_clamps_widths() {
        let wide = ToolItem { tool: Tool::Ink(InkKind::Pen), color: 0, width: 500.0 };
        let thin = ToolItem { tool: Tool::Ink(InkKind::Pencil), color: 0, width: 0.0 };
        let p = ToolPicker::with_items(vec![wide, thin]).expect("valid");
        assert_eq!(p.selected(), Tool::Ink(InkKind::Pen));
        assert_eq!(p.items()[0].width, 20.0);
        assert_eq!(p.items()[1].width, 1.0);
    }

    #[test]
    fn set_width_clamps_to_the_ink_kind_range() {
        let mut p = ToolPicker::new();
        assert_eq!(p.set_width(Tool::Ink(InkKind::Pen), 100.0), Some(20.0));
        assert_eq!(p.set_width(Tool::Ink(InkKind::Pen), 0.0), Some(1.0));
        assert_eq!(p.set_width(Tool::Ink(InkKind::Marker), 10.0), Some(10.0));
        assert_eq!(p.set_width(Tool::Ink(InkKind::Pen), f32::INFINITY), None, "non-finite widths are refused");
        assert_eq!(p.set_width(Tool::Lasso, 5.0), None, "the lasso has no width");
        assert_eq!(p.current_ink().map(|i| i.width), Some(1.0));
    }

    #[test]
    fn set_color_changes_only_inking_tools() {
        let mut p = ToolPicker::new();
        assert!(p.set_color(Tool::Ink(InkKind::Pen), 0xff12_3456));
        assert_eq!(p.current_ink().map(|i| i.color), Some(0xff12_3456));
        assert!(!p.set_color(Tool::Lasso, 0xffff_ffff));
        assert!(!p.set_color(Tool::Eraser(EraserKind::Bitmap), 0xffff_ffff));
    }

    #[test]
    fn visibility_is_a_plain_flag() {
        let mut p = ToolPicker::new();
        p.set_visible(true);
        assert!(p.is_visible());
        p.set_visible(false);
        assert!(!p.is_visible());
    }
}
