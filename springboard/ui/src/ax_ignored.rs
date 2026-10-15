//! Ignored accessibility cells: list cells that carry only decoration or input chrome and are hidden from assistive
//! technology. An image-only cell and a text-field cell are ignored, so a screen reader walks past them, while a text or
//! button cell stays exposed. Mirrors the reference ignored-cell classes for image and text-field cells, expressed as a
//! filter over the element map's entries.

use crate::ax::Element;

/// The kind of a list cell, which decides whether assistive technology may see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    /// a cell whose only content is an image or an icon
    Image,
    /// a cell whose content is an editable text field
    TextField,
    /// a cell with readable text
    Text,
    /// a cell that performs an action when pressed
    Button,
}

impl CellKind {
    /// True when the cell is hidden from assistive technology. The reference ignores image cells and text-field cells.
    pub fn is_ignored(self) -> bool {
        matches!(self, CellKind::Image | CellKind::TextField)
    }
}

/// One cell of a list with its element entry, as a producer adds it to the element map.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub kind: CellKind,
    pub element: Element,
}

/// The elements a screen reader may walk, in list order. Ignored cells are dropped; the rest keep their order.
pub fn exposed(cells: &[Cell]) -> Vec<&Element> {
    cells.iter().filter(|c| !c.kind.is_ignored()).map(|c| &c.element).collect()
}

/// How many cells of `cells` are hidden from assistive technology.
pub fn ignored_count(cells: &[Cell]) -> usize {
    cells.iter().filter(|c| c.kind.is_ignored()).count()
}
